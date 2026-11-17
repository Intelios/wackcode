//! The shell tool.
//!
//! Every command is a task of the chat's registry (`wack_agent::tasks`): a command in the
//! foreground is a task this call waits on. When the wait ends early it carries on in the
//! background and the model is told, once, everything it needs ([`moved_result`]): the model asked
//! (`run_in_background`), its timeout passed, or the user pressed ⌘B. Stopping the run kills a
//! command still in the foreground ([`Holding`]); background ones carry on.
//!
//! Each command runs in its own process group, killed when the command ends, so nothing a
//! finished command started outlives it. API keys other tools read from the environment are
//! removed from the command's environment. WackCode's own keys never enter the environment at all
//! (they live in `secrets.json`), but the user may have launched the app from a shell that had
//! some.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wack_agent::{
    Loading, Tool, ToolContext, ToolOutput, ToolSpec,
    tasks::{Holding, Kind, STOP_TASK, Why, moved_result},
};

use crate::text::{parse, tail};

pub use wack_agent::SECRET_ENV;

const DEFAULT_TIMEOUT: u64 = 120;
const MAX_TIMEOUT: u64 = 600;

pub struct Bash;

#[derive(Deserialize)]
struct Input {
    command: String,
    #[serde(default)]
    description: String,
    timeout: Option<u64>,
    #[serde(default)]
    run_in_background: bool,
}

#[async_trait]
impl Tool for Bash {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".into(),
            description: format!("Runs a command with /bin/bash in the working directory and returns its output (stdout and stderr together) and exit code. Long output is cut to its end. A command still running after its timeout ({DEFAULT_TIMEOUT} seconds unless you set one) is moved to the background, not stopped, and you are told when it finishes. Set run_in_background to start it there at once: a dev server, a watcher, or a long build while you do other work. Anything a command started is stopped when the command ends, so run a server itself in the background rather than with `&`. Nothing can answer a prompt: use non-interactive flags."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to run." },
                    "description": { "type": "string", "description": "What it does in a few plain words, shown to the user, e.g. \"Run the unit tests\"." },
                    "timeout": { "type": "integer", "description": format!("Seconds to wait before moving it to the background, up to {MAX_TIMEOUT}.") },
                    "run_in_background": { "type": "boolean", "description": "Start it in the background and return at once. You are told when it finishes." }
                },
                "required": ["command"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "run a shell command: builds, tests, Git, servers, anything the other tools don't cover".into(),
            guidelines: vec![format!(
                "A server or watcher can stay up across turns: start it with bash's run_in_background, and stop it with {STOP_TASK} once it is no longer needed. You are told when a background command finishes, so never poll it, sleep while it runs, or start a second copy."
            )],
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let input: Input = parse(input)?;
        let wait = Duration::from_secs(input.timeout.unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT));
        let tasks = &context.tasks;
        if input.run_in_background {
            let task = tasks.start(&context.cwd, &input.command, &input.description, Kind::Shell, Some(Why::Asked))?;
            return Ok(moved_result(&task, Why::Asked, wait).into());
        }

        let task = tasks.start(&context.cwd, &input.command, &input.description, Kind::Shell, None)?;
        let _holding = Holding(task.clone());
        let early = tokio::select! {
            () = task.ended() => None,
            () = tokio::time::sleep(wait) => Some(Why::TimedOut),
            () = task.detached() => Some(Why::User),
        };
        if let Some(why) = early {
            // ⌘B has already moved it. A timeout moves it now, unless it ended at that moment.
            if why == Why::User || task.move_to_background(why) {
                return Ok(moved_result(&task, why, wait).into());
            }
            task.ended().await;
        }

        let output = task.output();
        let (shown, cut) = tail(&output);
        let text = if cut || task.dropped_output() { format!("[Only the end of the output is shown.]\n{shown}") } else { shown.to_string() };
        let with = |note: String| if text.trim().is_empty() { note } else { format!("{}\n\n{note}", text.trim_end()) };
        match task.info().status {
            wack_agent::tasks::Status::Finished => Ok(if text.trim().is_empty() { "(No output.)".to_string() } else { text }.into()),
            wack_agent::tasks::Status::Failed { code: Some(code), .. } => Err(with(format!("Exit code {code}."))),
            wack_agent::tasks::Status::Failed { .. } => Err(with("The command was killed by a signal.".to_string())),
            wack_agent::tasks::Status::Stopped { .. } => Err(with("The command was stopped.".to_string())),
            wack_agent::tasks::Status::Running => Err(with("The command could not be waited for.".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    fn context(folder: &tempfile::TempDir) -> ToolContext<'static> {
        ToolContext::detached(folder.path().to_path_buf())
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn gone(pid: i32) -> bool {
        for _ in 0..100 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn runs_in_the_working_directory_and_merges_both_streams() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("here.txt"), "").unwrap();
        let output = Bash.run(json!({ "command": "ls; echo oops >&2" }), &context(&folder)).await.unwrap().text;
        assert!(output.contains("here.txt") && output.contains("oops"));
        assert_eq!(Bash.run(json!({ "command": "true" }), &context(&folder)).await.unwrap().text, "(No output.)");
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_output_and_exit_code() {
        let folder = tempfile::tempdir().unwrap();
        let error = Bash.run(json!({ "command": "echo before; exit 3" }), &context(&folder)).await.unwrap_err();
        assert_eq!(error, "before\n\nExit code 3.");
    }

    #[tokio::test]
    async fn provider_keys_are_not_inherited() {
        let folder = tempfile::tempdir().unwrap();
        // SAFETY: this test alone sets this variable, and nothing else reads it.
        unsafe { std::env::set_var("OPENROUTER_API_KEY", "sk-or-test") };
        let output = Bash.run(json!({ "command": "echo \"key=[$OPENROUTER_API_KEY] home=$HOME\"" }), &context(&folder)).await.unwrap().text;
        assert!(output.starts_with("key=[] home=/"), "got {output}");
    }

    #[tokio::test]
    async fn a_timeout_moves_the_command_to_the_background() {
        let folder = tempfile::tempdir().unwrap();
        let context = context(&folder);
        let result = Bash.run(json!({ "command": "echo $$; sleep 60", "timeout": 1 }), &context).await.unwrap().text;
        assert!(result.starts_with("Command did not complete within its 1s timeout and was moved to the background (ID: "), "got {result}");
        let task = context.tasks.running().pop().expect("still running");
        let pid: i32 = task.output().trim().parse().unwrap();
        assert!(alive(pid), "a timeout no longer kills the command");
        context.tasks.terminate_all();
        assert!(gone(pid).await);
    }

    #[tokio::test]
    async fn a_background_command_returns_at_once_and_reports_its_end() {
        let folder = tempfile::tempdir().unwrap();
        let context = context(&folder);
        let started = std::time::Instant::now();
        let result = Bash.run(json!({ "command": "sleep 0.3; echo done", "run_in_background": true }), &context).await.unwrap().text;
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(result.starts_with("Started in the background (ID: ") && !result.contains("No output had been read"), "{result}");
        context.tasks.news().await;
        let news = context.tasks.take_news();
        assert!(news[0].text.contains("finished: exit code 0") && news[0].text.contains("done"), "{}", news[0].text);
    }

    #[tokio::test]
    async fn the_user_can_move_a_running_command_to_the_background() {
        let folder = tempfile::tempdir().unwrap();
        let context = context(&folder);
        let tasks = context.tasks.clone();
        let mover = tokio::spawn(async move {
            while !tasks.has_foreground() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tasks.background_foreground()
        });
        let result = Bash.run(json!({ "command": "sleep 60" }), &context).await.unwrap().text;
        assert!(mover.await.unwrap());
        assert!(result.starts_with("The user moved this command to the background"), "{result}");
        assert_eq!(context.tasks.running().len(), 1);
        context.tasks.terminate_all();
    }

    #[tokio::test]
    async fn a_background_process_does_not_outlive_a_command_that_finished() {
        let folder = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let output = Bash.run(json!({ "command": "sleep 60 & echo $!" }), &context(&folder)).await.unwrap().text;
        assert!(started.elapsed() < Duration::from_secs(10), "the tool must not wait for the background process");
        assert!(gone(output.trim().parse().unwrap()).await);
    }

    #[tokio::test]
    async fn stopping_the_run_kills_the_command() {
        let folder = tempfile::tempdir().unwrap();
        let marker = folder.path().join("pid");
        let context = context(&folder);
        let run = Bash.run(json!({ "command": "echo $$ > pid; sleep 60" }), &context);
        let background = context.tasks.start(folder.path(), "sleep 60", "", Kind::Shell, Some(Why::Asked)).unwrap();
        // The agent loop stops a tool by dropping its future.
        assert!(tokio::time::timeout(Duration::from_millis(500), run).await.is_err());
        let pid: i32 = std::fs::read_to_string(marker).unwrap().trim().parse().unwrap();
        assert!(gone(pid).await);
        assert!(background.info().status.running(), "stopping the run leaves background commands alone");
        context.tasks.terminate_all();
    }

    #[tokio::test]
    async fn long_output_keeps_its_end() {
        let folder = tempfile::tempdir().unwrap();
        let output = Bash.run(json!({ "command": "seq 1 200000" }), &context(&folder)).await.unwrap().text;
        assert!(output.starts_with("[Only the end of the output is shown.]"));
        assert!(output.trim_end().ends_with("200000"));
        assert!(output.len() <= crate::text::MAX_BYTES + 100);
    }
}
