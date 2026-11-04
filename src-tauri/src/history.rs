//! Cold history is keyless and read-only. Only the initial snapshot holds TaskLocks; image
//! previews stream afterward. A load ticket owns cancellation and process cleanup, and every
//! emission is serialized with live-worker readiness and replacement by a newer cold load.
use crate::{
    models::{HistoryPreview, ModelRecord, TaskRecord, ToolCatalogEntry},
    worker,
};
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    future::{poll_fn, Future},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    task::Poll,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    sync::watch,
};

const READ_TIMEOUT: Duration = Duration::from_secs(30);

struct ActiveLoad {
    task_id: String,
    history_id: String,
    cancel: watch::Sender<bool>,
    pid: Option<u32>,
}

impl ActiveLoad {
    fn cancel(self) {
        let _ = self.cancel.send(true);
        // App exit cannot wait for the async reader to notice cancellation. A helper owns
        // only its own image threads, so killing its PID takes down the whole read operation.
        if let Some(pid) = self.pid {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    }
}

#[derive(Clone, Default)]
pub struct HistoryState(Arc<Mutex<Option<ActiveLoad>>>);

impl HistoryState {
    /// Called before waiting for a task lock: an obsolete queued open cannot revive a reader.
    pub fn begin(&self, task_id: &str, history_id: String) -> LoadTicket {
        let (cancel, receiver) = watch::channel(false);
        let mut active = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous) = active.take() {
            previous.cancel();
        }
        *active = Some(ActiveLoad {
            task_id: task_id.into(),
            history_id: history_id.clone(),
            cancel,
            pid: None,
        });
        LoadTicket {
            state: self.clone(),
            task_id: task_id.into(),
            history_id,
            cancel: receiver,
        }
    }

    pub fn cancel(&self, task_id: &str, history_id: Option<&str>) {
        let mut active = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active.as_ref().is_some_and(|load| {
            load.task_id == task_id && history_id.is_none_or(|id| id == load.history_id)
        }) {
            if let Some(previous) = active.take() {
                previous.cancel();
            }
        }
    }

    pub fn stop(&self) {
        if let Some(previous) = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            previous.cancel();
        }
    }

    /// WorkerState holds its readiness lock before entering here. Keep that lock order; a
    /// preview can land before a ready snapshot, but never after one has taken authority.
    pub fn emit_if_current(
        &self,
        app: &AppHandle,
        task_id: &str,
        history_id: &str,
        event: Value,
    ) -> Result<bool, String> {
        let active = self
            .0
            .lock()
            .map_err(|_| "The saved-history reader lock was poisoned.".to_string())?;
        if !active
            .as_ref()
            .is_some_and(|load| load.task_id == task_id && load.history_id == history_id)
        {
            return Ok(false);
        }
        app.emit("worker-event", event)
            .map_err(|_| "Could not show this chat's saved history.".to_string())?;
        Ok(true)
    }
}

pub struct LoadTicket {
    state: HistoryState,
    pub task_id: String,
    pub history_id: String,
    cancel: watch::Receiver<bool>,
}

impl LoadTicket {
    pub fn is_current(&self) -> bool {
        // Release watch's read guard before taking the registry lock: cancellation owns
        // that lock while sending on the watch channel.
        let cancelled = *self.cancel.borrow();
        !cancelled
            && self.state.0.lock().ok().is_some_and(|active| {
                active.as_ref().is_some_and(|load| {
                    load.task_id == self.task_id && load.history_id == self.history_id
                })
            })
    }

    fn set_pid(&self, pid: Option<u32>) {
        if let Ok(mut active) = self.state.0.lock() {
            if let Some(load) = active
                .as_mut()
                .filter(|load| load.task_id == self.task_id && load.history_id == self.history_id)
            {
                load.pid = pid;
            }
        }
    }
}

impl Drop for LoadTicket {
    fn drop(&mut self) {
        self.state.cancel(&self.task_id, Some(&self.history_id));
    }
}

/// One NDJSON line from the reader. The snapshot arrives as raw JSON: the host forwards
/// it verbatim to the renderer instead of paying a full Value parse and re-serialize on
/// multi-megabyte transcripts. Preview batches stay typed; they are small.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryFrame {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub snapshot: Option<Box<serde_json::value::RawValue>>,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub previews: Vec<HistoryPreview>,
    #[serde(default)]
    pub done: bool,
}

pub struct HistoryReader {
    load: LoadTicket,
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

pub struct InitialHistory {
    pub snapshot: Box<serde_json::value::RawValue>,
    pub reader: HistoryReader,
}

impl HistoryReader {
    pub fn history_id(&self) -> &str {
        &self.load.history_id
    }

    async fn next(&mut self) -> Result<Option<HistoryFrame>, String> {
        if !self.load.is_current() {
            return Ok(None);
        }
        let cancelled = std::pin::pin!(self.load.cancel.changed());
        let line = std::pin::pin!(tokio::time::timeout(READ_TIMEOUT, self.lines.next_line()));
        let mut cancelled = cancelled;
        let mut line = line;
        let line = poll_fn(|cx| {
            if cancelled.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Ok(None));
            }
            match line.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => Poll::Ready(
                    result
                        .map_err(|_| "Reading this chat's saved history timed out.".to_string())?
                        .map_err(|_| "Could not read this chat's saved history.".to_string()),
                ),
            }
        })
        .await?;
        line.map(|line| {
            serde_json::from_str(&line)
                .map_err(|_| "The saved-history reader returned an invalid transcript.".to_string())
        })
        .transpose()
    }

    async fn next_of(&mut self, kind: &str) -> Result<Option<HistoryFrame>, String> {
        match self.next().await? {
            Some(frame) if frame.kind == kind => Ok(Some(frame)),
            _ => Ok(None),
        }
    }

    pub async fn stop(mut self) {
        // Clear the registered PID before reaping so a concurrent cancellation cannot signal
        // a PID after the OS has recycled it. kill_on_drop remains the async-abort fallback.
        self.load.set_pid(None);
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    /// The task lock has been released. Only image data may change from this point onward.
    pub fn stream(mut self, app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                let (previews, done) = match self.next_of("history_previews").await {
                    Ok(Some(frame)) => (frame.previews, frame.done),
                    // Cancellation is already stale and emission below is ignored. EOF,
                    // malformed output or a stuck image ends the remaining pending previews.
                    _ => (Vec::new(), true),
                };
                let mut event = json!({ "type": "history_previews", "taskId": self.load.task_id,
                    "historyId": self.load.history_id, "previews": previews });
                if done {
                    event["done"] = json!(true);
                }
                let emitted = app
                    .state::<worker::WorkerState>()
                    .emit_history_if_cold(&app, &self.load.task_id, &self.load.history_id, event)
                    .unwrap_or(false);
                if done || !emitted {
                    break;
                }
            }
            self.stop().await;
        });
    }
}

pub async fn read(
    app: &AppHandle,
    task: &TaskRecord,
    context_window: Option<u64>,
    model: Option<&ModelRecord>,
    tools: &[ToolCatalogEntry],
    load: LoadTicket,
) -> Result<Option<InitialHistory>, String> {
    if !load.is_current() {
        return Ok(None);
    }
    let started = Instant::now();
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
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the saved-history reader.".to_string())?;
    load.set_pid(child.id());
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "The saved-history reader has no input pipe.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "The saved-history reader has no output pipe.".to_string())?;
    if let Some(stderr) = child.stderr.take() {
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // Never log helper errors: session/parser errors can contain private contents.
                // Only the helper's explicitly opted-in scalar timing records are printable.
                if cfg!(debug_assertions) && line.starts_with("history timing: ") {
                    if let Ok(value) = serde_json::from_str::<Value>(&line[16..]) {
                        if let Some(record) = timing_record(&value) {
                            eprintln!("history timing: {record}");
                        }
                    }
                }
            }
        });
    }
    // Host-owned fields travel with the request so the reader can stamp them onto the
    // snapshot itself; Rust then forwards the snapshot bytes untouched.
    let input = json!({ "taskId": task.id, "sessionFile": task.session_file, "mode": task.mode,
        "thinkingLevel": task.thinking_level, "contextWindow": context_window, "debugTimings": cfg!(debug_assertions),
        "model": { "provider": task.provider_id, "id": task.model_id, "name": model.map(|model| &model.name) },
        "availableThinkingLevels": model.map(|model| &model.thinking_levels).cloned().unwrap_or_default(),
        "modelMissing": model.is_none(), "tools": tools });
    let mut reader = HistoryReader {
        load,
        child,
        lines: BufReader::new(stdout).lines(),
    };
    if stdin.write_all(input.to_string().as_bytes()).await.is_err() {
        reader.stop().await;
        return Err("Could not send this chat to the saved-history reader.".into());
    }
    drop(stdin);
    match reader.next_of("snapshot").await {
        Ok(Some(frame)) if frame.snapshot.is_some() => {
            if cfg!(debug_assertions) {
                eprintln!(
                    "history timing: {}",
                    json!({ "phase": "initial", "durationMs": started.elapsed().as_secs_f64() * 1000.0,
                "messages": frame.message_count.unwrap_or(0) })
                );
            }
            Ok(Some(InitialHistory { snapshot: frame.snapshot.unwrap(), reader }))
        }
        result => {
            let cancelled = !reader.load.is_current();
            reader.stop().await;
            if cancelled {
                Ok(None)
            } else {
                Err(result.err().unwrap_or_else(|| "Could not read this chat's saved history. Its session file may be missing or damaged.".into()))
            }
        }
    }
}

/// Whitelist timing fields rather than forwarding arbitrary helper stderr.
fn timing_record(value: &Value) -> Option<Value> {
    let phase = value.get("phase")?.as_str()?;
    if !matches!(
        phase,
        "projection" | "previews" | "initial" | "snapshot" | "prepare" | "read" | "normalize" | "stats"
    ) {
        return None;
    }
    let mut record = json!({ "phase": phase, "durationMs": value.get("durationMs")?.as_f64()? });
    for key in ["messages", "images"] {
        if let Some(count) = value.get(key).and_then(Value::as_u64) {
            record[key] = json!(count);
        }
    }
    Some(record)
}

#[tauri::command]
pub fn cancel_history(app: AppHandle, task_id: String, history_id: Option<String>) {
    app.state::<HistoryState>()
        .cancel(&task_id, history_id.as_deref());
}

#[cfg(debug_assertions)]
#[tauri::command]
pub fn developer_record_history_timing(
    snapshot_ms: f64,
    commit_ms: f64,
    paint_ms: f64,
    message_count: usize,
    previews_done_ms: Option<f64>,
) {
    if [snapshot_ms, commit_ms, paint_ms]
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
        && previews_done_ms.is_none_or(|value| value.is_finite() && value >= 0.0)
    {
        eprintln!(
            "history timing: {}",
            json!({ "phase": "display", "snapshotMs": snapshot_ms,
            "commitMs": commit_ms, "paintMs": paint_ms, "messages": message_count,
            "previewsDoneMs": previews_done_ms })
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    async fn fixture_reader(
        state: &HistoryState,
        id: &str,
    ) -> (HistoryReader, tokio::process::ChildStdin, u32) {
        // cat keeps stdout open until the test closes stdin: the initial frame can be read
        // while later preview work is deliberately blocked, with no shell/grandchild process.
        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let load = state.begin("chat", id.into());
        load.set_pid(Some(pid));
        (
            HistoryReader {
                load,
                child,
                lines: BufReader::new(stdout).lines(),
            },
            stdin,
            pid,
        )
    }

    #[test]
    fn initial_frame_does_not_wait_for_process_exit_or_thumbnail_work() {
        runtime().block_on(async {
            let state = HistoryState::default();
            let (mut reader, mut stdin, _) = fixture_reader(&state, "first").await;
            stdin
                .write_all(b"{\"type\":\"snapshot\",\"snapshot\":{\"messages\":[]},\"messageCount\":0}\n")
                .await
                .unwrap();
            let frame = tokio::time::timeout(Duration::from_secs(1), reader.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(frame.kind, "snapshot");
            assert_eq!(frame.message_count, Some(0));
            assert!(frame.snapshot.is_some());
            assert!(reader.child.try_wait().unwrap().is_none());
            state.cancel("chat", None);
            assert!(reader.next().await.unwrap().is_none());
            reader.stop().await;
        });
    }

    #[test]
    fn cancelling_before_initial_frame_stops_and_reaps_the_helper() {
        runtime().block_on(async {
            let state = HistoryState::default();
            let (mut reader, _stdin, pid) = fixture_reader(&state, "first").await;
            state.cancel("chat", Some("first"));
            assert!(reader.next().await.unwrap().is_none());
            reader.stop().await;
            assert_eq!(
                kill(Pid::from_raw(pid as i32), None),
                Err(nix::errno::Errno::ESRCH)
            );
        });
    }

    #[test]
    fn replacing_a_reader_terminates_its_process_without_killing_the_new_load() {
        runtime().block_on(async {
            let state = HistoryState::default();
            let (reader, _stdin, pid) = fixture_reader(&state, "first").await;
            let next = state.begin("chat", "next".into());
            reader.stop().await;
            assert_eq!(
                kill(Pid::from_raw(pid as i32), None),
                Err(nix::errno::Errno::ESRCH)
            );
            assert!(next.is_current());
        });
    }

    #[test]
    fn malformed_frames_never_expose_their_content_in_errors() {
        runtime().block_on(async {
            let state = HistoryState::default();
            let (mut reader, mut stdin, _) = fixture_reader(&state, "first").await;
            stdin
                .write_all(b"private conversation text\n")
                .await
                .unwrap();
            assert_eq!(
                reader.next().await.unwrap_err(),
                "The saved-history reader returned an invalid transcript."
            );
            reader.stop().await;
        });
    }

    #[test]
    fn replacing_a_load_cancels_it_even_while_waiting_for_the_task_lock() {
        let state = HistoryState::default();
        let first = state.begin("chat", "first".into());
        let next = state.begin("chat", "next".into());
        assert!(!first.is_current());
        assert!(*first.cancel.borrow());
        assert!(next.is_current());
        drop(first);
        assert!(next.is_current());
    }

    #[test]
    fn navigation_shutdown_and_takeover_cancel_owned_loads() {
        let state = HistoryState::default();
        let first = state.begin("first-chat", "first".into());
        let next = state.begin("next-chat", "next".into());
        assert!(!first.is_current());
        state.cancel("first-chat", None);
        assert!(next.is_current());
        state.cancel("next-chat", None);
        assert!(!next.is_current());
        let final_load = state.begin("final-chat", "final".into());
        state.stop();
        assert!(!final_load.is_current());
    }

    #[test]
    fn late_cleanup_cannot_cancel_a_replacement() {
        let state = HistoryState::default();
        let _first = state.begin("chat", "first".into());
        let next = state.begin("chat", "next".into());
        state.cancel("chat", Some("first"));
        assert!(next.is_current());
    }

    #[test]
    fn only_scalar_timing_fields_are_forwarded() {
        assert_eq!(
            timing_record(
                &json!({ "phase": "projection", "durationMs": 10, "images": 2, "private": "secret" })
            ),
            Some(json!({ "phase": "projection", "durationMs": 10.0, "images": 2 }))
        );
        assert!(timing_record(&json!({ "phase": "private text", "durationMs": 10 })).is_none());
        assert!(
            timing_record(&json!({ "phase": "projection", "durationMs": "private text" }))
                .is_none()
        );
    }
}
