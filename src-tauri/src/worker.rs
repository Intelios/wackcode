use crate::{models::{ProviderRecord, TaskRecord, TaskStatus}, storage::MetadataState};
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::{Arc, Mutex}};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::{ChildStdin, Command}, sync::Mutex as AsyncMutex};

const PROVIDER_ENVIRONMENT_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY", "GEMINI_API_KEY",
    "GOOGLE_API_KEY", "DEEPSEEK_API_KEY", "XAI_API_KEY", "GROQ_API_KEY",
    "MISTRAL_API_KEY", "CEREBRAS_API_KEY", "TOGETHER_API_KEY", "FIREWORKS_API_KEY",
    "AZURE_OPENAI_API_KEY", "AWS_BEARER_TOKEN_BEDROCK", "HF_TOKEN"
];

#[derive(Clone)]
pub struct WorkerProcess {
    pub pid: u32,
    pub fingerprint: String,
    stdin: Arc<AsyncMutex<ChildStdin>>,
}

#[derive(Default)]
pub struct WorkerState {
    workers: Mutex<HashMap<String, WorkerProcess>>,
}

impl WorkerState {
    pub fn get(&self, task_id: &str) -> Result<Option<WorkerProcess>, String> {
        Ok(self.workers.lock().map_err(|_| "Worker lock was poisoned".to_string())?.get(task_id).cloned())
    }

    fn insert(&self, task_id: String, worker: WorkerProcess) -> Result<(), String> {
        self.workers.lock().map_err(|_| "Worker lock was poisoned".to_string())?.insert(task_id, worker);
        Ok(())
    }

    fn remove_if_pid(&self, task_id: &str, pid: u32) -> Result<bool, String> {
        let mut workers = self.workers.lock().map_err(|_| "Worker lock was poisoned".to_string())?;
        if workers.get(task_id).is_some_and(|worker| worker.pid == pid) {
            workers.remove(task_id);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn remove(&self, task_id: &str) -> Result<Option<WorkerProcess>, String> {
        Ok(self.workers.lock().map_err(|_| "Worker lock was poisoned".to_string())?.remove(task_id))
    }

    pub fn terminate_all(&self) {
        if let Ok(mut workers) = self.workers.lock() {
            for (_, worker) in workers.drain() {
                let _ = killpg(Pid::from_raw(worker.pid as i32), Signal::SIGTERM);
            }
        }
    }
}

pub fn fingerprint(provider: &ProviderRecord, model_id: &str) -> Result<String, String> {
    serde_json::to_string(&json!({
        "provider": provider,
        "modelId": model_id,
    })).map_err(|error| error.to_string())
}

pub async fn ensure_worker(
    app: &AppHandle,
    task: &TaskRecord,
    provider: &ProviderRecord,
    api_key: &str,
) -> Result<(), String> {
    let worker_state = app.state::<WorkerState>();
    let wanted_fingerprint = fingerprint(provider, &task.model_id)?;
    if let Some(existing) = worker_state.get(&task.id)? {
        if existing.fingerprint == wanted_fingerprint { return Ok(()); }
        terminate_worker(app, &task.id, true).await?;
    }

    let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let agent_dir = app_data.join("agent").join(&task.id);
    let session_dir = app_data.join("sessions").join(&task.id);
    std::fs::create_dir_all(&agent_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&session_dir).map_err(|error| error.to_string())?;

    let worker_path = worker_entry_path(app)?;
    let node_path = node_executable_path()?;
    let mut command = Command::new(node_path);
    command
        .arg(worker_path)
        .current_dir(&task.workspace_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_OFFLINE", "1");
    for key in PROVIDER_ENVIRONMENT_KEYS { command.env_remove(key); }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn().map_err(|error| format!("Could not start the bundled Pi worker: {error}"))?;
    let pid = child.id().ok_or_else(|| "The Pi worker did not return a process id".to_string())?;
    let stdin = child.stdin.take().ok_or_else(|| "The Pi worker has no stdin".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "The Pi worker has no stdout".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "The Pi worker has no stderr".to_string())?;
    worker_state.insert(task.id.clone(), WorkerProcess {
        pid,
        fingerprint: wanted_fingerprint,
        stdin: Arc::new(AsyncMutex::new(stdin)),
    })?;

    let task_id = task.id.clone();
    let app_for_process = app.clone();
    tauri::async_runtime::spawn(async move {
        let stdout_app = app_for_process.clone();
        let stdout_task_id = task_id.clone();
        let stdout_reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                handle_worker_line(&stdout_app, &stdout_task_id, &line);
            }
        });
        let stderr_tail = Arc::new(AsyncMutex::new(String::new()));
        let stderr_copy = stderr_tail.clone();
        let stderr_reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut tail = stderr_copy.lock().await;
                tail.push_str(&line);
                tail.push('\n');
                if tail.len() > 4_000 { *tail = tail[tail.len() - 4_000..].to_string(); }
            }
        });
        let _status = child.wait().await;
        let _ = stdout_reader.await;
        let _ = stderr_reader.await;
        let stderr_message = stderr_tail.lock().await.trim().to_string();
        // Intentional shutdown paths remove the process from the registry before
        // signaling it. Any process that exits while still registered is a crash,
        // including a nominal exit code caused by an external SIGTERM handler.
        let unexpected = app_for_process.state::<WorkerState>().remove_if_pid(&task_id, pid).unwrap_or(true);
        if unexpected {
            let message = if stderr_message.is_empty() {
                "The Pi worker stopped unexpectedly.".to_string()
            } else {
                format!("The Pi worker stopped unexpectedly: {}", redact_and_limit(&stderr_message))
            };
            let _ = app_for_process.state::<MetadataState>().mutate(|data| {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
                        task.status = TaskStatus::Interrupted;
                    }
                    task.last_error = Some(message.clone());
                }
                Ok(())
            });
            let _ = app_for_process.emit("worker-event", json!({
                "type": "worker_error", "taskId": task_id, "message": message
            }));
        }
    });

    let models: Vec<Value> = provider.models.iter().filter_map(|model| {
        Some(json!({
            "id": model.id,
            "name": model.name,
            "contextWindow": model.context_window?,
            "maxTokens": model.max_tokens?,
            "reasoning": model.reasoning,
            "thinkingLevels": model.thinking_levels,
            "thinkingLevelMap": model.thinking_level_map,
        }))
    }).collect();
    let init = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "init",
        "taskId": task.id,
        "cwd": task.workspace_path,
        "agentDir": agent_dir,
        "sessionDir": session_dir,
        "sessionFile": task.session_file,
        "provider": {
            "id": provider.id,
            "name": provider.name,
            "baseUrl": provider.base_url,
            "api": provider.api_format,
            "models": models,
        },
        "modelId": task.model_id,
        "apiKey": api_key,
        "thinkingLevel": task.thinking_level,
    });
    send(app, &task.id, &init).await
}

pub async fn send(app: &AppHandle, task_id: &str, value: &Value) -> Result<(), String> {
    let worker = app.state::<WorkerState>().get(task_id)?
        .ok_or_else(|| "This task's Pi worker is not running".to_string())?;
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let mut stdin = worker.stdin.lock().await;
    stdin.write_all(&bytes).await.map_err(|error| format!("Could not send a command to Pi: {error}"))?;
    stdin.flush().await.map_err(|error| error.to_string())
}

pub async fn terminate_worker(app: &AppHandle, task_id: &str, graceful: bool) -> Result<(), String> {
    let Some(worker) = app.state::<WorkerState>().remove(task_id)? else { return Ok(()); };
    if graceful {
        let value = json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "shutdown" });
        let mut bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        if let Ok(mut stdin) = worker.stdin.try_lock() {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.flush().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let _ = killpg(Pid::from_raw(worker.pid as i32), Signal::SIGTERM);
    Ok(())
}

fn worker_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/index.js"))
    } else {
        Ok(app.path().resource_dir().map_err(|error| error.to_string())?.join("resources/worker/dist/index.js"))
    }
}

fn node_executable_path() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(std::env::var_os("WACKCODE_NODE_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("node")))
    } else {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Ok(executable.parent().ok_or_else(|| "Could not locate the app executable directory".to_string())?.join("wackcode-node"))
    }
}

fn handle_worker_line(app: &AppHandle, task_id: &str, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else { return; };
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let mut should_save = false;
    if event_type == "ready" || event_type == "snapshot" {
        if let Some(session_file) = value.pointer("/snapshot/sessionFile").and_then(Value::as_str) {
            if let Ok(mut data) = app.state::<MetadataState>().data.lock() {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    if task.session_file.as_deref() != Some(session_file) {
                        task.session_file = Some(session_file.to_string());
                        should_save = true;
                    }
                }
            }
        }
    } else if event_type == "run_state" {
        let next = match value.get("state").and_then(Value::as_str) {
            Some("running") => Some(TaskStatus::Running),
            Some("stopping") => Some(TaskStatus::Stopping),
            Some("interrupted") => Some(TaskStatus::Interrupted),
            Some("idle") => Some(TaskStatus::Idle),
            _ => None,
        };
        if let Some(next) = next {
            if let Ok(mut data) = app.state::<MetadataState>().data.lock() {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    task.status = next;
                    if event_type == "run_state" && value.get("state").and_then(Value::as_str) == Some("running") {
                        task.last_error = None;
                    }
                    task.updated_at = chrono::Utc::now().to_rfc3339();
                    should_save = true;
                }
            }
        }
    } else if event_type == "worker_error" {
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            if let Ok(mut data) = app.state::<MetadataState>().data.lock() {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    task.last_error = Some(redact_and_limit(message));
                    should_save = true;
                }
            }
        }
    }
    if should_save { let _ = app.state::<MetadataState>().save(); }
    let _ = app.emit("worker-event", value);
}

fn redact_and_limit(message: &str) -> String {
    let mut safe = message.to_string();
    for marker in ["sk-", "Bearer "] {
        while let Some(start) = safe.find(marker) {
            let end = safe[start..].find(char::is_whitespace).map(|index| start + index).unwrap_or(safe.len());
            safe.replace_range(start..end, "[credential redacted]");
        }
    }
    safe.chars().take(1_000).collect()
}
