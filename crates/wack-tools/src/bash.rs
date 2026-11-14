//! The shell tool.
//!
//! Each command runs in its own process group, and the whole group is killed when the command
//! ends for any reason: it finished, it timed out, or the user stopped the run (which drops this
//! tool's future). So nothing a command starts outlives it. Commands that should keep running in
//! the background are a later feature with its own tool.
//!
//! API keys that other tools read from the environment are removed from the command's
//! environment. WackCode's own keys never enter the environment at all (they live in
//! `secrets.json`), but the user may have launched the app from a shell that had some.

use std::{
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, process::Command};
use wack_agent::{Loading, Tool, ToolContext, ToolSpec};

use crate::text::{MAX_BYTES, parse, tail};

const DEFAULT_TIMEOUT: u64 = 300;
const MAX_TIMEOUT: u64 = 3_600;
/// How much output is held while a command runs. Only its end is ever shown.
const HELD_BYTES: usize = 4 * MAX_BYTES;

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

pub struct Bash;

#[derive(Deserialize)]
struct Input {
    command: String,
    timeout: Option<u64>,
}

/// Kills a process group when dropped.
struct Group(i32);

impl Drop for Group {
    fn drop(&mut self) {
        // SAFETY: a plain system call. A negative id addresses the group; if every member has
        // already exited the call fails harmlessly.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

/// The end of a command's output, plus how much there was in all.
#[derive(Default)]
struct Output {
    held: Vec<u8>,
    dropped: bool,
}

impl Output {
    fn push(&mut self, chunk: &[u8]) {
        self.held.extend_from_slice(chunk);
        if self.held.len() > 2 * HELD_BYTES {
            let extra = self.held.len() - HELD_BYTES;
            self.held.drain(..extra);
            self.dropped = true;
        }
    }

    fn text(&self) -> String {
        let text = String::from_utf8_lossy(&self.held);
        let (shown, cut) = tail(&text);
        if cut || self.dropped {
            format!("[Only the end of the output is shown.]\n{shown}")
        } else {
            shown.to_string()
        }
    }
}

#[async_trait]
impl Tool for Bash {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "bash".into(),
            description: format!("Runs a command with /bin/bash in the working directory and returns its output (stdout and stderr together) and exit code. Long output is cut to its end. The command is stopped after its timeout ({DEFAULT_TIMEOUT} seconds unless you set one), and anything it started is stopped when it ends, so a server or watcher cannot be left running in the background. Nothing can answer a prompt: use non-interactive flags."),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to run." },
                    "timeout": { "type": "integer", "description": format!("Seconds to allow, up to {MAX_TIMEOUT}. Raise it for long builds and test runs.") }
                },
                "required": ["command"]
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "run a shell command: builds, tests, Git, anything the other tools don't cover".into(),
        }
    }

    async fn run(&self, input: Value, context: &ToolContext) -> Result<String, String> {
        let input: Input = parse(input)?;
        let limit = Duration::from_secs(input.timeout.unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT));

        let mut command = Command::new("/bin/bash");
        command
            .arg("-c")
            .arg(&input.command)
            .current_dir(&context.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        for name in SECRET_ENV {
            command.env_remove(name);
        }
        let mut child = command.spawn().map_err(|error| format!("Could not start the command: {error}."))?;
        let group = child.id().map(|id| Group(id as i32));

        let output = Arc::new(Mutex::new(Output::default()));
        let mut readers = Vec::new();
        let streams: [Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>; 2] = [
            child.stdout.take().map(|stream| Box::new(stream) as _),
            child.stderr.take().map(|stream| Box::new(stream) as _),
        ];
        for mut stream in streams.into_iter().flatten() {
            let output = output.clone();
            readers.push(tokio::spawn(async move {
                let mut buffer = [0u8; 8_192];
                while let Ok(read) = stream.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    output.lock().unwrap().push(&buffer[..read]);
                }
            }));
        }

        let status = tokio::time::timeout(limit, child.wait()).await;
        // Whatever the command left running holds the output pipes open; killing the group is
        // what lets the readers finish.
        drop(group);
        for reader in readers {
            let _ = tokio::time::timeout(Duration::from_secs(2), reader).await;
        }
        let text = output.lock().unwrap().text();
        let with = |note: String| if text.trim().is_empty() { note } else { format!("{}\n\n{note}", text.trim_end()) };
        match status {
            Err(_) => {
                let _ = child.wait().await;
                Err(with(format!("The command was stopped: it ran longer than {} seconds. Set a longer timeout if it needs one.", limit.as_secs())))
            }
            Ok(Err(error)) => Err(with(format!("The command could not be waited for: {error}."))),
            Ok(Ok(status)) if status.success() => Ok(if text.trim().is_empty() { "(No output.)".to_string() } else { text }),
            Ok(Ok(status)) => Err(with(match status.code() {
                Some(code) => format!("Exit code {code}."),
                None => "The command was killed by a signal.".to_string(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn context(folder: &tempfile::TempDir) -> ToolContext {
        ToolContext { cwd: folder.path().to_path_buf(), cancel: CancellationToken::new() }
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
        let output = Bash.run(json!({ "command": "ls; echo oops >&2" }), &context(&folder)).await.unwrap();
        assert!(output.contains("here.txt") && output.contains("oops"));
        assert_eq!(Bash.run(json!({ "command": "true" }), &context(&folder)).await.unwrap(), "(No output.)");
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
        let output = Bash.run(json!({ "command": "echo \"key=[$OPENROUTER_API_KEY] home=$HOME\"" }), &context(&folder)).await.unwrap();
        assert!(output.starts_with("key=[] home=/"), "got {output}");
    }

    #[tokio::test]
    async fn a_timeout_stops_the_command_and_everything_it_started() {
        let folder = tempfile::tempdir().unwrap();
        let error = Bash
            .run(json!({ "command": "sleep 60 & echo $!; sleep 60", "timeout": 1 }), &context(&folder))
            .await
            .unwrap_err();
        assert!(error.contains("ran longer than 1 seconds"), "got {error}");
        let pid: i32 = error.lines().next().unwrap().trim().parse().unwrap();
        assert!(gone(pid).await, "the background process must die with the command");
    }

    #[tokio::test]
    async fn a_background_process_does_not_outlive_a_command_that_finished() {
        let folder = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let output = Bash.run(json!({ "command": "sleep 60 & echo $!" }), &context(&folder)).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "the tool must not wait for the background process");
        assert!(gone(output.trim().parse().unwrap()).await);
    }

    #[tokio::test]
    async fn stopping_the_run_kills_the_command() {
        let folder = tempfile::tempdir().unwrap();
        let marker = folder.path().join("pid");
        let context = context(&folder);
        let run = Bash.run(json!({ "command": "echo $$ > pid; sleep 60" }), &context);
        // The agent loop stops a tool by dropping its future.
        assert!(tokio::time::timeout(Duration::from_millis(500), run).await.is_err());
        let pid: i32 = std::fs::read_to_string(marker).unwrap().trim().parse().unwrap();
        assert!(gone(pid).await);
    }

    #[tokio::test]
    async fn long_output_keeps_its_end() {
        let folder = tempfile::tempdir().unwrap();
        let output = Bash.run(json!({ "command": "seq 1 200000" }), &context(&folder)).await.unwrap();
        assert!(output.starts_with("[Only the end of the output is shown.]"));
        assert!(output.trim_end().ends_with("200000"));
        assert!(output.len() <= MAX_BYTES + 100);
    }
}
