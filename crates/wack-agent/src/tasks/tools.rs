//! The tools for tasks the agent runs in the background, and the feature that tells the model
//! about them.
//!
//! There is no status tool on purpose: the model cannot poll what it cannot ask about. A task's
//! end, and each batch of a watch's lines, reach the model as a note before its next request
//! ([`TasksFeature::notes`]), or wake an idle chat (the app's job). `wait_for_tasks` is the one way
//! to block, once per reply and for a bounded time.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Kind, NOTE_PREFIX, Status, StoppedBy, Task, TaskInfo, Tasks, Why, elapsed, first_line, quoted};
use crate::{
    agent::Feature,
    session::{Note, Session},
    tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec},
};

pub const STOP_TASK: &str = "stop_task";
pub const WAIT_FOR_TASKS: &str = "wait_for_tasks";
pub const WATCH: &str = "watch";

/// The longest `wait_for_tasks` blocks: under a five-minute prompt cache, so the request after
/// the wait still finds it warm.
const WAIT_LIMIT: Duration = Duration::from_secs(270);
/// A watch reports at most this often, and gathers lines for this long.
const WATCH_EVERY: Duration = Duration::from_secs(5);
const WATCH_TICK: Duration = Duration::from_millis(200);
/// A watch that reported this many times within `CHATTY_WINDOW` is stopped.
const CHATTY_NOTICES: usize = 24;
const CHATTY_WINDOW: Duration = Duration::from_secs(120);
const WATCH_LINES: usize = 40;
const DEFAULT_WATCH_SECONDS: u64 = 600;
const MAX_WATCH_SECONDS: u64 = 3_600;

pub const NO_SUCH_TASK: &str =
    "That task is no longer available in this chat: it may have ended long ago, or WackCode restarted. Do not assume it succeeded.";

/// What `bash` returns for a command that carries on in the background. Every sentence answers
/// a question the model would otherwise answer by polling.
pub fn moved_result(task: &Task, why: Why, timeout: Duration) -> String {
    let id = &task.id;
    let opening = match why {
        Why::Asked => format!("Started in the background (ID: {id})."),
        Why::TimedOut => format!(
            "Command did not complete within its {}s timeout and was moved to the background (ID: {id}).",
            timeout.as_secs()
        ),
        Why::User => format!("The user moved this command to the background (ID: {id})."),
    };
    let mut lines = vec![
        opening,
        format!("Output is being written to: {}", task.log().display()),
        "You will be notified when it finishes — do not poll that file, sleep, or run commands to check on it.".to_string(),
        format!("It is running now: do not start a second copy. If it is no longer needed, stop it with {STOP_TASK}."),
        "Read the file only if the user asks for progress, or if you need a line it prints before you can go on, such as a server's ready line or port.".to_string(),
    ];
    if why != Why::Asked && task.bytes() == 0 {
        lines.push(
            "No output had been read from it when it was moved. A program that buffers its output, or a pipe into one such as tail, can keep the output file empty until the command finishes."
                .to_string(),
        );
    }
    lines.join("\n")
}

fn parse<T: serde::de::DeserializeOwned>(input: Value) -> Result<T, String> {
    serde_json::from_value(input).map_err(|error| format!("The tool's input is wrong: {error}."))
}

// -------------------------------------------------------------------------------------------------
// stop_task

pub struct StopTask;

#[derive(serde::Deserialize)]
struct StopInput {
    id: String,
}

#[async_trait]
impl Tool for StopTask {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: STOP_TASK.into(),
            description: "Stops a command running in the background, or a watch, by its ID, with everything it started. Use it when the command is no longer needed, is stuck, or the user asks.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The task's ID, from the result that started it." } },
                "required": ["id"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "stop a background command or watch by its ID".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: StopInput = parse(input)?;
        let Some(task) = context.tasks.get(&input.id) else {
            return Err(NO_SUCH_TASK.to_string());
        };
        let command = first_line(&task.command).to_string();
        if !task.info().status.running() {
            return Ok(format!("Task {} (`{command}`) had already ended.", task.id).into());
        }
        context.tasks.stop(&task.id, StoppedBy::Agent).await;
        Ok(format!("Stopped task {} (`{command}`).", task.id).into())
    }
}

// -------------------------------------------------------------------------------------------------
// wait_for_tasks

pub struct WaitForTasks;

#[async_trait]
impl Tool for WaitForTasks {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WAIT_FOR_TASKS.into(),
            description: format!(
                "Pauses until one of your background commands, watches or agents reports something (a command finishes or prints watched lines, an agent finishes), then returns what arrived and what is still running. Call it only when you have no other work and cannot go on without the result; otherwise carry on, or end your turn: you are told when they report, even after your turn has ended. It waits at most {} seconds, and only once per reply.",
                WAIT_LIMIT.as_secs()
            ),
            input_schema: json!({ "type": "object", "properties": {} }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "pause until a background command or agent reports, when there is nothing else to do".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, _input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let (tasks, agents) = (&context.tasks, &context.agents);
        let agents_working = || agents.active().into_iter().filter(|info| info.background).collect::<Vec<_>>();
        if tasks.running().is_empty() && !tasks.has_news() && agents_working().is_empty() && !agents.has_news() {
            return Err("Nothing is running in the background, so there is nothing to wait for.".to_string());
        }
        let reply = tasks.inner.replies.load(Ordering::SeqCst);
        if tasks.inner.waited.swap(reply + 1, Ordering::SeqCst) == reply + 1 {
            return Err(
                "You already waited once in this reply. Carry on with other work, or end your turn: you will be told when a task reports."
                    .to_string(),
            );
        }
        let started = Instant::now();
        if !tasks.has_news() && !agents.has_news() {
            tokio::select! {
                () = tasks.news() => {}
                () = agents.news() => {}
                () = tokio::time::sleep(WAIT_LIMIT) => {}
                () = context.cancel.cancelled() => return Err("Stopped by the user while waiting. The tasks are still running.".to_string()),
            }
        }
        // An agent's report is handed over here rather than as a note: it leaves the inbox once.
        let mut news = tasks.take_news();
        news.extend(agents.take_news());
        let mut parts = Vec::new();
        if news.is_empty() {
            parts.push(format!("Nothing reported in {}.", elapsed(started.elapsed())));
        } else {
            parts.push("While you waited:".to_string());
            parts.extend(news.into_iter().map(|note| note.text));
        }
        let running = tasks.running();
        let working = agents_working();
        if running.is_empty() && working.is_empty() {
            parts.push("Nothing else is running.".to_string());
        } else {
            let mut list = vec!["Still running:".to_string()];
            for task in running {
                let info = task.info();
                let last = if info.last_line.trim().is_empty() { String::new() } else { format!(" — last line: {}", info.last_line.trim()) };
                list.push(format!("- {} (`{}`), for {}{last}", info.id, first_line(&info.command), elapsed(task.elapsed())));
            }
            for info in working {
                let since = agents.get(&info.id).map(|child| elapsed(child.elapsed())).unwrap_or_default();
                list.push(format!("- agent {} ({} · {}), for {since}", info.id, info.role, info.description));
            }
            parts.push(list.join("\n"));
        }
        Ok(parts.join("\n\n").into())
    }
}

// -------------------------------------------------------------------------------------------------
// watch

pub struct Watch;

#[derive(serde::Deserialize)]
struct WatchInput {
    command: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    persistent: bool,
    expires_after: Option<u64>,
}

#[async_trait]
impl Tool for Watch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: WATCH.into(),
            description: format!(
                "Runs a command in the background and tells you each line it prints to standard output, as notices that arrive on their own (gathered for {} seconds at a time), so you can react to events without polling: errors from a dev server, a test watcher's results, a file appearing. Filter it so only lines that matter are printed, e.g. `tail -f server.log | grep --line-buffered -i error` (without --line-buffered, lines arrive late). Standard error goes to its output file, not to you. It stops after expires_after seconds ({DEFAULT_WATCH_SECONDS} unless set, at most {MAX_WATCH_SECONDS}) unless persistent is true; stop it sooner with {STOP_TASK}. A watch that reports every {} seconds for {} minutes is stopped as too noisy.",
                WATCH_EVERY.as_secs(),
                WATCH_EVERY.as_secs(),
                CHATTY_WINDOW.as_secs() / 60
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command whose output lines you want to hear about." },
                    "description": { "type": "string", "description": "A few words saying what it watches for, shown to the user, e.g. \"Dev server errors\"." },
                    "persistent": { "type": "boolean", "description": "Keep watching until stopped, with no time limit." },
                    "expires_after": { "type": "integer", "description": format!("Seconds to watch for, up to {MAX_WATCH_SECONDS}.") }
                },
                "required": ["command", "description"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "run a command in the background and be told each line it prints (watch a server for errors)".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: WatchInput = parse(input)?;
        if input.command.trim().is_empty() {
            return Err("The command is empty.".to_string());
        }
        let limit = (!input.persistent).then(|| Duration::from_secs(input.expires_after.unwrap_or(DEFAULT_WATCH_SECONDS).clamp(1, MAX_WATCH_SECONDS)));
        let task = context.tasks.start(&context.cwd, &input.command, &input.description, Kind::Watch, Some(Why::Asked))?;
        tokio::spawn(report(context.tasks.clone(), task.clone(), limit));
        let ending = match limit {
            Some(limit) => format!("It stops after {} seconds; stop it sooner with {STOP_TASK} once you no longer need it.", limit.as_secs()),
            None => format!("It keeps watching until you stop it with {STOP_TASK}: do so once you no longer need it."),
        };
        Ok(format!(
            "Watching in the background (ID: {}). Each line it prints reaches you as a notice, a few seconds' worth at a time, and you are told when it ends; do not poll its output file ({}). {ending}",
            task.id,
            task.log().display()
        ).into())
    }
}

/// Sends a watch's lines as notices, at most one every `WATCH_EVERY`, until it ends; stops it at
/// its time limit, or when it never goes quiet.
async fn report(tasks: Tasks, task: Arc<Task>, limit: Option<Duration>) {
    let weak = Arc::downgrade(&tasks.inner);
    drop(tasks);
    let mut last: Option<Instant> = None;
    let mut sent: VecDeque<Instant> = VecDeque::new();
    loop {
        tokio::select! {
            () = task.ended() => return,
            () = tokio::time::sleep(WATCH_TICK) => {}
        }
        let Some(inner) = weak.upgrade() else { return };
        let tasks = Tasks { inner };
        if limit.is_some_and(|limit| task.elapsed() >= limit) {
            tasks.stop(&task.id, StoppedBy::Expired).await;
            return;
        }
        if last.is_some_and(|last| last.elapsed() < WATCH_EVERY) {
            continue;
        }
        let lines = task.take_lines(false);
        if lines.is_empty() {
            continue;
        }
        let now = Instant::now();
        last = Some(now);
        tasks.tell(Note {
            tag: format!("{NOTE_PREFIX}{}:lines", task.id),
            text: format!("Watch {} ({}) printed:\n```\n{}\n```", task.id, task.info().label(), quoted(&lines, WATCH_LINES)),
        });
        sent.push_back(now);
        while sent.front().is_some_and(|at| now.duration_since(*at) > CHATTY_WINDOW) {
            sent.pop_front();
        }
        if sent.len() >= CHATTY_NOTICES {
            tasks.stop(&task.id, StoppedBy::Chatty).await;
            return;
        }
    }
}

// -------------------------------------------------------------------------------------------------
// The feature

/// Hands the model the chat's task notices, saves the task list in the session, and tells the
/// model about tasks a previous run left that died with the app.
#[derive(Clone)]
pub struct TasksFeature {
    tasks: Tasks,
    orphans: Arc<Mutex<Vec<Note>>>,
}

pub const TASKS_STATE: &str = "tasks";

impl TasksFeature {
    pub fn new(session: &Session, tasks: Tasks) -> Self {
        let saved: Vec<TaskInfo> = session.state(TASKS_STATE).and_then(|value| serde_json::from_value(value.clone()).ok()).unwrap_or_default();
        let orphans = saved
            .into_iter()
            .filter(|info| info.status.running() && tasks.get(&info.id).is_none())
            .map(|info| Note {
                tag: format!("{NOTE_PREFIX}{}:lost", info.id),
                text: format!(
                    "Background task {} (`{}`) is no longer running: it was stopped when WackCode quit. Its output up to then is in {}.",
                    info.id,
                    first_line(&info.command),
                    info.log.display()
                ),
            })
            .collect();
        Self { tasks, orphans: Arc::new(Mutex::new(orphans)) }
    }
}

impl Feature for TasksFeature {
    fn notes(&self, _session: &Session) -> Vec<Note> {
        let mut notes: Vec<Note> = self.orphans.lock().unwrap().drain(..).collect();
        notes.extend(self.tasks.take_news());
        notes
    }

    fn pending(&self) -> bool {
        self.tasks.wakes()
    }

    fn after_turn(&self, _reply: &wack_llm::Message) {
        self.tasks.inner.replies.fetch_add(1, Ordering::SeqCst);
    }

    fn state_to_save(&self) -> Vec<(String, Value)> {
        let list = self.tasks.list();
        vec![(TASKS_STATE.to_string(), serde_json::to_value(list).unwrap_or(Value::Null))]
    }

    fn summary_sections(&self) -> Vec<String> {
        let running: Vec<String> = self
            .tasks
            .list()
            .into_iter()
            .filter(|info| info.status == Status::Running)
            .map(|info| format!("- {} (`{}`), output in {}", info.id, first_line(&info.command), info.log.display()))
            .collect();
        if running.is_empty() {
            return Vec::new();
        }
        vec![format!("## Running in the background\n\n{}", running.join("\n"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::gone;

    fn context(folder: &tempfile::TempDir, tasks: &Tasks) -> ToolContext<'static> {
        let mut context = ToolContext::detached(folder.path().to_path_buf());
        context.tasks = tasks.clone();
        context
    }

    #[tokio::test]
    async fn stop_task_stops_and_says_only_that() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let task = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        let result = StopTask.run(json!({ "id": task.id }), &context(&folder, &tasks)).await.unwrap().text;
        assert_eq!(result, format!("Stopped task {} (`sleep 60`).", task.id));
        assert!(gone(task.pgid).await);
        let unknown = StopTask.run(json!({ "id": "zzzz" }), &context(&folder, &tasks)).await.unwrap_err();
        assert_eq!(unknown, NO_SUCH_TASK);
    }

    #[tokio::test]
    async fn waiting_returns_the_news_and_only_once_per_reply() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let context = context(&folder, &tasks);
        assert!(WaitForTasks.run(json!({}), &context).await.unwrap_err().contains("Nothing is running"));

        tasks.start(folder.path(), "sleep 0.2; echo built", "", Kind::Shell, Some(Why::Asked)).unwrap();
        let slow = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        let result = WaitForTasks.run(json!({}), &context).await.unwrap().text;
        assert!(result.contains("While you waited:") && result.contains("built"), "{result}");
        assert!(result.contains(&format!("- {} (`sleep 60`)", slow.id)), "{result}");
        assert!(!tasks.has_news(), "what the wait returned is not delivered again");

        let again = WaitForTasks.run(json!({}), &context).await.unwrap_err();
        assert!(again.contains("already waited"), "{again}");
        tasks.inner.replies.fetch_add(1, Ordering::SeqCst);
        tasks.tell(Note { tag: "task:x:end".into(), text: "news".into() });
        assert!(WaitForTasks.run(json!({}), &context).await.unwrap().text.contains("news"));
        tasks.terminate_all();
    }

    #[tokio::test]
    async fn a_watch_sends_its_lines_in_batches_and_reports_its_end() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        let context = context(&folder, &tasks);
        let result = Watch
            .run(json!({ "command": "echo one; echo two; echo o\"\"ops >&2; sleep 0.5", "description": "Lines" }), &context)
            .await
            .unwrap()
            .text;
        assert!(result.starts_with("Watching in the background (ID: "), "{result}");
        let task = tasks.list().pop().unwrap();
        tasks.get(&task.id).unwrap().ended().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let news = tasks.take_news();
        let all: String = news.iter().map(|note| note.text.clone()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("one\ntwo"), "{all}");
        assert!(!all.contains("oops"), "standard error is not sent: {all}");
        assert!(news.last().unwrap().tag.ends_with(":end"));
        let lines_notices = news.iter().filter(|note| note.tag.ends_with(":lines")).count();
        assert!(lines_notices <= 1, "two lines printed together arrive together");
    }

    #[tokio::test]
    async fn a_watch_stops_at_its_limit() {
        let folder = tempfile::tempdir().unwrap();
        let tasks = Tasks::new(folder.path());
        Watch.run(json!({ "command": "sleep 60", "description": "x", "expires_after": 1 }), &context(&folder, &tasks)).await.unwrap();
        let task = tasks.get(&tasks.list()[0].id).unwrap();
        tokio::time::timeout(Duration::from_secs(10), task.ended()).await.unwrap();
        assert_eq!(task.info().status, Status::Stopped { by: StoppedBy::Expired });
        tasks.news().await;
        assert!(tasks.take_news().iter().any(|note| note.text.contains("time limit")));
    }

    #[test]
    fn a_moved_command_is_explained_once_and_fully() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let folder = tempfile::tempdir().unwrap();
            let tasks = Tasks::new(folder.path());
            let task = tasks.start(folder.path(), "sleep 60", "", Kind::Shell, None).unwrap();
            let text = moved_result(&task, Why::TimedOut, Duration::from_secs(120));
            assert!(text.starts_with(&format!("Command did not complete within its 120s timeout and was moved to the background (ID: {}).", task.id)));
            assert!(text.contains("do not poll") && text.contains("stop_task") && text.contains("No output had been read"));
            assert!(moved_result(&task, Why::Asked, Duration::ZERO).starts_with("Started in the background"));
            tasks.terminate_all();
        });
    }
}
