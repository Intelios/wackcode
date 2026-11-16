//! Shell tasks: every command the agent runs is a task this registry owns and watches.
//!
//! A chat owns one [`Tasks`] for as long as it is open, so a task outlives the run that started
//! it: a dev server keeps serving after the agent's turn ends. A command in the foreground is just
//! a task the `bash` call waits on; when its wait ends early (the model asked, its timeout passed,
//! the user pressed ⌘B) it carries on in the background and the model is told when it finishes,
//! through the chat's inbox ([`Tasks::take_news`]). There is deliberately no status tool: the
//! model is told, never asks.
//!
//! Invariants:
//! - Each task runs in its own process group, and the whole group is killed when the command's
//!   own process exits, so nothing a finished command started outlives it.
//! - A foreground task whose wait is dropped (the user stopped the run) is killed: see
//!   [`Holding`]. Tasks already in the background are left alone.
//! - Whether a task finished in the foreground or the background is decided under one lock, so
//!   its end reaches the model exactly once: as the `bash` result, or as a notice.
//! - A notice leaves the inbox only when something takes it to hand to the model.
//! - Provider keys (`SECRET_ENV`) are removed from every command's environment.
//! - Dropping the last handle kills every task still running; quitting calls
//!   [`Tasks::terminate_all`] first, which asks them to stop before killing them.

mod tools;

use std::{
    collections::VecDeque,
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{Notify, watch},
};

use crate::session::Note;

pub use tools::{NO_SUCH_TASK, STOP_TASK, StopTask, TASKS_STATE, TasksFeature, WAIT_FOR_TASKS, WATCH, WaitForTasks, Watch, moved_result};

/// Provider credentials a command has no business inheriting.
pub const SECRET_ENV: [&str; 18] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "ANTHROPIC_IDENTITY_TOKEN_FILE",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "DEEPSEEK_API_KEY",
    "XAI_API_KEY",
    "GROQ_API_KEY",
    "MISTRAL_API_KEY",
    "CEREBRAS_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AWS_BEARER_TOKEN_BEDROCK",
    "HF_TOKEN",
];

/// At most this many tasks run at once in one chat.
pub const MAX_RUNNING: usize = 8;
/// Finished tasks beyond this many are forgotten, oldest first.
pub const MAX_KEPT: usize = 32;
/// The end of a task's output kept in memory, for its result and the UI.
const RING_BYTES: usize = 256 * 1024;
/// A task's log file stops growing here.
const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
/// Lines of output quoted in a notice.
const NOTICE_LINES: usize = 20;
/// The tag every task note starts with.
pub const NOTE_PREFIX: &str = "task:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Shell,
    /// Each line it prints is a notice (the `watch` tool).
    Watch,
}

/// Why a command is running in the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    /// The model started it there.
    Asked,
    /// It outlasted its timeout.
    TimedOut,
    /// The user moved it (⌘B).
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoppedBy {
    Agent,
    User,
    /// A watch reached its time limit.
    Expired,
    /// A watch printed too often.
    Chatty,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Status {
    Running,
    Finished,
    Failed { code: Option<i32>, signal: Option<i32> },
    Stopped { by: StoppedBy },
}

impl Status {
    pub fn running(&self) -> bool {
        *self == Status::Running
    }
}

/// What the app and the session know of a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub kind: Kind,
    /// Milliseconds since 1970.
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Why>,
    pub log: PathBuf,
    /// How much it has printed. Not saved: it changes all the time.
    #[serde(skip)]
    pub bytes: u64,
    /// The last line it printed. Not saved.
    #[serde(skip)]
    pub last_line: String,
}

impl TaskInfo {
    /// What to call it: its description, or its command's first line.
    pub fn label(&self) -> &str {
        if self.description.trim().is_empty() { first_line(&self.command) } else { self.description.trim() }
    }
}

/// What a registry tells whoever watches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// A task started, ended, moved or printed.
    Changed,
    /// A notice is waiting for the model.
    News,
}

type Observer = Arc<dyn Fn(Signal) + Send + Sync>;

/// One chat's tasks. Cloning shares them.
#[derive(Clone)]
pub struct Tasks {
    inner: Arc<Inner>,
}

struct Inner {
    folder: PathBuf,
    tasks: Mutex<Vec<Arc<Task>>>,
    inbox: Mutex<VecDeque<Note>>,
    news: Notify,
    observer: Mutex<Option<Observer>>,
    /// A scratch registry deletes its folder when it goes.
    scratch: bool,
    /// Replies the model has finished, and one more than the reply it last waited in, so
    /// `wait_for_tasks` blocks at most once per reply.
    replies: AtomicU64,
    waited: AtomicU64,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().unwrap_or_else(|e| e.into_inner()).iter() {
            if task.info().status.running() {
                task.mark_stopped(StoppedBy::Quit);
                signal_group(task.pgid, libc::SIGKILL);
            }
        }
        if self.scratch {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }
}

pub struct Task {
    pub id: String,
    pub command: String,
    pub description: String,
    pub kind: Kind,
    started: Instant,
    started_at: u64,
    log: PathBuf,
    pgid: i32,
    live: Mutex<Live>,
    exited: watch::Sender<bool>,
    /// The user moved the foreground command to the background.
    detached: Notify,
    file: Mutex<Option<File>>,
}

struct Live {
    status: Status,
    ended_at: Option<u64>,
    background: Option<Why>,
    bytes: u64,
    ring: Ring,
    /// A watch's whole lines not yet sent, and the line being written.
    lines: Vec<String>,
    partial: String,
}

impl Task {
    pub fn info(&self) -> TaskInfo {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        TaskInfo {
            id: self.id.clone(),
            command: self.command.clone(),
            description: self.description.clone(),
            kind: self.kind,
            started_at: self.started_at,
            ended_at: live.ended_at,
            status: live.status.clone(),
            background: live.background,
            log: self.log.clone(),
            bytes: live.bytes,
            last_line: live.ring.last_line().to_string(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn log(&self) -> &Path {
        &self.log
    }

    /// The end of what it printed.
    pub fn output(&self) -> String {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).ring.text()
    }

    pub fn dropped_output(&self) -> bool {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).ring.dropped
    }

    pub fn bytes(&self) -> u64 {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }

    /// Waits until the command's process has exited and its end is recorded.
    pub async fn ended(&self) {
        let mut exited = self.exited.subscribe();
        let _ = exited.wait_for(|done| *done).await;
    }

    /// Resolves when the user moves this command to the background.
    pub async fn detached(&self) {
        self.detached.notified().await
    }

    /// Moves a running task to the background. False when it had already ended or moved: then
    /// whoever waits on it reports its end.
    pub fn move_to_background(&self, why: Why) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !live.status.running() || live.background.is_some() {
            return false;
        }
        live.background = Some(why);
        true
    }

    fn mark_stopped(&self, by: StoppedBy) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !live.status.running() {
            return false;
        }
        live.status = Status::Stopped { by };
        true
    }

    fn record(&self, bytes: &[u8], text: &str, watched: bool) {
        if let Ok(mut file) = self.file.lock()
            && let Some(handle) = file.as_mut()
        {
            let written = self.live.lock().map(|live| live.bytes).unwrap_or(0);
            if written + bytes.len() as u64 <= MAX_LOG_BYTES {
                let _ = handle.write_all(bytes);
            } else {
                let _ = handle.write_all(b"\n[WackCode stopped writing this log: it reached 64 MB.]\n");
                *file = None;
            }
        }
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.bytes += bytes.len() as u64;
        live.ring.push(text);
        if watched {
            let mut partial = std::mem::take(&mut live.partial);
            partial.push_str(text);
            let mut rest = partial.as_str();
            while let Some(end) = rest.find('\n') {
                let line = rest[..end].trim_end_matches('\r').to_string();
                live.lines.push(line);
                rest = &rest[end + 1..];
            }
            live.partial = rest.to_string();
        }
    }

    /// A watch's lines not yet sent, at most `max`, and how many more there were.
    fn take_lines(&self, flush: bool) -> Vec<String> {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if flush && !live.partial.is_empty() {
            let line = std::mem::take(&mut live.partial);
            live.lines.push(line);
        }
        std::mem::take(&mut live.lines)
    }
}

/// Kills a foreground task when its wait is dropped (the user stopped the run), unless it has
/// moved to the background.
pub struct Holding(pub Arc<Task>);

impl Drop for Holding {
    fn drop(&mut self) {
        let foreground = {
            let live = self.0.live.lock().unwrap_or_else(|e| e.into_inner());
            live.status.running() && live.background.is_none()
        };
        if foreground && self.0.mark_stopped(StoppedBy::User) {
            signal_group(self.0.pgid, libc::SIGKILL);
        }
    }
}

impl Tasks {
    /// A registry whose logs go in `folder` (made when the first task starts).
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self::make(folder.into(), false)
    }

    /// A registry for a run nobody keeps: its logs go in a temporary folder removed with it.
    pub fn scratch() -> Self {
        let folder = std::env::temp_dir().join(format!("wackcode-tasks-{}-{}", std::process::id(), new_id()));
        Self::make(folder, true)
    }

    fn make(folder: PathBuf, scratch: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                folder,
                tasks: Mutex::new(Vec::new()),
                inbox: Mutex::new(VecDeque::new()),
                news: Notify::new(),
                observer: Mutex::new(None),
                scratch,
                replies: AtomicU64::new(0),
                waited: AtomicU64::new(0),
            }),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.inner.folder
    }

    /// Calls `observer` whenever a task changes or a notice arrives. Replaces any earlier one.
    pub fn observe(&self, observer: impl Fn(Signal) + Send + Sync + 'static) {
        *self.inner.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(observer));
    }

    fn signal(&self, signal: Signal) {
        let observer = self.inner.observer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(observer) = observer {
            observer(signal);
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<Task>> {
        self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|task| task.id == id.trim()).cloned()
    }

    /// Every task this chat still knows, oldest first.
    pub fn list(&self) -> Vec<TaskInfo> {
        self.all().iter().map(|task| task.info()).collect()
    }

    fn all(&self) -> Vec<Arc<Task>> {
        self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn running(&self) -> Vec<Arc<Task>> {
        self.all().into_iter().filter(|task| task.info().status.running()).collect()
    }

    // ---------------------------------------------------------------------------------------------
    // The inbox

    /// Leaves a notice for the model.
    pub fn tell(&self, note: Note) {
        self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).push_back(note);
        self.inner.news.notify_waiters();
        self.inner.news.notify_one();
        self.signal(Signal::News);
    }

    pub fn has_news(&self) -> bool {
        !self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Takes every notice waiting, oldest first.
    pub fn take_news(&self) -> Vec<Note> {
        self.inner.inbox.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    /// Resolves once a notice is waiting.
    pub async fn news(&self) {
        loop {
            let notified = self.inner.news.notified();
            if self.has_news() {
                return;
            }
            notified.await;
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Starting and stopping

    /// Starts `command` with `/bin/bash -c` in `cwd`. `background` is set for a command that
    /// starts there; otherwise the caller waits on it while holding a [`Holding`].
    pub fn start(&self, cwd: &Path, command: &str, description: &str, kind: Kind, background: Option<Why>) -> Result<Arc<Task>, String> {
        if command.contains('\0') {
            return Err("The command cannot contain null characters.".to_string());
        }
        self.make_room()?;
        std::fs::create_dir_all(&self.inner.folder).map_err(|error| format!("Could not create the folder for task output: {error}."))?;
        let id = new_id();
        let log = self.inner.folder.join(format!("{id}.log"));
        let file = File::create(&log).map_err(|error| format!("Could not create the task's output file: {error}."))?;

        let mut process = Command::new("/bin/bash");
        process
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        for name in SECRET_ENV {
            process.env_remove(name);
        }
        let mut child = process.spawn().map_err(|error| format!("Could not start the command: {error}."))?;
        let pgid = child.id().map(|id| id as i32).unwrap_or(0);
        let (exited, _) = watch::channel(false);
        let task = Arc::new(Task {
            id,
            command: command.to_string(),
            description: description.trim().to_string(),
            kind,
            started: Instant::now(),
            started_at: now_millis(),
            log,
            pgid,
            live: Mutex::new(Live {
                status: Status::Running,
                ended_at: None,
                background,
                bytes: 0,
                ring: Ring::default(),
                lines: Vec::new(),
                partial: String::new(),
            }),
            exited,
            detached: Notify::new(),
            file: Mutex::new(Some(file)),
        });
        self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner()).push(task.clone());

        let mut readers = Vec::new();
        let streams: [(Option<Box<dyn AsyncRead + Send + Unpin>>, bool); 2] = [
            (child.stdout.take().map(|stream| Box::new(stream) as _), kind == Kind::Watch),
            (child.stderr.take().map(|stream| Box::new(stream) as _), false),
        ];
        for (stream, watched) in streams {
            let Some(mut stream) = stream else { continue };
            let task = task.clone();
            let tasks = Arc::downgrade(&self.inner);
            readers.push(tokio::spawn(async move {
                let mut buffer = [0u8; 8_192];
                let mut carry = Vec::new();
                while let Ok(read) = stream.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    let text = decode(&mut carry, &buffer[..read]);
                    task.record(&buffer[..read], &text, watched);
                    if let Some(inner) = tasks.upgrade() {
                        Tasks { inner }.signal(Signal::Changed);
                    }
                }
            }));
        }

        let waiter = task.clone();
        let tasks = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let status = child.wait().await;
            // Whatever the command left running holds the output pipes open; killing the group
            // is what lets the readers finish.
            signal_group(waiter.pgid, libc::SIGKILL);
            for reader in readers {
                let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
            }
            let notify = {
                let mut live = waiter.live.lock().unwrap_or_else(|e| e.into_inner());
                if live.status.running() {
                    live.status = match status {
                        Ok(status) if status.success() => Status::Finished,
                        Ok(status) => {
                            use std::os::unix::process::ExitStatusExt;
                            Status::Failed { code: status.code(), signal: status.signal() }
                        }
                        Err(_) => Status::Failed { code: None, signal: None },
                    };
                }
                live.ended_at = Some(now_millis());
                // Stopped by the agent: stop_task's result already said so. Quitting: nobody is
                // left to tell.
                live.background.is_some() && !matches!(live.status, Status::Stopped { by: StoppedBy::Agent | StoppedBy::Quit })
            };
            waiter.file.lock().unwrap_or_else(|e| e.into_inner()).take();
            // `send` would drop the value when nobody is subscribed yet.
            waiter.exited.send_replace(true);
            if let Some(inner) = tasks.upgrade() {
                let tasks = Tasks { inner };
                if notify {
                    let lines = if waiter.kind == Kind::Watch { waiter.take_lines(true) } else { Vec::new() };
                    tasks.tell(Note { tag: format!("{NOTE_PREFIX}{}:end", waiter.id), text: end_notice(&waiter, &lines) });
                }
                tasks.signal(Signal::Changed);
            }
        });
        self.signal(Signal::Changed);
        Ok(task)
    }

    /// Adds a task with no process behind it, as `info` describes, having printed `output`: for
    /// gallery scenes and tests.
    pub fn insert_fixture(&self, info: TaskInfo, output: &str) {
        let (exited, _) = watch::channel(!info.status.running());
        let age = Duration::from_millis(now_millis().saturating_sub(info.started_at));
        let mut ring = Ring::default();
        ring.push(output);
        let task = Arc::new(Task {
            id: info.id,
            command: info.command,
            description: info.description,
            kind: info.kind,
            started: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
            started_at: info.started_at,
            log: info.log,
            pgid: 0,
            live: Mutex::new(Live {
                status: info.status,
                ended_at: info.ended_at,
                background: info.background,
                bytes: output.len() as u64,
                ring,
                lines: Vec::new(),
                partial: String::new(),
            }),
            exited,
            detached: Notify::new(),
            file: Mutex::new(None),
        });
        self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner()).push(task);
    }

    /// Refuses a ninth running task, and forgets the oldest finished ones beyond the limit.
    fn make_room(&self) -> Result<(), String> {
        let mut tasks = self.inner.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let running = tasks.iter().filter(|task| task.info().status.running()).count();
        if running >= MAX_RUNNING {
            return Err(format!(
                "{MAX_RUNNING} commands are already running in this chat. Stop one you no longer need with {STOP_TASK} before starting another."
            ));
        }
        while tasks.len() >= MAX_KEPT {
            let Some(oldest) = tasks.iter().position(|task| !task.info().status.running()) else { break };
            tasks.remove(oldest);
        }
        Ok(())
    }

    /// Stops a task: SIGINT, then SIGTERM after two seconds, then SIGKILL after one more. Returns
    /// its record, or `None` when no task has that id.
    pub async fn stop(&self, id: &str, by: StoppedBy) -> Option<TaskInfo> {
        let task = self.get(id)?;
        if task.mark_stopped(by) {
            self.signal(Signal::Changed);
            for (signal, wait) in [(libc::SIGINT, 2), (libc::SIGTERM, 1), (libc::SIGKILL, 2)] {
                signal_group(task.pgid, signal);
                if tokio::time::timeout(Duration::from_secs(wait), task.ended()).await.is_ok() {
                    break;
                }
            }
        }
        Some(task.info())
    }

    /// Moves the command the agent is waiting on to the background. False when none is waiting.
    pub fn background_foreground(&self) -> bool {
        let mut moved = false;
        for task in self.running() {
            if task.kind == Kind::Shell && task.move_to_background(Why::User) {
                task.detached.notify_one();
                moved = true;
            }
        }
        if moved {
            self.signal(Signal::Changed);
        }
        moved
    }

    /// Whether the agent is waiting on a command in the foreground.
    pub fn has_foreground(&self) -> bool {
        self.running().iter().any(|task| task.kind == Kind::Shell && task.info().background.is_none())
    }

    /// Stops every running task, asking first (SIGTERM) and killing what is left after a second.
    /// Blocks: for quitting and deleting a chat.
    pub fn terminate_all(&self) {
        let running: Vec<_> = self.running().into_iter().filter(|task| task.mark_stopped(StoppedBy::Quit)).collect();
        if running.is_empty() {
            return;
        }
        for task in &running {
            signal_group(task.pgid, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while running.iter().any(|task| group_alive(task.pgid)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        for task in &running {
            signal_group(task.pgid, libc::SIGKILL);
        }
        self.signal(Signal::Changed);
    }
}

/// What the model is told when a background task ends.
fn end_notice(task: &Task, watched: &[String]) -> String {
    let info = task.info();
    let what = if task.kind == Kind::Watch { "Watch" } else { "Background task" };
    let command = first_line(&task.command);
    let after = elapsed(task.elapsed());
    let ending = match &info.status {
        Status::Finished => format!("finished: exit code 0 after {after}."),
        Status::Failed { code: Some(code), .. } => format!("failed: exit code {code} after {after}."),
        Status::Failed { signal: Some(signal), .. } => format!("was killed by signal {signal} after {after}."),
        Status::Failed { .. } => format!("ended after {after}; its exit status is unknown."),
        Status::Stopped { by: StoppedBy::User } => format!("was stopped by the user after {after}."),
        Status::Stopped { by: StoppedBy::Expired } => {
            format!("reached its time limit and was stopped after {after}. Start it again with persistent: true if it should keep watching.")
        }
        Status::Stopped { by: StoppedBy::Chatty } => format!(
            "was stopped after {after}: it printed so often that it reported every 5 seconds for 2 minutes. Start it again with a narrower filter, such as grep --line-buffered for only the lines you need."
        ),
        Status::Stopped { .. } | Status::Running => format!("was stopped after {after}."),
    };
    let mut text = format!("{what} {} (`{command}`) {ending}", task.id);
    let lines: Vec<String> = if task.kind == Kind::Watch {
        watched.to_vec()
    } else {
        let output = task.output();
        let all: Vec<&str> = output.lines().collect();
        all[all.len().saturating_sub(NOTICE_LINES)..].iter().map(|line| line.to_string()).collect()
    };
    if !lines.is_empty() {
        let heading = if task.kind == Kind::Watch { "Lines it printed since the last notice" } else { "Last lines" };
        text.push_str(&format!("\n{heading}:\n```\n{}\n```", quoted(&lines, NOTICE_LINES * 2)));
    }
    text.push_str(&format!("\nFull output: {}", task.log.display()));
    text
}

/// Lines for a notice, the overflow counted rather than shown.
fn quoted(lines: &[String], max: usize) -> String {
    let mut shown: Vec<String> = lines.iter().take(max).map(|line| crate_cut(line, 400).to_string()).collect();
    if lines.len() > max {
        shown.push(format!("… {} more", lines.len() - max));
    }
    shown.join("\n")
}

fn crate_cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn first_line(command: &str) -> &str {
    let line = command.trim().lines().next().unwrap_or_default();
    crate_cut(line, 120)
}

/// `42s`, `1m32s`, `1h04m`.
pub fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{:02}m", seconds / 3_600, seconds / 60 % 60)
    }
}

/// The end of a task's output, never cut inside a character.
#[derive(Default)]
struct Ring {
    text: String,
    dropped: bool,
}

impl Ring {
    fn push(&mut self, text: &str) {
        self.text.push_str(text);
        if self.text.len() > 2 * RING_BYTES {
            let mut from = self.text.len() - RING_BYTES;
            while !self.text.is_char_boundary(from) {
                from += 1;
            }
            self.text.drain(..from);
            self.dropped = true;
        }
    }

    fn text(&self) -> String {
        self.text.clone()
    }

    fn last_line(&self) -> &str {
        self.text.trim_end().rsplit('\n').next().unwrap_or_default().trim_end_matches('\r')
    }
}

/// Turns bytes into text, holding back the start of a character split across reads.
fn decode(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
    carry.extend_from_slice(bytes);
    let valid = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        // An incomplete character at the end waits for the next read; anything else is invalid
        // and shown as a replacement character.
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => carry.len(),
    };
    let text = String::from_utf8_lossy(&carry[..valid]).into_owned();
    carry.drain(..valid);
    text
}

fn signal_group(pgid: i32, signal: libc::c_int) {
    if pgid > 0 {
        // SAFETY: a plain system call. A negative id addresses the group; if every member has
        // already exited the call fails harmlessly.
        unsafe {
            libc::kill(-pgid, signal);
        }
    }
}

fn group_alive(pgid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the group exists.
    pgid > 0 && unsafe { libc::kill(-pgid, 0) } == 0
}

pub fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or(0)
}

/// Eight characters of base 36, random enough that two tasks in one chat never share one.
fn new_id() -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    // SAFETY: arc4random takes no arguments and cannot fail.
    let mut value = (u64::from(unsafe { libc::arc4random() }) << 32) | u64::from(unsafe { libc::arc4random() });
    let mut id = String::with_capacity(8);
    for _ in 0..8 {
        id.push(DIGITS[(value % 36) as usize] as char);
        value /= 36;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    pub(crate) async fn gone(pid: i32) -> bool {
        for _ in 0..150 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[test]
    fn decoding_keeps_a_character_split_across_reads() {
        let mut carry = Vec::new();
        let bytes = "é!".as_bytes();
        assert_eq!(decode(&mut carry, &bytes[..1]), "");
        assert_eq!(decode(&mut carry, &bytes[1..]), "é!");
        assert!(carry.is_empty());
        assert_eq!(decode(&mut carry, b"\xff ok"), "\u{fffd} ok");
    }

    #[test]
    fn the_ring_keeps_the_end_on_a_character_boundary() {
        let mut ring = Ring::default();
        for _ in 0..RING_BYTES {
            ring.push("éa");
        }
        assert!(ring.dropped);
        assert!(ring.text.len() <= 2 * RING_BYTES);
        assert!(ring.text.ends_with("éa"));
    }

    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(elapsed(Duration::from_secs(42)), "42s");
        assert_eq!(elapsed(Duration::from_secs(92)), "1m32s");
        assert_eq!(elapsed(Duration::from_secs(3_840)), "1h04m");
    }

    #[tokio::test]
    async fn a_background_task_reports_its_end_once() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path().join("tasks"));
        let task = tasks.start(folder.path(), "echo hello; exit 3", "", Kind::Shell, Some(Why::Asked)).unwrap();
        task.ended().await;
        tasks.news().await;
        let news = tasks.take_news();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].tag, format!("task:{}:end", task.id));
        assert!(news[0].text.contains("failed: exit code 3"), "{}", news[0].text);
        assert!(news[0].text.contains("hello"));
        assert!(tasks.take_news().is_empty());
        assert_eq!(std::fs::read_to_string(task.log()).unwrap(), "hello\n");
    }

    #[tokio::test]
    async fn a_foreground_task_that_ends_sends_no_notice() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let task = tasks.start(folder.path(), "true", "", Kind::Shell, None).unwrap();
        task.ended().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!tasks.has_news());
        assert_eq!(task.info().status, Status::Finished);
    }

    #[tokio::test]
    async fn stopping_kills_the_whole_group_and_tells_nobody() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let marker = folder.path().join("pid");
        let task = tasks
            .start(folder.path(), &format!("sleep 60 & echo $! > {}; wait", marker.display()), "", Kind::Shell, Some(Why::Asked))
            .unwrap();
        while !marker.exists() || std::fs::read_to_string(&marker).unwrap().trim().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pid: i32 = std::fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
        let info = tasks.stop(&task.id, StoppedBy::Agent).await.unwrap();
        assert_eq!(info.status, Status::Stopped { by: StoppedBy::Agent });
        assert!(gone(pid).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!tasks.has_news(), "the agent stopped it, so it already knows");
        assert!(tasks.stop("nope", StoppedBy::Agent).await.is_none());
    }

    #[tokio::test]
    async fn a_task_the_user_stops_is_reported() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let task = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::TimedOut)).unwrap();
        tasks.stop(&task.id, StoppedBy::User).await.unwrap();
        tasks.news().await;
        assert!(tasks.take_news()[0].text.contains("was stopped by the user"));
    }

    #[tokio::test]
    async fn dropping_the_hold_kills_a_foreground_task_but_not_a_background_one() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let foreground = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, None).unwrap();
        let background = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        drop(Holding(foreground.clone()));
        drop(Holding(background.clone()));
        assert!(gone(foreground.pgid).await);
        assert!(alive(background.pgid));
        assert_eq!(foreground.info().status, Status::Stopped { by: StoppedBy::User });
        tasks.terminate_all();
        assert!(gone(background.pgid).await);
    }

    #[tokio::test]
    async fn the_running_limit_holds_and_finished_tasks_are_forgotten_first() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        for _ in 0..MAX_RUNNING {
            tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        }
        let refused = tasks.start(folder.path(), "true", "", Kind::Shell, None).err().unwrap();
        assert!(refused.contains("stop_task"), "{refused}");
        tasks.terminate_all();
        for task in tasks.all() {
            tokio::time::timeout(Duration::from_secs(5), task.ended()).await.expect("a stopped task ends");
        }
        for _ in 0..MAX_KEPT + 4 {
            let task = tasks.start(folder.path(), "true", "", Kind::Shell, None).unwrap();
            tokio::time::timeout(Duration::from_secs(5), task.ended()).await.expect("a quick task ends");
        }
        assert!(tasks.list().len() <= MAX_KEPT);
    }

    #[tokio::test]
    async fn dropping_the_registry_kills_what_is_still_running() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let task = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        let pgid = task.pgid;
        drop(tasks);
        assert!(gone(pgid).await);
    }

    #[tokio::test]
    async fn moving_to_the_background_happens_once() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let task = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, None).unwrap();
        assert!(tasks.has_foreground());
        assert!(tasks.background_foreground());
        assert!(!tasks.has_foreground());
        assert!(!task.move_to_background(Why::TimedOut));
        assert_eq!(task.info().background, Some(Why::User));
        tasks.terminate_all();
    }
}
