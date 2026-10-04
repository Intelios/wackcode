//! Cold chat history is a read-only operation. The reader projects the saved session without
//! creating an agent, loading extensions, touching credentials, or needing the workspace/model.
//! Callers hold TaskLocks through emission so a cold read cannot land after a new live run.
use crate::{models::TaskRecord, worker};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::{io::AsyncWriteExt, process::Command};

pub async fn read(
    app: &AppHandle,
    task: &TaskRecord,
    context_window: Option<u64>,
) -> Result<Value, String> {
    let reader = if tauri::is_dev() {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/session-reader.js")
    } else {
        app.path()
            .resource_dir()
            .map_err(|_| "The app resources could not be found.".to_string())?
            .join("resources/worker/dist/session-reader.js")
    };
    let mut command = Command::new(worker::node_executable_path()?);
    command
        .arg(reader)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_OFFLINE", "1");
    // Dev builds resolve `node` through PATH; the packaged runtime has an absolute path.
    // Keep only this lookup variable, never the rest of the host's environment.
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let input = json!({
        "taskId": task.id, "sessionFile": task.session_file,
        "mode": task.mode, "thinkingLevel": task.thinking_level, "contextWindow": context_window
    });
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        let mut child = command
            .spawn()
            .map_err(|_| "Could not start the saved-history reader.".to_string())?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "The saved-history reader has no input pipe.".to_string())?;
        stdin
            .write_all(input.to_string().as_bytes())
            .await
            .map_err(|_| "Could not send this chat to the saved-history reader.".to_string())?;
        drop(stdin);
        child
            .wait_with_output()
            .await
            .map_err(|_| "Could not read this chat's saved history.".to_string())
    })
    .await
    .map_err(|_| "Reading this chat's saved history timed out.".to_string())??;
    if !output.status.success() {
        // The helper has no credentials; its errors never include session contents.
        return Err(
            "Could not read this chat's saved history. Its session file may be missing or damaged."
                .into(),
        );
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "The saved-history reader returned an invalid transcript.".to_string())
}
