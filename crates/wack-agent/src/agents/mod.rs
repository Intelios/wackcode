//! Sub-agents: a chat hands a self-contained task to a child agent with its own context, and gets
//! its report back.
//!
//! - **Roles** (`definition`): Explore, Review and General are built in; the user's own files
//!   add roles or replace these.
//! - **The tool** (`tools`): `agent` starts a child and waits for its report, or lets it work in
//!   the background and report later. The roster is in its description, so it is part of the
//!   cached prompt and never changes during a run. Several calls in one reply run side by side.
//! - **A child** (`child`) is a full [`crate::Agent`] with its own session file beside its
//!   parent's (`sessions/<chat>/<id>.jsonl`), its own shell tasks, the role's tools out of the
//!   parent's, and none of the parent-only ones (no spawning, so one level only; no questions to
//!   the user; no plan or todo tools; no memory). It runs on its own Tokio task, so a waiting
//!   call can be dropped without tearing the child down mid-write.
//! - **The registry** ([`Agents`]) is the chat's, beside its shell tasks, and outlives runs. It
//!   numbers children, keeps what the UI shows of each (status, steps, tokens, latest activity),
//!   queues them past [`MAX_RUNNING`], holds the reports of those in the background until the
//!   model is told ([`AgentsFeature`]), and stops them.
//!
//! Invariants:
//! - A child's report reaches its parent exactly once. Whether it is the waiting call's result or
//!   a note for the model is decided under the child's own lock: moving to the background and
//!   ending both take it, so a child that ends as it moves goes one way only. A note leaves the
//!   inbox only when something takes it to hand to the model.
//! - A waiting call that is dropped (the user stopped the parent's run) stops the child it waited
//!   for, unless that child has moved to the background ([`Waiting`]). The app's Stop also stops
//!   every child in the background: an agent left running only spends tokens.
//! - At most [`MAX_PER_MESSAGE`] children start between two messages from the user, which bounds
//!   how often background reports can wake an idle chat.
//! - A report the user already knows about is quiet: it rides along with the parent's next
//!   request but never wakes it (the user stopped the child, or started its round themselves).
//! - Ids are never reused within a chat: a new one is past every child the registry knows and
//!   every session file in the chat's folder.

pub mod child;
pub mod definition;
mod feature;
pub mod prompt;
pub mod tools;

#[cfg(test)]
mod flow_tests;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, Semaphore, watch};

use crate::{agent::Control, session::Note};

pub use child::{ChildModel, child_tools};
pub use definition::{AgentDef, Catalog, ModelPref};
pub use feature::{AGENTS_STATE, AgentsFeature};
pub use tools::{AGENT, AgentTool};

/// The longest report handed back to the parent; the rest is cut.
pub const MAX_REPORT_CHARS: usize = 50_000;
/// At most this many children of one chat work at once; the rest wait their turn.
pub const MAX_RUNNING: usize = 6;
/// At most this many children start between two messages from the user.
pub const MAX_PER_MESSAGE: u32 = 16;
/// The tag every background report's note starts with.
pub const NOTE_PREFIX: &str = "agent:";
/// Finished children beyond this many are forgotten, oldest first.
const MAX_KEPT: usize = 64;

/// How freely the parent may start sub-agents (Settings › Sub-agents).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delegation {
    /// The model decides when delegating pays.
    #[default]
    Auto,
    /// Only when the user asks for it.
    OnRequest,
    /// No sub-agent tools at all.
    Off,
}

/// Everything a run needs to start sub-agents, resolved by the engine before it starts.
#[derive(Clone)]
pub struct AgentsSetup {
    /// The roles switched on, in the order the model is shown them.
    pub catalog: Vec<AgentDef>,
    pub delegation: Delegation,
    /// The chat's own model, for roles that use it.
    pub chat: ChildModel,
    /// Settings › Sub-agents' helper model, for roles that ask for it.
    pub helper: Option<ChildModel>,
    /// The chat's registry, which outlives the run.
    pub agents: Agents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoppedBy {
    User,
    /// It used all its steps and kept going.
    Limit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Status {
    /// Waiting for one of the [`MAX_RUNNING`] places.
    Queued,
    Running,
    Done,
    Failed { message: String },
    Stopped { by: StoppedBy },
    /// It was working when the app quit.
    Interrupted,
}

impl Status {
    /// Queued or running: it has not ended.
    pub fn active(&self) -> bool {
        matches!(self, Status::Queued | Status::Running)
    }
}

/// What a child is doing now: its latest tool and what it is pointed at.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub tool: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
}

/// What the app and the session know of a child.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub role: String,
    pub description: String,
    /// The parent's call that started it.
    pub call_id: String,
    pub session: PathBuf,
    pub model: String,
    /// Whether its role can change files.
    pub edits: bool,
    /// Milliseconds since 1970.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    pub status: Status,
    /// It reports as a note rather than as its call's result.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub background: bool,
    pub tool_calls: u32,
    /// How full its context was at its latest reply.
    pub context_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<Activity>,
}

/// What a registry tells whoever watches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// A child started, took a step or ended.
    Changed,
    /// A report is waiting for the model.
    News,
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

/// One chat's sub-agents. Cloning shares them.
#[derive(Clone)]
pub struct Agents {
    inner: Arc<Inner>,
}

impl Default for Agents {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                children: Mutex::default(),
                inbox: Mutex::default(),
                news: Notify::new(),
                observer: Mutex::default(),
                slots: Arc::new(Semaphore::new(MAX_RUNNING)),
                started: AtomicU32::new(0),
            }),
        }
    }
}

struct Inner {
    children: Mutex<Vec<Arc<Child>>>,
    /// Each report for the model, and whether it should wake the chat.
    inbox: Mutex<VecDeque<(Note, bool)>>,
    news: Notify,
    observer: Mutex<Option<Observer>>,
    slots: Arc<Semaphore>,
    /// Children started since the user last wrote.
    started: AtomicU32,
}

impl Inner {
    fn signal(&self, signal: Signal) {
        let observer = self.observer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(observer) = observer {
            observer(signal);
        }
    }
}

pub struct Child {
    pub id: String,
    pub role: String,
    pub description: String,
    pub call_id: String,
    pub session: PathBuf,
    pub model: String,
    pub edits: bool,
    started: Instant,
    started_at: u64,
    control: Control,
    live: Mutex<Live>,
    ended: watch::Sender<bool>,
    /// The child moved to the background while a call waited for it.
    detached: Notify,
    registry: Weak<Inner>,
}

struct Live {
    status: Status,
    ended_at: Option<u64>,
    background: bool,
    /// Its report rides along quietly: the user started this round.
    quiet: bool,
    tool_calls: u32,
    context_tokens: u64,
    activity: Option<Activity>,
    /// The parent's result, waiting for the call that waits on this child.
    report: Option<Result<String, String>>,
}

impl Child {
    pub fn info(&self) -> AgentInfo {
        let live = self.live();
        AgentInfo {
            id: self.id.clone(),
            role: self.role.clone(),
            description: self.description.clone(),
            call_id: self.call_id.clone(),
            session: self.session.clone(),
            model: self.model.clone(),
            edits: self.edits,
            started_at: self.started_at,
            ended_at: live.ended_at,
            status: live.status.clone(),
            background: live.background,
            tool_calls: live.tool_calls,
            context_tokens: live.context_tokens,
            activity: live.activity.clone(),
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Queued or running.
    pub fn active(&self) -> bool {
        self.live().status.active()
    }

    pub fn in_background(&self) -> bool {
        self.live().background
    }

    /// The user stops the child. Its report says so, and is quiet: the user knows.
    pub fn stop(&self) {
        self.live().quiet = true;
        self.control.stop();
    }

    /// Waits until the child has ended and its report is decided.
    pub async fn ended(&self) {
        let mut ended = self.ended.subscribe();
        let _ = ended.wait_for(|done| *done).await;
    }

    /// Resolves when the child moves to the background.
    pub async fn detached(&self) {
        self.detached.notified().await
    }

    /// Moves a working child to the background: it will report as a note. False when it has
    /// already ended or moved; then whoever waits on it gets its report.
    pub fn move_to_background(&self) -> bool {
        {
            let mut live = self.live();
            if !live.status.active() || live.background {
                return false;
            }
            live.background = true;
        }
        self.detached.notify_waiters();
        self.signal(Signal::Changed);
        true
    }

    /// The report for the call that waited, once. `None` before the child ends, once taken, or
    /// when it went to the background.
    pub fn take_report(&self) -> Option<Result<String, String>> {
        self.live().report.take()
    }

    fn signal(&self, signal: Signal) {
        if let Some(registry) = self.registry.upgrade() {
            registry.signal(signal);
        }
    }

    /// Records one step of the child's run.
    fn step(&self, update: impl FnOnce(&mut Live)) {
        update(&mut self.live());
        self.signal(Signal::Changed);
    }

    /// Records the child's end and hands its report to the waiting call, or to the inbox when
    /// it is in the background.
    fn finish(&self, status: Status, report: Result<String, String>) {
        let mut news = false;
        {
            let mut live = self.live();
            live.status = status;
            live.ended_at = Some(now_millis());
            live.activity = None;
            if live.background {
                if let Some(registry) = self.registry.upgrade() {
                    let note = Note { tag: format!("{NOTE_PREFIX}{}", self.id), text: self.background_report(&report) };
                    registry.inbox.lock().unwrap_or_else(|e| e.into_inner()).push_back((note, !live.quiet));
                    news = true;
                }
            } else {
                live.report = Some(report);
            }
        }
        self.ended.send_replace(true);
        if news && let Some(registry) = self.registry.upgrade() {
            registry.news.notify_waiters();
            registry.signal(Signal::News);
        }
        self.signal(Signal::Changed);
    }

    fn background_report(&self, report: &Result<String, String>) -> String {
        match report {
            Ok(text) => format!("Agent {} ({} · {}) finished in the background. Its report:\n\n{text}", self.id, self.role, self.description),
            Err(text) => format!("Agent {} ({} · {}) ended in the background without a report. {text}", self.id, self.role, self.description),
        }
    }
}

/// Stops the child a call waits for when the wait is dropped (the user stopped the parent's
/// run), unless it moved to the background.
pub struct Waiting(pub Arc<Child>);

impl Drop for Waiting {
    fn drop(&mut self) {
        let foreground = {
            let live = self.0.live();
            live.status.active() && !live.background
        };
        if foreground {
            self.0.stop();
        }
    }
}

impl Agents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tells `observer` whenever a child starts, takes a step or ends, and when a report waits
    /// for the model. One observer at a time.
    pub fn set_observer(&self, observer: impl Fn(Signal) + Send + Sync + 'static) {
        *self.inner.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(observer));
    }

    /// Every child, oldest first.
    pub fn list(&self) -> Vec<AgentInfo> {
        self.children().iter().map(|child| child.info()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Arc<Child>> {
        self.children().into_iter().find(|child| child.id == id)
    }

    /// The children queued or working.
    pub fn active(&self) -> Vec<AgentInfo> {
        self.children().iter().filter(|child| child.active()).map(|child| child.info()).collect()
    }

    /// Stops every child queued or working, in the foreground or not (the user's Stop).
    pub fn stop_all(&self) {
        for child in self.children() {
            if child.active() {
                child.stop();
            }
        }
    }

    /// Moves every child a call is waiting on to the background (⌘B). True when any moved.
    pub fn background_foreground(&self) -> bool {
        // Every one moves, so no short-circuit.
        let moved: Vec<bool> = self.children().iter().map(|child| child.move_to_background()).collect();
        moved.contains(&true)
    }

    /// The user wrote: children may start again.
    pub fn reset_budget(&self) {
        self.inner.started.store(0, Ordering::SeqCst);
    }

    /// Counts one more child against [`MAX_PER_MESSAGE`]. False when none may start.
    pub(crate) fn spend(&self) -> bool {
        self.inner.started.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |started| (started < MAX_PER_MESSAGE).then_some(started + 1)).is_ok()
    }

    /// The reports waiting for the model, in the order they came. Empties the inbox.
    pub fn take_news(&self) -> Vec<Note> {
        self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).drain(..).map(|(note, _)| note).collect()
    }

    pub fn has_news(&self) -> bool {
        !self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// True when a waiting report should wake an idle chat.
    pub fn wakes(&self) -> bool {
        self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(_, wakes)| *wakes)
    }

    /// Resolves the next time a report reaches the inbox.
    pub async fn news(&self) {
        self.inner.news.notified().await
    }

    fn children(&self) -> Vec<Arc<Child>> {
        self.inner.children.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Adds children a session saved that this registry doesn't know (the app was relaunched).
    /// One that was working is now Interrupted; those are returned.
    pub fn restore(&self, saved: Vec<AgentInfo>) -> Vec<AgentInfo> {
        let mut children = self.inner.children.lock().unwrap_or_else(|e| e.into_inner());
        let mut interrupted = Vec::new();
        for mut info in saved {
            if children.iter().any(|child| child.id == info.id) {
                continue;
            }
            if info.status.active() {
                info.status = Status::Interrupted;
                info.activity = None;
                interrupted.push(info.clone());
            }
            children.push(Arc::new(Child::restored(info, Arc::downgrade(&self.inner))));
        }
        children.sort_by_key(|child| number(&child.id).unwrap_or(0));
        drop(children);
        if !interrupted.is_empty() {
            self.inner.signal(Signal::Changed);
        }
        interrupted
    }

    /// Adds a queued child with the next free id, whose session goes in `folder`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add(
        &self,
        folder: &Path,
        role: &str,
        description: &str,
        call_id: &str,
        model: &str,
        edits: bool,
        background: bool,
        control: Control,
    ) -> Arc<Child> {
        let mut children = self.inner.children.lock().unwrap_or_else(|e| e.into_inner());
        let known = children.iter().filter_map(|child| number(&child.id));
        let on_disk = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.path().file_stem().and_then(|stem| number(&stem.to_string_lossy())));
        let next = known.chain(on_disk).max().unwrap_or(0) + 1;
        let id = format!("a{next}");
        let child = Arc::new(Child {
            session: folder.join(format!("{id}.jsonl")),
            id,
            role: role.to_string(),
            description: description.to_string(),
            call_id: call_id.to_string(),
            model: model.to_string(),
            edits,
            started: Instant::now(),
            started_at: now_millis(),
            control,
            live: Mutex::new(Live {
                status: Status::Queued,
                ended_at: None,
                background,
                quiet: false,
                tool_calls: 0,
                context_tokens: 0,
                activity: None,
                report: None,
            }),
            ended: watch::channel(false).0,
            detached: Notify::new(),
            registry: Arc::downgrade(&self.inner),
        });
        children.push(child.clone());
        // Forget the oldest finished children past the cap.
        while children.len() > MAX_KEPT {
            match children.iter().position(|child| !child.active()) {
                Some(index) => drop(children.remove(index)),
                None => break,
            }
        }
        drop(children);
        self.inner.signal(Signal::Changed);
        child
    }

    /// A place to run in, waiting while [`MAX_RUNNING`] others work. `None` if `control` stops
    /// first.
    pub(crate) async fn slot(&self, control: &Control) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let stop = control.stop_token();
        tokio::select! {
            biased;
            () = stop.cancelled() => None,
            permit = self.inner.slots.clone().acquire_owned() => permit.ok(),
        }
    }
}

impl Child {
    /// A child from a session's saved list, with no run behind it.
    fn restored(info: AgentInfo, registry: Weak<Inner>) -> Self {
        Self {
            id: info.id,
            role: info.role,
            description: info.description,
            call_id: info.call_id,
            session: info.session,
            model: info.model,
            edits: info.edits,
            started: Instant::now(),
            started_at: info.started_at,
            control: Control::new(),
            live: Mutex::new(Live {
                status: info.status,
                ended_at: info.ended_at,
                background: info.background,
                quiet: false,
                tool_calls: info.tool_calls,
                context_tokens: info.context_tokens,
                activity: None,
                report: None,
            }),
            ended: watch::channel(true).0,
            detached: Notify::new(),
            registry,
        }
    }
}

/// The number in a child's id (`a12` is 12).
fn number(id: &str) -> Option<u64> {
    id.strip_prefix('a')?.parse().ok()
}

/// The folder a chat's children keep their sessions in: beside the chat's own file, named after it.
pub fn children_folder(parent_session: &Path) -> PathBuf {
    parent_session.with_extension("")
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(agents: &Agents, folder: &Path, background: bool, control: Control) -> Arc<Child> {
        agents.add(folder, "explore", "Find it", "c1", "m", false, background, control)
    }

    #[test]
    fn ids_continue_past_every_child_known_or_saved() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("a4.jsonl"), "").unwrap();
        let agents = Agents::new();
        let first = add(&agents, folder.path(), false, Control::new());
        let second = add(&agents, folder.path(), false, Control::new());
        assert_eq!((first.id.as_str(), second.id.as_str()), ("a5", "a6"));
        assert_eq!(second.session, folder.path().join("a6.jsonl"));
        assert_eq!(children_folder(Path::new("/data/sessions/chat-1.jsonl")), Path::new("/data/sessions/chat-1"));
    }

    #[tokio::test]
    async fn a_report_goes_to_the_waiting_call_or_the_inbox_never_both() {
        let folder = tempfile::tempdir().unwrap();
        let agents = Agents::new();
        let signals = Arc::new(Mutex::new(Vec::new()));
        let seen = signals.clone();
        agents.set_observer(move |signal| seen.lock().unwrap().push(signal));

        let waited = add(&agents, folder.path(), false, Control::new());
        waited.finish(Status::Done, Ok("report".into()));
        waited.ended().await;
        assert_eq!(waited.take_report(), Some(Ok("report".into())));
        assert_eq!(waited.take_report(), None);
        assert!(!agents.has_news());
        assert!(!waited.move_to_background(), "an ended child can't move");

        let moved = add(&agents, folder.path(), false, Control::new());
        assert!(moved.move_to_background());
        assert!(!moved.move_to_background());
        moved.finish(Status::Done, Ok("later".into()));
        assert_eq!(moved.take_report(), None);
        assert!(agents.wakes());
        let news = agents.take_news();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].tag, "agent:a2");
        assert!(news[0].text.starts_with("Agent a2 (explore · Find it) finished in the background. Its report:\n\nlater"));
        assert!(!agents.has_news());
        assert!(signals.lock().unwrap().contains(&Signal::News));
    }

    #[tokio::test]
    async fn ending_while_moving_reports_exactly_once() {
        for _ in 0..200 {
            let folder = tempfile::tempdir().unwrap();
            let agents = Agents::new();
            let child = add(&agents, folder.path(), false, Control::new());
            let racer = child.clone();
            let mover = tokio::spawn(async move { racer.move_to_background() });
            child.finish(Status::Done, Ok("r".into()));
            let moved = mover.await.unwrap();
            let as_result = child.take_report().is_some();
            let as_note = !agents.take_news().is_empty();
            assert!(as_result != as_note, "exactly one way: result {as_result}, note {as_note}, moved {moved}");
            assert_eq!(as_note, child.in_background());
        }
    }

    #[test]
    fn stop_all_reaches_every_active_child_and_waiting_stops_only_the_foreground() {
        let folder = tempfile::tempdir().unwrap();
        let agents = Agents::new();
        let (done, foreground, background) = (Control::new(), Control::new(), Control::new());
        add(&agents, folder.path(), false, done.clone()).finish(Status::Done, Ok(String::new()));
        let waited = add(&agents, folder.path(), false, foreground.clone());
        let left = add(&agents, folder.path(), true, background.clone());
        drop(Waiting(left));
        assert!(!background.stopped(), "a child in the background outlives the call");
        drop(Waiting(waited));
        assert!(foreground.stopped());
        agents.stop_all();
        assert!(background.stopped() && !done.stopped());
    }

    #[test]
    fn the_budget_resets_when_the_user_writes() {
        let agents = Agents::new();
        assert!((0..MAX_PER_MESSAGE).all(|_| agents.spend()));
        assert!(!agents.spend());
        agents.reset_budget();
        assert!(agents.spend());
    }

    #[test]
    fn a_relaunch_restores_the_list_and_interrupts_what_was_working() {
        let folder = tempfile::tempdir().unwrap();
        let before = Agents::new();
        add(&before, folder.path(), false, Control::new()).finish(Status::Done, Ok(String::new()));
        add(&before, folder.path(), true, Control::new());
        let saved = before.list();

        let after = Agents::new();
        let interrupted = after.restore(saved.clone());
        assert_eq!(interrupted.iter().map(|info| info.id.as_str()).collect::<Vec<_>>(), ["a2"]);
        let statuses: Vec<Status> = after.list().into_iter().map(|info| info.status).collect();
        assert_eq!(statuses, [Status::Done, Status::Interrupted]);
        assert!(after.restore(saved).is_empty(), "children it knows are left alone");
        // A new child numbers on from them.
        assert_eq!(add(&after, folder.path(), false, Control::new()).id, "a3");
    }
}
