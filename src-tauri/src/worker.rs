use crate::{models::{BuiltinModelSuggestion, PackageRecord, ProviderKind, ProviderRecord, TaskMode, TaskRecord, TaskStatus, ToolCatalogEntry}, storage::MetadataState, subscriptions};
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, process::Stdio, sync::{Arc, Mutex}};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::{ChildStdin, Command}, sync::{oneshot, Mutex as AsyncMutex}};

const PROVIDER_ENVIRONMENT_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY", "GEMINI_API_KEY",
    "GOOGLE_API_KEY", "DEEPSEEK_API_KEY", "XAI_API_KEY", "GROQ_API_KEY",
    "MISTRAL_API_KEY", "CEREBRAS_API_KEY", "TOGETHER_API_KEY", "FIREWORKS_API_KEY",
    "AZURE_OPENAI_API_KEY", "AWS_BEARER_TOKEN_BEDROCK", "HF_TOKEN"
];

pub(crate) fn strip_provider_env(command: &mut Command) {
    for key in PROVIDER_ENVIRONMENT_KEYS { command.env_remove(key); }
}

/// Commands awaiting their `response` line, by command id. Owned by one worker process: its
/// stdout reader resolves them and drops the rest when the process goes away.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

#[derive(Clone)]
pub struct WorkerProcess {
    pub pid: u32,
    pub fingerprint: String,
    stdin: Arc<AsyncMutex<ChildStdin>>,
    pending: Pending,
}

/// How to start a worker beyond the task record itself.
#[derive(Default)]
pub struct WorkerOptions {
    /// Build this task's first session as a fork of another chat's (`init.forkFrom`).
    pub fork_from: Option<Value>,
    /// Wait for `init` to finish, so errors reach the caller and `session_file` is recorded.
    pub wait_ready: bool,
}

const INIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

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

    pub fn task_ids(&self) -> Result<Vec<String>, String> {
        Ok(self.workers.lock().map_err(|_| "Worker lock was poisoned".to_string())?.keys().cloned().collect())
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

/// Enabled resource paths from trusted packages only, grouped by kind.
///
/// This is the single gate on what a session may execute. A package the user installed but
/// never accepted the warning for contributes nothing, and a resource the user switched off
/// contributes nothing — so neither can reach a worker at all.
pub fn resource_paths(packages: &[PackageRecord]) -> Value {
    let mut grouped = serde_json::Map::new();
    for kind in ["extensions", "skills", "prompts", "themes"] {
        let paths: Vec<String> = packages
            .iter()
            .filter(|package| !package.trusted_at.is_empty())
            .flat_map(|package| package.enabled_paths(kind))
            .collect();
        grouped.insert(kind.to_string(), json!(paths));
    }
    Value::Object(grouped)
}

/// A worker is respawned when this changes. Resources are resolved once at spawn time, so they
/// belong here alongside the provider and model; tool toggles do not, because `set_tools`
/// applies them live.
pub fn fingerprint(provider: &ProviderRecord, model_id: &str, resources: &Value) -> Result<String, String> {
    serde_json::to_string(&json!({
        "provider": provider,
        "modelId": model_id,
        "resources": resources,
    })).map_err(|error| error.to_string())
}

pub async fn ensure_worker(
    app: &AppHandle,
    task: &TaskRecord,
    provider: &ProviderRecord,
    api_key: Option<&str>,
) -> Result<(), String> {
    ensure_worker_with(app, task, provider, api_key, WorkerOptions::default()).await
}

pub async fn ensure_worker_with(
    app: &AppHandle,
    task: &TaskRecord,
    provider: &ProviderRecord,
    api_key: Option<&str>,
    options: WorkerOptions,
) -> Result<(), String> {
    let worker_state = app.state::<WorkerState>();
    let wanted_fingerprint = {
        let state = app.state::<MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        fingerprint(provider, &task.model_id, &resource_paths(&data.packages))?
    };
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
    strip_provider_env(&mut command);
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
    let pending: Pending = Arc::default();
    worker_state.insert(task.id.clone(), WorkerProcess {
        pid,
        fingerprint: wanted_fingerprint,
        stdin: Arc::new(AsyncMutex::new(stdin)),
        pending: pending.clone(),
    })?;

    let task_id = task.id.clone();
    let subscription_worker = provider.kind == ProviderKind::Subscription;
    let app_for_process = app.clone();
    tauri::async_runtime::spawn(async move {
        let stdout_app = app_for_process.clone();
        let stdout_task_id = task_id.clone();
        let stdout_reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                handle_worker_line(&stdout_app, &stdout_task_id, &line, &pending);
            }
            // Dropping the senders tells every waiting request the worker is gone.
            if let Ok(mut pending) = pending.lock() { pending.clear(); }
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
            let message = if subscription_worker {
                "The Pi worker stopped unexpectedly while using a subscription.".to_string()
            } else if stderr_message.is_empty() {
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

    let (disabled_tools, resources) = {
        let state = app.state::<MetadataState>();
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.tool_config.disabled.clone(), resource_paths(&data.packages))
    };

    let auth_path = if provider.kind == ProviderKind::Subscription {
        Some(subscriptions::auth_path(app, &provider.id)?)
    } else { None };
    let init = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "init",
        "taskId": task.id,
        "cwd": task.workspace_path,
        "agentDir": agent_dir,
        "sessionDir": session_dir,
        "sessionFile": task.session_file,
        "provider": worker_provider_json(provider),
        "modelId": task.model_id,
        "apiKey": api_key,
        "authPath": auth_path,
        "thinkingLevel": task.thinking_level,
        "disabledTools": disabled_tools,
        "resources": resources,
        // The record's mode is the durable hint; the worker's plan-mode extension reconciles
        // it with whatever the restored session says. A fork takes the mode its fork point had.
        "mode": if options.fork_from.is_some() { Value::Null } else { json!(task.mode) },
        "forkFrom": options.fork_from,
        "subagents": subagent_payload(app)?,
    });
    if options.wait_ready {
        request(app, &task.id, init, INIT_TIMEOUT).await.map(|_| ())
    } else {
        send(app, &task.id, &init).await
    }
}

/// Send a command and wait for its `response`, returning the response's `result`.
///
/// Used for commands whose outcome the caller needs (moving in the session tree, resending) —
/// most commands stay fire-and-forget, with the worker's events telling the UI what happened.
pub async fn request(app: &AppHandle, task_id: &str, value: Value, timeout: std::time::Duration) -> Result<Value, String> {
    let worker = app.state::<WorkerState>().get(task_id)?
        .ok_or_else(|| "This task's Pi worker is not running".to_string())?;
    let id = value.get("id").and_then(Value::as_str).ok_or_else(|| "A worker request needs an id".to_string())?.to_string();
    let (sender, receiver) = oneshot::channel();
    worker.pending.lock().map_err(|_| "Worker lock was poisoned".to_string())?.insert(id.clone(), sender);
    if let Err(error) = write_line(&worker, &value).await {
        if let Ok(mut pending) = worker.pending.lock() { pending.remove(&id); }
        return Err(error);
    }
    match tokio::time::timeout(timeout, receiver).await {
        Err(_) => {
            if let Ok(mut pending) = worker.pending.lock() { pending.remove(&id); }
            Err("Pi did not answer in time.".into())
        }
        Ok(Err(_)) => Err("The Pi worker stopped before it answered.".into()),
        Ok(Ok(response)) => {
            if response.get("success").and_then(Value::as_bool) == Some(true) {
                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            } else {
                Err(response.get("error").and_then(Value::as_str).map(redact_and_limit)
                    .unwrap_or_else(|| "Pi could not do that.".into()))
            }
        }
    }
}

/// Best-effort fan-out to every running worker. A worker that has already exited is skipped
/// rather than failing the whole call: the same value also travels in `init`, so the next
/// spawn picks it up.
pub async fn broadcast(app: &AppHandle, value: &Value) -> Result<(), String> {
    for task_id in app.state::<WorkerState>().task_ids()? {
        let _ = send(app, &task_id, value).await;
    }
    Ok(())
}

/// A connection as the worker's protocol describes it. Only models with confirmed limits are
/// sent: Pi cannot run a model without them.
pub fn worker_provider_json(provider: &ProviderRecord) -> Value {
    let models: Vec<Value> = provider.models.iter().filter_map(|model| {
        Some(json!({
            "id": model.id,
            "name": model.name,
            "contextWindow": model.context_window?,
            "maxTokens": model.max_tokens?,
            "reasoning": model.reasoning,
            "thinkingLevels": model.thinking_levels,
            "thinkingLevelMap": model.thinking_level_map,
            "vision": model.vision,
        }))
    }).collect();
    json!({
        "id": provider.id,
        "name": provider.name,
        "kind": provider.kind,
        "baseUrl": provider.base_url,
        "api": provider.api_format,
        "models": models,
    })
}

/// The `subagents` value for `init` and `set_subagents`: null while sub-agents are off, and
/// otherwise the roster plus the credentials of the connections agents' own models use. Like
/// `init`'s own key, those credentials only ever travel over the worker's stdin.
pub fn subagent_payload(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<MetadataState>();
    let (config, providers) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.subagents.clone(), data.providers.clone())
    };
    Ok(crate::subagents::runtime_payload(&config, &providers, |provider| {
        if provider.kind == ProviderKind::Subscription {
            if !subscriptions::has_credential(app, &provider.id) { return None; }
            let path = subscriptions::auth_path(app, &provider.id).ok()?;
            Some(crate::subagents::ProviderCredential { api_key: None, auth_path: Some(path.to_string_lossy().into_owned()) })
        } else {
            let key = state.secrets.get(&provider.id).ok()?;
            Some(crate::subagents::ProviderCredential { api_key: Some(key), auth_path: None })
        }
    }))
}

/// Push the current sub-agent settings to every running worker. Applied live on the worker's
/// next turn — never a restart, so a running chat is never interrupted by a settings change.
pub async fn broadcast_subagents(app: &AppHandle) -> Result<(), String> {
    let payload = subagent_payload(app)?;
    broadcast(app, &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_subagents", "subagents": payload })).await
}

pub async fn send(app: &AppHandle, task_id: &str, value: &Value) -> Result<(), String> {
    let worker = app.state::<WorkerState>().get(task_id)?
        .ok_or_else(|| "This task's Pi worker is not running".to_string())?;
    write_line(&worker, value).await
}

async fn write_line(worker: &WorkerProcess, value: &Value) -> Result<(), String> {
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

fn manager_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/manager.js"))
    } else {
        Ok(app.path().resource_dir().map_err(|error| error.to_string())?.join("resources/worker/dist/manager.js"))
    }
}

fn catalog_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/catalog.js"))
    } else {
        Ok(app.path().resource_dir().map_err(|error| error.to_string())?.join("resources/worker/dist/catalog.js"))
    }
}

/// Read only the static Pi catalogue in a short-lived, offline process without credentials.
pub async fn list_builtin_models(app: &AppHandle) -> Result<Vec<BuiltinModelSuggestion>, String> {
    let mut command = Command::new(node_executable_path()?);
    command
        .arg(catalog_entry_path(app)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_OFFLINE", "1");
    for key in PROVIDER_ENVIRONMENT_KEYS { command.env_remove(key); }
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "Reading the bundled Pi model catalogue timed out.".to_string())?
        .map_err(|error| format!("Could not read the bundled Pi model catalogue: {error}"))?;
    if !output.status.success() {
        return Err(format!("Could not read the bundled Pi model catalogue: {}",
            redact_and_limit(&String::from_utf8_lossy(&output.stderr))));
    }
    if output.stdout.len() > 2_000_000 {
        return Err("The bundled Pi model catalogue is unexpectedly large.".into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not parse the bundled Pi model catalogue: {error}"))
}

/// The npm staged out of the pinned Node tarball by `scripts/prepare-runtime.mjs`. A packaged
/// .app launched from Finder gets a minimal PATH and usually cannot see a system npm, so Pi is
/// pointed at this copy instead. Falls back to bare `npm` when the staged copy is absent.
fn npm_command(app: &AppHandle) -> Option<Vec<String>> {
    let node = node_executable_path().ok()?;
    let cli = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/npm/bin/npm-cli.js")
    } else {
        app.path().resource_dir().ok()?.join("resources/npm/bin/npm-cli.js")
    };
    if !cli.exists() {
        return None;
    }
    Some(vec![node.to_string_lossy().into_owned(), cli.to_string_lossy().into_owned()])
}

/// Shared store for installed packages: Pi's own settings.json plus npm/ and git/. Deliberately
/// separate from the per-task agent directories, which are deleted with their task.
/// Serializes package operations. Two concurrent installs would race Pi's settings.json and
/// clobber each other's in-memory state.
#[derive(Default)]
pub struct ManagerState {
    lock: AsyncMutex<()>,
}

/// Every manager protocol line carries this prefix. Pi spawns npm with inherited stdio and the
/// override for that is not public API, so npm's own output arrives on the same stream; the
/// prefix is what separates protocol from chatter.
const MANAGER_FRAME: char = '\u{1e}';
const MANAGER_NOISE_LIMIT: usize = 4_000;

/// Run one package operation in a short-lived Node process and return the catalog it produced.
///
/// This process never receives an API key, and unlike a task worker it does not set PI_OFFLINE,
/// because installing is the one thing it exists to do.
pub async fn run_manager(app: &AppHandle, command: Value) -> Result<Value, String> {
    let manager_state = app.state::<ManagerState>();
    let _guard = manager_state.lock.lock().await;

    let pi_dir = package_dir(app)?;
    let entry = manager_entry_path(app)?;
    let node = node_executable_path()?;
    let mut command_builder = Command::new(node);
    command_builder
        .arg(entry)
        .current_dir(&pi_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0")
        .env("PI_SKIP_VERSION_CHECK", "1")
        // Installing is the one thing this process exists for, and Pi gates installs on this.
        .env_remove("PI_OFFLINE");
    // npm and git run as children here; neither has any business seeing a provider key.
    for key in PROVIDER_ENVIRONMENT_KEYS {
        command_builder.env_remove(key);
    }
    let mut process = command_builder
        .spawn()
        .map_err(|error| format!("Could not start the package manager: {error}"))?;
    let mut stdin = process.stdin.take().ok_or_else(|| "The package manager has no stdin".to_string())?;
    let stdout = process.stdout.take().ok_or_else(|| "The package manager has no stdout".to_string())?;
    let stderr = process.stderr.take().ok_or_else(|| "The package manager has no stderr".to_string())?;

    let init = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "type": "init",
        "piDir": pi_dir,
        "npmCommand": npm_command(app),
    });
    let command_id = uuid::Uuid::new_v4().to_string();
    let mut request = command;
    if let Some(object) = request.as_object_mut() {
        object.insert("id".into(), Value::String(command_id.clone()));
    }
    for line in [&init, &request] {
        let mut bytes = serde_json::to_vec(line).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).await.map_err(|error| format!("Could not reach the package manager: {error}"))?;
    }
    stdin.flush().await.map_err(|error| error.to_string())?;

    let stderr_handle = tokio::spawn(async move {
        let mut tail = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tail.push_str(&line);
            tail.push('\n');
        }
        tail
    });

    let mut reader = BufReader::new(stdout).lines();
    let mut catalog = Value::Null;
    let mut noise = String::new();
    let mut outcome: Option<Result<(), String>> = None;
    while let Ok(Some(line)) = reader.next_line().await {
        let Some(payload) = line.strip_prefix(MANAGER_FRAME) else {
            if noise.len() < MANAGER_NOISE_LIMIT { noise.push_str(&line); noise.push('\n'); }
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(payload) else { continue; };
        match value.get("type").and_then(Value::as_str) {
            Some("catalog") => catalog = value.get("packages").cloned().unwrap_or(Value::Null),
            Some("progress") => { let _ = app.emit("package-event", value); }
            Some("manager_error") => {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    outcome = Some(Err(redact_and_limit(message)));
                    break;
                }
            }
            Some("response") if value.get("id").and_then(Value::as_str) == Some(command_id.as_str()) => {
                outcome = Some(if value.get("success").and_then(Value::as_bool) == Some(true) {
                    Ok(())
                } else {
                    Err(redact_and_limit(value.get("error").and_then(Value::as_str).unwrap_or("The package operation failed")))
                });
                break;
            }
            _ => {}
        }
    }

    let _ = process.start_kill();
    let stderr_tail = stderr_handle.await.unwrap_or_default();
    match outcome {
        Some(Ok(())) => Ok(catalog),
        Some(Err(message)) => Err(with_context(message, &noise, &stderr_tail)),
        None => Err(with_context(
            "The package manager stopped before finishing".to_string(),
            &noise,
            &stderr_tail,
        )),
    }
}

/// npm reports the actionable part of a failure on its own streams, so fold a bounded tail of
/// them into the message the user sees.
fn with_context(message: String, noise: &str, stderr: &str) -> String {
    let combined = format!("{noise}{stderr}");
    let detail = combined.trim();
    if detail.is_empty() {
        return message;
    }
    let tail: String = {
        let characters: Vec<char> = detail.chars().collect();
        characters[characters.len().saturating_sub(600)..].iter().collect()
    };
    format!("{message}\n{}", redact_and_limit(&tail))
}

pub fn package_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|error| error.to_string())?.join("pi");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

pub(crate) fn node_executable_path() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(std::env::var_os("WACKCODE_NODE_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("node")))
    } else {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Ok(executable.parent().ok_or_else(|| "Could not locate the app executable directory".to_string())?.join("wackcode-node"))
    }
}

fn handle_worker_line(app: &AppHandle, task_id: &str, line: &str, pending: &Pending) {
    let Ok(mut value) = serde_json::from_str::<Value>(line) else { return; };
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    if event_type == "response" {
        let waiting = value.get("id").and_then(Value::as_str)
            .and_then(|id| pending.lock().ok().and_then(|mut pending| pending.remove(id)));
        // A caller is waiting for this one and reports it itself.
        if let Some(sender) = waiting {
            let _ = sender.send(value);
            return;
        }
    }
    if event_type == "worker_error" {
        if let Some(message) = value.get("message").and_then(Value::as_str).map(redact_and_limit) {
            value["message"] = Value::String(message);
        }
    }
    let mut should_save = false;
    if event_type == "ready" || event_type == "snapshot" || event_type == "snapshot_delta" {
        if let Ok(mut data) = app.state::<MetadataState>().data.lock() {
            // Deltas carry the session file only when it changed; full snapshots always have it.
            let session_file = value
                .pointer("/snapshot/sessionFile")
                .or_else(|| value.pointer("/delta/sessionFile"))
                .and_then(Value::as_str);
            if let Some(session_file) = session_file {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    if task.session_file.as_deref() != Some(session_file) {
                        task.session_file = Some(session_file.to_string());
                        should_save = true;
                    }
                }
            }
            // The catalogue only exists on a live session, so cache the latest one for the
            // Tools panel to render when no chat is open. Deltas never carry tools — they ride
            // full snapshots, which is where tool-changing commands force one.
            if let Some(tools) = value.pointer("/snapshot/tools") {
                if let Ok(catalog) = serde_json::from_value::<Vec<ToolCatalogEntry>>(tools.clone()) {
                    if data.tool_catalog != catalog {
                        data.tool_catalog = catalog;
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
    } else if event_type == "plan_state" {
        // Mirror the worker's mode onto the record, the same way `session_file` is mirrored:
        // the record is the durable hint the UI uses before a worker reports in.
        let mode = match value.get("mode").and_then(Value::as_str) {
            Some("plan") => Some(TaskMode::Plan),
            Some("build") => Some(TaskMode::Build),
            _ => None,
        };
        if let Some(mode) = mode {
            if let Ok(mut data) = app.state::<MetadataState>().data.lock() {
                if let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) {
                    if task.mode != mode {
                        task.mode = mode;
                        task.updated_at = chrono::Utc::now().to_rfc3339();
                        should_save = true;
                    }
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
    for marker in ["access_token=", "refresh_token=", "device_code=", "code="] {
        while let Some(start) = safe.to_ascii_lowercase().find(marker) {
            let end = safe[start..].find(|character: char| character.is_whitespace() || character == '&' || character == '#')
                .map(|index| start + index).unwrap_or(safe.len());
            safe.replace_range(start..end, "[credential redacted]");
        }
    }
    safe.chars().take(1_000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PackageResourceRecord, ProviderRecord};

    fn package(source: &str, trusted: bool, resources: &[(&str, bool)]) -> PackageRecord {
        PackageRecord {
            source: source.into(),
            display_name: source.into(),
            kind: "npm".into(),
            version: None,
            installed_path: None,
            extensions: resources.iter().map(|(path, enabled)| PackageResourceRecord {
                path: (*path).into(), name: (*path).into(), enabled: *enabled,
            }).collect(),
            skills: Vec::new(),
            prompts: Vec::new(),
            themes: Vec::new(),
            errors: Vec::new(),
            trusted_at: if trusted { "2026-01-01T00:00:00Z".into() } else { String::new() },
            installed_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn provider() -> ProviderRecord {
        ProviderRecord {
            id: "p".into(), name: "P".into(), kind: ProviderKind::Custom, base_url: "https://example.test/v1".into(),
            api_format: "openai-completions".into(), models: Vec::new(),
            created_at: "now".into(), updated_at: "now".into(), has_api_key: true, connected: true,
        }
    }

    #[test]
    fn oauth_codes_and_tokens_are_redacted_before_events_or_metadata() {
        let message = "request failed: https://example.test/callback?code=private-code&state=ok access_token=private-token refresh_token=private-refresh";
        let safe = redact_and_limit(message);
        assert!(!safe.contains("private-code"));
        assert!(!safe.contains("private-token"));
        assert!(!safe.contains("private-refresh"));
        assert!(safe.contains("state=ok"));
    }

    #[test]
    fn only_trusted_packages_contribute_resource_paths() {
        let packages = vec![
            package("npm:trusted", true, &[("/pkg/trusted/a.ts", true)]),
            package("npm:untrusted", false, &[("/pkg/untrusted/evil.ts", true)]),
        ];
        let paths = resource_paths(&packages);
        let extensions = paths["extensions"].as_array().unwrap();
        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0], "/pkg/trusted/a.ts");
    }

    #[test]
    fn a_switched_off_resource_never_reaches_a_worker() {
        let packages = vec![package("npm:x", true, &[("/pkg/x/on.ts", true), ("/pkg/x/off.ts", false)])];
        let extensions = resource_paths(&packages)["extensions"].as_array().unwrap().clone();
        assert_eq!(extensions, vec!["/pkg/x/on.ts"]);
    }

    #[test]
    fn an_untrusted_package_contributes_nothing_even_with_every_resource_enabled() {
        let packages = vec![package("npm:appeared-somehow", false, &[
            ("/pkg/a.ts", true), ("/pkg/b.ts", true),
        ])];
        let paths = resource_paths(&packages);
        for kind in ["extensions", "skills", "prompts", "themes"] {
            assert!(paths[kind].as_array().unwrap().is_empty(), "{kind} leaked from an untrusted package");
        }
    }

    #[test]
    fn fingerprint_changes_with_resources_so_workers_respawn() {
        let before = resource_paths(&[package("npm:x", true, &[("/pkg/x/a.ts", true)])]);
        let after = resource_paths(&[package("npm:x", true, &[("/pkg/x/a.ts", false)])]);
        let provider = provider();
        assert_ne!(
            fingerprint(&provider, "m", &before).unwrap(),
            fingerprint(&provider, "m", &after).unwrap()
        );
        assert_eq!(
            fingerprint(&provider, "m", &before).unwrap(),
            fingerprint(&provider, "m", &before).unwrap()
        );
    }

    #[test]
    fn manager_noise_is_bounded_and_redacted() {
        let message = with_context("Install failed".into(), "npm warn deprecated\n", "sk-abcdefghijklmnop leaked");
        assert!(message.starts_with("Install failed\n"));
        assert!(message.contains("[credential redacted]"));
        assert!(!message.contains("sk-abcdefghijklmnop"));
    }
}
