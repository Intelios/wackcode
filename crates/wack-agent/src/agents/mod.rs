//! Sub-agents: a chat hands a self-contained task to a child agent with its own context, and gets
//! its report back.
//!
//! - **Roles** (`definition`): Explore, Review and General are built in; the user's own files
//!   add roles or replace these.
//! - **The tool** (`tools`): `agent` starts a child and waits for its report. The roster is in its
//!   description, so it is part of the cached prompt and never changes during a run.
//! - **A child** (`child`) is a full [`crate::Agent`] with its own session file beside its
//!   parent's (`sessions/<chat>/<id>.jsonl`), its own shell tasks, the role's tools out of the
//!   parent's, and none of the parent-only ones (no spawning, so one level only; no questions to
//!   the user; no plan or todo tools; no memory). It runs on its own Tokio task, so a waiting
//!   call can be dropped without tearing the child down mid-write.
//! - **The registry** ([`Agents`]) is the chat's, beside its shell tasks, and outlives runs. It
//!   numbers children, keeps what the UI shows of each (status, steps, tokens, latest activity),
//!   and stops them.
//!
//! Invariants:
//! - A child's report reaches its parent exactly once: decided under the child's own lock when
//!   it ends, and taken by the call that waits for it.
//! - Stopping the parent's run stops its children: a waiting call that is dropped stops the child
//!   it waited for ([`Waiting`]).
//! - Ids are never reused within a chat: a new one is past every child the registry knows and
//!   every session file in the chat's folder.

pub mod child;
pub mod definition;
pub mod prompt;
pub mod tools;

#[cfg(test)]
mod flow_tests;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::agent::Control;

pub use child::{ChildModel, child_tools};
pub use definition::{AgentDef, Catalog, ModelPref};
pub use tools::{AGENT, AgentTool};

/// The longest report handed back to the parent; the rest is cut.
pub const MAX_REPORT_CHARS: usize = 50_000;

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
    Running,
    Done,
    Failed { message: String },
    Stopped { by: StoppedBy },
    /// It was running when the app quit.
    Interrupted,
}

impl Status {
    pub fn running(&self) -> bool {
        *self == Status::Running
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
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

/// One chat's sub-agents. Cloning shares them.
#[derive(Clone, Default)]
pub struct Agents {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    children: Mutex<Vec<Arc<Child>>>,
    observer: Mutex<Option<Observer>>,
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
    registry: Weak<Inner>,
}

#[derive(Default)]
struct Live {
    status: Option<Status>,
    ended_at: Option<u64>,
    tool_calls: u32,
    context_tokens: u64,
    activity: Option<Activity>,
    /// The parent's result, waiting for the call that waits on this child.
    report: Option<Result<String, String>>,
}

impl Child {
    pub fn info(&self) -> AgentInfo {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
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
            status: live.status.clone().unwrap_or(Status::Running),
            tool_calls: live.tool_calls,
            context_tokens: live.context_tokens,
            activity: live.activity.clone(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn running(&self) -> bool {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).status.is_none()
    }

    /// Asks the child to stop. Its report says so.
    pub fn stop(&self) {
        self.control.stop();
    }

    /// Waits until the child has ended and its report is decided.
    pub async fn ended(&self) {
        let mut ended = self.ended.subscribe();
        let _ = ended.wait_for(|done| *done).await;
    }

    /// The report for the call that waited, once. `None` before the child ends, or once taken.
    pub fn take_report(&self) -> Option<Result<String, String>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).report.take()
    }

    fn signal(&self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.signal(Signal::Changed);
        }
    }

    /// Records one step of the child's run.
    fn step(&self, update: impl FnOnce(&mut Live)) {
        update(&mut self.live.lock().unwrap_or_else(|e| e.into_inner()));
        self.signal();
    }

    /// Records the child's end and its report, then wakes whoever waits.
    fn finish(&self, status: Status, report: Result<String, String>) {
        {
            let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
            live.status = Some(status);
            live.ended_at = Some(now_millis());
            live.activity = None;
            live.report = Some(report);
        }
        self.ended.send_replace(true);
        self.signal();
    }
}

/// Stops the child a call waits for when the wait is dropped: the user stopped the parent's run.
pub struct Waiting(pub Arc<Child>);

impl Drop for Waiting {
    fn drop(&mut self) {
        if self.0.running() {
            self.0.stop();
        }
    }
}

impl Agents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tells `observer` whenever a child starts, takes a step or ends. One observer at a time.
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

    /// The children still working.
    pub fn running(&self) -> Vec<AgentInfo> {
        self.children().iter().filter(|child| child.running()).map(|child| child.info()).collect()
    }

    /// Stops every child still working.
    pub fn stop_all(&self) {
        for child in self.children() {
            if child.running() {
                child.stop();
            }
        }
    }

    fn children(&self) -> Vec<Arc<Child>> {
        self.inner.children.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Adds a child with the next free id, whose session goes in `folder`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add(
        &self,
        folder: &Path,
        role: &str,
        description: &str,
        call_id: &str,
        model: &str,
        edits: bool,
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
            live: Mutex::new(Live::default()),
            ended: watch::channel(false).0,
            registry: Arc::downgrade(&self.inner),
        });
        children.push(child.clone());
        drop(children);
        self.inner.signal(Signal::Changed);
        child
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

    #[test]
    fn ids_continue_past_every_child_known_or_saved() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("a4.jsonl"), "").unwrap();
        let agents = Agents::new();
        let first = agents.add(folder.path(), "explore", "Find it", "c1", "m", false, Control::new());
        let second = agents.add(folder.path(), "general", "Do it", "c2", "m", true, Control::new());
        assert_eq!((first.id.as_str(), second.id.as_str()), ("a5", "a6"));
        assert_eq!(second.session, folder.path().join("a6.jsonl"));
        assert_eq!(children_folder(Path::new("/data/sessions/chat-1.jsonl")), Path::new("/data/sessions/chat-1"));
    }

    #[tokio::test]
    async fn a_report_is_taken_once_and_stop_all_reaches_only_running_children() {
        let folder = tempfile::tempdir().unwrap();
        let agents = Agents::new();
        let seen = Arc::new(Mutex::new(0));
        let counter = seen.clone();
        agents.set_observer(move |_| *counter.lock().unwrap() += 1);
        let (done, working) = (Control::new(), Control::new());
        let finished = agents.add(folder.path(), "explore", "a", "c1", "m", false, done.clone());
        let busy = agents.add(folder.path(), "explore", "b", "c2", "m", false, working.clone());
        finished.finish(Status::Done, Ok("report".into()));
        finished.ended().await;
        assert_eq!(finished.take_report(), Some(Ok("report".into())));
        assert_eq!(finished.take_report(), None);

        agents.stop_all();
        assert!(!done.stopped() && working.stopped());
        assert_eq!(agents.running().len(), 1, "a stopped child is running until its run ends");
        drop(Waiting(busy.clone()));
        assert!(*seen.lock().unwrap() >= 3);
        assert_eq!(agents.list()[0].status, Status::Done);
    }
}
