use crate::{
    models::{
        BuiltinModelSuggestion, PackageRecord, ProviderKind, ProviderRecord, TaskMode, TaskRecord,
        TaskStatus, ToolCatalogEntry,
    },
    storage::MetadataState,
    subscriptions,
};
use nix::{
    sys::signal::{killpg, Signal},
    unistd::Pid,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::{oneshot, Mutex as AsyncMutex},
};

const PROVIDER_ENVIRONMENT_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
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

pub(crate) fn strip_provider_env(command: &mut Command) {
    for key in PROVIDER_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
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

/// A worker whose chat is not the open one is stopped once its last output is this old. Every
/// command that needs the chat goes through `ensure_worker`, which respawns it transparently.
const IDLE_WORKER_AFTER: Duration = Duration::from_secs(15 * 60);
/// A sweep never stops the most recently active workers below this count, so chats the user
/// cycles through reopen instantly.
const IDLE_WORKER_KEEP: usize = 4;
const IDLE_WORKER_SWEEP: Duration = Duration::from_secs(60);
/// A sweep waits at most this long for a chat's task lock; a busy chat is simply skipped until
/// the next sweep.
const IDLE_WORKER_LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct WorkerState {
    workers: Mutex<HashMap<String, WorkerProcess>>,
}

impl WorkerState {
    pub fn get(&self, task_id: &str) -> Result<Option<WorkerProcess>, String> {
        Ok(self
            .workers
            .lock()
            .map_err(|_| "Worker lock was poisoned".to_string())?
            .get(task_id)
            .cloned())
    }

    fn insert(&self, task_id: String, worker: WorkerProcess) -> Result<(), String> {
        self.workers
            .lock()
            .map_err(|_| "Worker lock was poisoned".to_string())?
            .insert(task_id, worker);
        Ok(())
    }

    fn remove_if_pid(&self, task_id: &str, pid: u32) -> Result<bool, String> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| "Worker lock was poisoned".to_string())?;
        if workers.get(task_id).is_some_and(|worker| worker.pid == pid) {
            workers.remove(task_id);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn task_ids(&self) -> Result<Vec<String>, String> {
        Ok(self
            .workers
            .lock()
            .map_err(|_| "Worker lock was poisoned".to_string())?
            .keys()
            .cloned()
            .collect())
    }

    pub fn remove(&self, task_id: &str) -> Result<Option<WorkerProcess>, String> {
        Ok(self
            .workers
            .lock()
            .map_err(|_| "Worker lock was poisoned".to_string())?
            .remove(task_id))
    }

    pub fn terminate_all(&self) {
        if let Ok(mut workers) = self.workers.lock() {
            for (_, worker) in workers.drain() {
                let _ = killpg(Pid::from_raw(worker.pid as i32), Signal::SIGTERM);
            }
        }
    }
}

/// The chat the user has open. Runtime-only: the idle reaper never stops its worker, so walking
/// away from the app never kills the chat you come back to. Recorded on selection and on spawn
/// (spawning means the user is working in that chat, even before selection catches up).
#[derive(Default)]
pub struct SelectedTask(Mutex<Option<String>>);

impl SelectedTask {
    pub fn set(&self, task_id: &str) {
        if let Ok(mut selected) = self.0.lock() {
            *selected = Some(task_id.to_string());
        }
    }

    fn get(&self) -> Option<String> {
        self.0.lock().ok().and_then(|selected| selected.clone())
    }
}

/// Last output time of each live worker, runtime-only. Any stdout line refreshes it, so a
/// worker that streams, waits on MCP or finishes a title request is never idle, and a worker
/// whose chat was clicked away simply stops being refreshed.
#[derive(Default)]
pub struct WorkerActivity(Mutex<HashMap<String, Instant>>);

impl WorkerActivity {
    fn mark(&self, task_id: &str) {
        if let Ok(mut activity) = self.0.lock() {
            activity.insert(task_id.to_string(), Instant::now());
        }
    }
}

/// Stops the background idle reaper at app exit, before `terminate_all` runs.
#[derive(Default)]
pub struct ReaperHandle(Mutex<Option<tauri::async_runtime::JoinHandle<()>>>);

impl ReaperHandle {
    pub fn install(&self, reaper: tauri::async_runtime::JoinHandle<()>) {
        if let Ok(mut handle) = self.0.lock() {
            *handle = Some(reaper);
        }
    }

    pub fn stop(&self) {
        if let Ok(mut handle) = self.0.lock() {
            if let Some(reaper) = handle.take() {
                reaper.abort();
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
pub fn fingerprint(
    provider: &ProviderRecord,
    model_id: &str,
    resources: &Value,
) -> Result<String, String> {
    serde_json::to_string(&json!({
        "provider": provider,
        "modelId": model_id,
        "resources": resources,
    }))
    .map_err(|error| error.to_string())
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
        let data = state
            .data
            .lock()
            .map_err(|_| "Metadata lock was poisoned".to_string())?;
        fingerprint(provider, &task.model_id, &resource_paths(&data.packages))?
    };
    if let Some(existing) = worker_state.get(&task.id)? {
        if existing.fingerprint == wanted_fingerprint {
            return Ok(());
        }
        terminate_worker(app, &task.id, true).await?;
    }

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let agent_dir = app_data.join("agent").join(&task.id);
    let session_dir = app_data.join("sessions").join(&task.id);
    std::fs::create_dir_all(&agent_dir).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&session_dir).map_err(|error| error.to_string())?;

    let worker_path = worker_entry_path(app)?;
    let node_path = node_executable_path()?;
    let mut command = Command::new(node_path);
    // The user's login-shell environment first, so the agent's tools and stdio MCP servers find
    // Homebrew, nvm and friends even when the app was opened from Finder. Provider keys are
    // stripped from it below.
    crate::shell_env::apply(&mut command).await;
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
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start the bundled Pi worker: {error}"))?;
    let pid = child
        .id()
        .ok_or_else(|| "The Pi worker did not return a process id".to_string())?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "The Pi worker has no stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "The Pi worker has no stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "The Pi worker has no stderr".to_string())?;
    let pending: Pending = Arc::default();
    worker_state.insert(
        task.id.clone(),
        WorkerProcess {
            pid,
            fingerprint: wanted_fingerprint,
            stdin: Arc::new(AsyncMutex::new(stdin)),
            pending: pending.clone(),
        },
    )?;
    app.state::<WorkerActivity>().mark(&task.id);
    app.state::<SelectedTask>().set(&task.id);

    let task_id = task.id.clone();
    let subscription_worker = provider.kind == ProviderKind::Subscription;
    let app_for_process = app.clone();
    tauri::async_runtime::spawn(async move {
        let stdout_app = app_for_process.clone();
        let stdout_task_id = task_id.clone();
        let stdout_reader = tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                handle_worker_line(&stdout_app, &stdout_task_id, pid, &line, &pending);
            }
            // Dropping the senders tells every waiting request the worker is gone.
            if let Ok(mut pending) = pending.lock() {
                pending.clear();
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
                if tail.len() > 4_000 {
                    *tail = tail[tail.len() - 4_000..].to_string();
                }
            }
        });
        let _status = child.wait().await;
        let _ = stdout_reader.await;
        let _ = stderr_reader.await;
        let stderr_message = stderr_tail.lock().await.trim().to_string();
        // Intentional shutdown paths remove the process from the registry before
        // signaling it. Any process that exits while still registered is a crash,
        // including a nominal exit code caused by an external SIGTERM handler.
        let unexpected = app_for_process
            .state::<WorkerState>()
            .remove_if_pid(&task_id, pid)
            .unwrap_or(true);
        // Whatever this worker asked computer use to do can no longer be answered — unless a
        // replacement worker already owns the chat, whose requests must not be touched.
        let replaced = app_for_process.state::<WorkerState>().get(&task_id).ok().flatten().is_some();
        if !replaced {
            crate::computer_use::on_worker_stopped(&app_for_process, &task_id);
        }
        if unexpected {
            let message = if subscription_worker {
                "The Pi worker stopped unexpectedly while using a subscription.".to_string()
            } else if stderr_message.is_empty() {
                "The Pi worker stopped unexpectedly.".to_string()
            } else {
                format!(
                    "The Pi worker stopped unexpectedly: {}",
                    redact_and_limit(&stderr_message)
                )
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
            let _ = app_for_process.emit(
                "worker-event",
                json!({
                    "type": "worker_error", "taskId": task_id, "message": message
                }),
            );
            crate::menu_bar::worker_stopped(&app_for_process, &task_id, true);
        }
    });

    let (disabled_tools, resources, prompts) = {
        let state = app.state::<MetadataState>();
        let data = state
            .data
            .lock()
            .map_err(|_| "Metadata lock was poisoned".to_string())?;
        (
            data.tool_config.disabled.clone(),
            resource_paths(&data.packages),
            data.prompts.clone(),
        )
    };

    let auth_path = if provider.kind == ProviderKind::Subscription {
        Some(subscriptions::auth_path(app, &provider.id)?)
    } else {
        None
    };
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
        // Custom built-in prompts (Settings → Prompts); `{}` when every prompt is at its default.
        // Deliberately not part of the fingerprint: changes reach workers live via set_prompts.
        "prompts": prompts,
        // Enabled MCP servers with their header/env values. Live too, via set_mcp.
        "mcp": mcp_payload(app)?,
        // The user's own skill folders (Settings › Skills). Live too, via set_skills.
        "skills": skills_payload(app)?,
        // The user's own commands folder and switched-off keys (Settings › Commands). Live too,
        // via set_commands — deliberately not in the fingerprint, so a toggle never respawns.
        "commands": commands_payload(app)?,
        // This project's memory directory (Settings › Memory) and the effective on/off. Live
        // too, via set_memory; like skills, never in the fingerprint. A memory folder that
        // cannot be created degrades to off rather than blocking the chat from starting.
        "memory": crate::memory::payload(app, task).unwrap_or(Value::Null),
        // Computer use. Live too, via set_computer_use; the host enforces it per request anyway.
        "computerUse": computer_use_payload(app),
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
pub async fn request(
    app: &AppHandle,
    task_id: &str,
    value: Value,
    timeout: std::time::Duration,
) -> Result<Value, String> {
    let worker = app
        .state::<WorkerState>()
        .get(task_id)?
        .ok_or_else(|| "This task's Pi worker is not running".to_string())?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "A worker request needs an id".to_string())?
        .to_string();
    let (sender, receiver) = oneshot::channel();
    worker
        .pending
        .lock()
        .map_err(|_| "Worker lock was poisoned".to_string())?
        .insert(id.clone(), sender);
    if let Err(error) = write_line(&worker, &value).await {
        if let Ok(mut pending) = worker.pending.lock() {
            pending.remove(&id);
        }
        return Err(error);
    }
    match tokio::time::timeout(timeout, receiver).await {
        Err(_) => {
            if let Ok(mut pending) = worker.pending.lock() {
                pending.remove(&id);
            }
            Err("Pi did not answer in time.".into())
        }
        Ok(Err(_)) => Err("The Pi worker stopped before it answered.".into()),
        Ok(Ok(response)) => {
            if response.get("success").and_then(Value::as_bool) == Some(true) {
                Ok(response.get("result").cloned().unwrap_or(Value::Null))
            } else {
                Err(response
                    .get("error")
                    .and_then(Value::as_str)
                    .map(redact_and_limit)
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
    let models: Vec<Value> = provider
        .models
        .iter()
        .filter_map(|model| {
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
        })
        .collect();
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
        let data = state
            .data
            .lock()
            .map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.subagents.clone(), data.providers.clone())
    };
    Ok(crate::subagents::runtime_payload(
        &config,
        &providers,
        |provider| {
            if provider.kind == ProviderKind::Subscription {
                if !subscriptions::has_credential(app, &provider.id) {
                    return None;
                }
                let path = subscriptions::auth_path(app, &provider.id).ok()?;
                Some(crate::subagents::ProviderCredential {
                    api_key: None,
                    auth_path: Some(path.to_string_lossy().into_owned()),
                })
            } else {
                let key = state.secrets.get(&provider.id).ok()?;
                Some(crate::subagents::ProviderCredential {
                    api_key: Some(key),
                    auth_path: None,
                })
            }
        },
    ))
}

/// Push the current sub-agent settings to every running worker. Applied live on the worker's
/// next turn — never a restart, so a running chat is never interrupted by a settings change.
pub async fn broadcast_subagents(app: &AppHandle) -> Result<(), String> {
    let payload = subagent_payload(app)?;
    broadcast(app, &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_subagents", "subagents": payload })).await
}

/// The `computerUse` value for `init` and `set_computer_use`: on only while the user switched it
/// on and this Mac supports it.
pub fn computer_use_payload(app: &AppHandle) -> Value {
    let enabled = app.state::<MetadataState>().data.lock().map(|data| data.computer_use.enabled).unwrap_or(false);
    json!({ "enabled": enabled && crate::computer_use::supported() })
}

/// Push the computer-use setting to every running worker. Applied between runs; nothing restarts.
pub async fn broadcast_computer_use(app: &AppHandle) -> Result<(), String> {
    let enabled = computer_use_payload(app)["enabled"].as_bool().unwrap_or(false);
    broadcast(app, &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_computer_use", "enabled": enabled })).await
}

/// The `mcp` value for `init` and `set_mcp`: every enabled MCP server with its header and
/// environment values from `secrets.json`. Like `init`'s own key, they only travel over stdin.
pub fn mcp_payload(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<MetadataState>();
    let servers = state
        .data
        .lock()
        .map_err(|_| "Metadata lock was poisoned".to_string())?
        .mcp
        .servers
        .clone();
    Ok(crate::mcp::runtime_payload(&servers, |id| {
        crate::mcp::load_secrets(&state.secrets, id)
    }))
}

/// Push the MCP servers to every running worker. Applied between runs; nothing restarts.
pub async fn broadcast_mcp(app: &AppHandle) -> Result<(), String> {
    let payload = mcp_payload(app)?;
    broadcast(
        app,
        &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_mcp", "servers": payload }),
    )
    .await
}

/// The `skills` value for `init` and `set_skills`: `~/.agents/skills` and the other folders the
/// user switched on, as absolute paths, plus the skills switched off.
pub fn skills_payload(app: &AppHandle) -> Result<Value, String> {
    let home = crate::skills::home_dir(app)?;
    let state = app.state::<MetadataState>();
    let config = state
        .data
        .lock()
        .map_err(|_| "Metadata lock was poisoned".to_string())?
        .skills
        .clone();
    Ok(crate::skills::payload(&config, &home))
}

/// Push the skill folders to every running worker. Applied on the next turn; nothing restarts.
pub async fn broadcast_skills(app: &AppHandle) -> Result<(), String> {
    let payload = skills_payload(app)?;
    broadcast(
        app,
        &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_skills", "skills": payload }),
    )
    .await
}

/// The `commands` value for `init` and `set_commands`: the user's commands folder and the keys
/// switched off in Settings › Commands.
pub fn commands_payload(app: &AppHandle) -> Result<Value, String> {
    let dir = crate::slash_commands::dir(app)?;
    let state = app.state::<MetadataState>();
    let config = state
        .data
        .lock()
        .map_err(|_| "Metadata lock was poisoned".to_string())?
        .commands
        .clone();
    Ok(crate::slash_commands::payload(&config, &dir))
}

/// Push the command settings to every running worker. Applied on the next turn; nothing restarts.
pub async fn broadcast_commands(app: &AppHandle) -> Result<(), String> {
    let payload = commands_payload(app)?;
    broadcast(app, &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_commands", "commands": payload })).await
}

/// Push each chat's memory setting to its own worker. Per worker, not one value for all: the
/// directory is whatever project that chat belongs to. Applied on the next turn; nothing
/// restarts, and a chat without a worker picks the same value up in its next `init`.
pub async fn broadcast_memory(app: &AppHandle) -> Result<(), String> {
    for task_id in app.state::<crate::worker::WorkerState>().task_ids()? {
        let task = {
            let state = app.state::<MetadataState>();
            let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
            data.tasks.iter().find(|task| task.id == task_id).cloned()
        };
        let Some(task) = task else { continue };
        // One bad memory folder must not leave the other chats' workers stale: degrade that
        // chat to memory-off and keep going.
        let payload = crate::memory::payload(app, &task).unwrap_or(Value::Null);
        let _ = send(
            app,
            &task_id,
            &json!({ "id": uuid::Uuid::new_v4().to_string(), "type": "set_memory", "memory": payload }),
        )
        .await;
    }
    Ok(())
}

pub async fn send(app: &AppHandle, task_id: &str, value: &Value) -> Result<(), String> {
    let worker = app
        .state::<WorkerState>()
        .get(task_id)?
        .ok_or_else(|| "This task's Pi worker is not running".to_string())?;
    write_line(&worker, value).await
}

async fn write_line(worker: &WorkerProcess, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let mut stdin = worker.stdin.lock().await;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|error| format!("Could not send a command to Pi: {error}"))?;
    stdin.flush().await.map_err(|error| error.to_string())
}

pub async fn terminate_worker(
    app: &AppHandle,
    task_id: &str,
    graceful: bool,
) -> Result<(), String> {
    let Some(worker) = app.state::<WorkerState>().remove(task_id)? else {
        crate::menu_bar::worker_stopped(app, task_id, false);
        return Ok(());
    };
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
    crate::menu_bar::worker_stopped(app, task_id, false);
    Ok(())
}

/// Chooses which idle workers a sweep stops: those past `IDLE_WORKER_AFTER`, least recently
/// active first, never the newest `IDLE_WORKER_KEEP`. Pure so the policy is unit-testable
/// without processes. Activity ties break by task id for a deterministic kill order.
fn idle_reap_plan(candidates: &[(String, Instant)], now: Instant) -> Vec<String> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let stoppable = sorted.len().saturating_sub(IDLE_WORKER_KEEP);
    sorted[..stoppable]
        .iter()
        .filter(|(_, last)| now.duration_since(*last) >= IDLE_WORKER_AFTER)
        .map(|(task_id, _)| task_id.clone())
        .collect()
}

/// One idle-reaper pass. Each stop happens under the chat's task lock with the status
/// re-checked, so a run can never race a reap, and goes through `terminate_worker`, which
/// removes the worker from the registry before signalling — so the crash monitor reports
/// nothing and the chat's status and transcript are untouched.
async fn sweep_idle_workers(app: &AppHandle) {
    let selected = app.state::<SelectedTask>().get();
    let Ok(ids) = app.state::<WorkerState>().task_ids() else {
        return;
    };
    let statuses: HashMap<String, TaskStatus> = match app.state::<MetadataState>().data.lock() {
        Ok(data) => data
            .tasks
            .iter()
            .map(|task| (task.id.clone(), task.status.clone()))
            .collect(),
        Err(_) => return,
    };
    let now = Instant::now();
    let candidates: Vec<(String, Instant)> = {
        let activity_state = app.state::<WorkerActivity>();
        let Ok(mut activity) = activity_state.0.lock() else {
            return;
        };
        // Prune tasks whose worker already exited through some other path.
        activity.retain(|task_id, _| ids.contains(task_id));
        ids.iter()
            .filter(|task_id| selected.as_deref() != Some(task_id.as_str()))
            .filter(|task_id| statuses.get(task_id.as_str()) == Some(&TaskStatus::Idle))
            .filter_map(|task_id| {
                activity
                    .get(task_id)
                    .map(|last| ((*task_id).clone(), *last))
            })
            .collect()
    };
    for task_id in idle_reap_plan(&candidates, now) {
        let lock = crate::commands::task_lock(app, &task_id);
        let Ok(_guard) = tokio::time::timeout(IDLE_WORKER_LOCK_WAIT, lock.lock()).await else {
            continue;
        };
        // Re-check under the lock: a prompt queued behind us flips the status away from Idle,
        // and another stop path may already have removed the worker.
        let worker_live = app
            .state::<WorkerState>()
            .get(&task_id)
            .map(|worker| worker.is_some())
            .unwrap_or(false);
        let status_idle = app
            .state::<MetadataState>()
            .data
            .lock()
            .ok()
            .and_then(|data| {
                data.tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .map(|task| matches!(task.status, TaskStatus::Idle))
            })
            .unwrap_or(false);
        if worker_live && status_idle {
            let _ = terminate_worker(app, &task_id, true).await;
        }
    }
}

/// Runs the idle reaper until `ReaperHandle::stop`. Started once from `lib.rs` setup; the
/// first interval tick fires immediately and finds only freshly spawned workers.
pub fn start_idle_reaper(app: AppHandle) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(IDLE_WORKER_SWEEP);
        loop {
            ticker.tick().await;
            sweep_idle_workers(&app).await;
        }
    })
}

fn worker_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/index.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/index.js"))
    }
}

fn manager_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/manager.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/manager.js"))
    }
}

pub(crate) fn mcp_probe_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/mcp-probe.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/mcp-probe.js"))
    }
}

pub(crate) fn skills_scan_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/skills-scan.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/skills-scan.js"))
    }
}

/// The keyless scan Settings › Commands runs (`commands-scan.js`; see `slash_commands.rs`).
pub(crate) fn commands_scan_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/commands-scan.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/commands-scan.js"))
    }
}

fn catalog_entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/catalog.js"))
    } else {
        Ok(app
            .path()
            .resource_dir()
            .map_err(|error| error.to_string())?
            .join("resources/worker/dist/catalog.js"))
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
    for key in PROVIDER_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "Reading the bundled Pi model catalogue timed out.".to_string())?
        .map_err(|error| format!("Could not read the bundled Pi model catalogue: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not read the bundled Pi model catalogue: {}",
            redact_and_limit(&String::from_utf8_lossy(&output.stderr))
        ));
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
        app.path()
            .resource_dir()
            .ok()?
            .join("resources/npm/bin/npm-cli.js")
    };
    if !cli.exists() {
        return None;
    }
    Some(vec![
        node.to_string_lossy().into_owned(),
        cli.to_string_lossy().into_owned(),
    ])
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
    let mut stdin = process
        .stdin
        .take()
        .ok_or_else(|| "The package manager has no stdin".to_string())?;
    let stdout = process
        .stdout
        .take()
        .ok_or_else(|| "The package manager has no stdout".to_string())?;
    let stderr = process
        .stderr
        .take()
        .ok_or_else(|| "The package manager has no stderr".to_string())?;

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
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| format!("Could not reach the package manager: {error}"))?;
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
            if noise.len() < MANAGER_NOISE_LIMIT {
                noise.push_str(&line);
                noise.push('\n');
            }
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("catalog") => catalog = value.get("packages").cloned().unwrap_or(Value::Null),
            Some("progress") => {
                let _ = app.emit("package-event", value);
            }
            Some("manager_error") => {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    outcome = Some(Err(redact_and_limit(message)));
                    break;
                }
            }
            Some("response")
                if value.get("id").and_then(Value::as_str) == Some(command_id.as_str()) =>
            {
                outcome = Some(
                    if value.get("success").and_then(Value::as_bool) == Some(true) {
                        Ok(())
                    } else {
                        Err(redact_and_limit(
                            value
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("The package operation failed"),
                        ))
                    },
                );
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
        characters[characters.len().saturating_sub(600)..]
            .iter()
            .collect()
    };
    format!("{message}\n{}", redact_and_limit(&tail))
}

pub fn package_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("pi");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

pub(crate) fn node_executable_path() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(std::env::var_os("WACKCODE_NODE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("node")))
    } else {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Ok(executable
            .parent()
            .ok_or_else(|| "Could not locate the app executable directory".to_string())?
            .join("wackcode-node"))
    }
}

fn handle_worker_line(
    app: &AppHandle,
    task_id: &str,
    worker_pid: u32,
    line: &str,
    pending: &Pending,
) {
    // Any output, even an unparseable line, proves the worker is alive: refresh its idle clock.
    app.state::<WorkerActivity>().mark(task_id);
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if event_type == "usage_record" {
        if let Some(record) = value.get("record") {
            crate::usage::record(app, task_id, record.clone());
        }
        return;
    }
    if event_type == "browser_cancel" || event_type == "computer_cancel" {
        if let Some(request_id) = value.get("requestId").and_then(Value::as_str) {
            if event_type == "browser_cancel" {
                app.state::<crate::browser::BrowserManager>().cancel(request_id);
            } else {
                app.state::<crate::computer_use::ComputerUseManager>().cancel(request_id);
            }
        }
        return;
    }
    if event_type == "browser_request" || event_type == "computer_request" {
        let channel = if event_type == "browser_request" { NativeChannel::Browser } else { NativeChannel::Computer };
        spawn_native_request(app, task_id, worker_pid, channel, &value);
        return;
    }
    if event_type == "title_result" {
        let attempt_id = value.get("attemptId").and_then(Value::as_str).unwrap_or("");
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .and_then(normalize_auto_title);
        let accepted = app
            .state::<MetadataState>()
            .mutate(|data| {
                let Some(task) = data.tasks.iter_mut().find(|task| task.id == task_id) else {
                    return Ok(false);
                };
                if task.auto_title_attempt_id.as_deref() != Some(attempt_id) {
                    return Ok(false);
                }
                task.auto_title_attempt_id = None;
                if let Some(title) = &title {
                    task.name = title.clone();
                }
                task.updated_at = chrono::Utc::now().to_rfc3339();
                Ok(true)
            })
            .unwrap_or(false);
        if !accepted {
            return;
        }
        if let Some(title) = title {
            let _ = app.emit(
                "worker-event",
                json!({ "type": "title_changed", "taskId": task_id, "name": title }),
            );
        } else {
            let _ = app.emit("worker-event", json!({ "type": "extension_notice", "taskId": task_id,
                "message": "Automatic title could not be generated. The current title was kept.", "level": "warning" }));
        }
        let _ = crate::menu_bar::refresh(app);
        return;
    }
    if event_type == "response" {
        let waiting = value.get("id").and_then(Value::as_str).and_then(|id| {
            pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(id))
        });
        // A caller is waiting for this one and reports it itself.
        if let Some(sender) = waiting {
            let _ = sender.send(value);
            return;
        }
    }
    if event_type == "worker_error" {
        if let Some(message) = value
            .get("message")
            .and_then(Value::as_str)
            .map(redact_and_limit)
        {
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
                if let Ok(catalog) = serde_json::from_value::<Vec<ToolCatalogEntry>>(tools.clone())
                {
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
                    if event_type == "run_state"
                        && value.get("state").and_then(Value::as_str) == Some("running")
                    {
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
        let mode = value
            .get("mode")
            .cloned()
            .and_then(|mode| serde_json::from_value::<TaskMode>(mode).ok());
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
    if should_save {
        let _ = app.state::<MetadataState>().save();
    }
    if event_type == "run_state" {
        if let Some(state) = value.get("state").and_then(Value::as_str) {
            crate::computer_use::on_run_state(app, task_id, state);
        }
    }
    crate::menu_bar::handle_worker_event(app, task_id, &value);
    let _ = app.emit("worker-event", value);
}

#[derive(Clone, Copy)]
enum NativeChannel {
    Browser,
    Computer,
}

/// Runs a worker's request to a native subsystem (the browser preview or computer use) and
/// answers with `<channel>_response`. Both bypass the worker's prompt queue in both directions.
fn spawn_native_request(app: &AppHandle, task_id: &str, worker_pid: u32, channel: NativeChannel, value: &Value) {
    let Some(request_id) = value.get("requestId").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    let request = value.get("request").cloned().unwrap_or(Value::Null);
    let request_app = app.clone();
    let request_task = task_id.to_string();
    tauri::async_runtime::spawn(async move {
        let (result, response_type) = match channel {
            NativeChannel::Browser => (
                crate::browser::execute_agent_request(request_app.clone(), request_task.clone(), request_id.clone(), request).await,
                "browser_response",
            ),
            NativeChannel::Computer => (
                crate::computer_use::execute_agent_request(request_app.clone(), request_task.clone(), request_id.clone(), request).await,
                "computer_response",
            ),
        };
        // A restarted worker owns the same chat id but a different generation. Never let it
        // receive a response to an operation issued by the worker that was replaced.
        let current = request_app.state::<WorkerState>().get(&request_task).ok().flatten();
        if current.as_ref().map(|worker| worker.pid) != Some(worker_pid) {
            return;
        }
        let response = match result {
            Ok(result) => {
                json!({ "id": uuid::Uuid::new_v4().to_string(), "type": response_type, "requestId": request_id, "success": true, "result": result })
            }
            Err(error) => {
                json!({ "id": uuid::Uuid::new_v4().to_string(), "type": response_type, "requestId": request_id, "success": false, "error": redact_and_limit(&error) })
            }
        };
        if let Some(worker) = current {
            let _ = write_line(&worker, &response).await;
        }
    });
}

fn normalize_auto_title(raw: &str) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact
        .trim_matches(|c: char| {
            c == '"' || c == '\'' || c == '“' || c == '”' || c == '‘' || c == '’'
        })
        .trim();
    let title: String = trimmed.chars().take(80).collect();
    let one_word_ascii = title.is_ascii() && title.split_whitespace().count() < 3;
    let has_emoji = title
        .chars()
        .any(|c| matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF));
    if title.is_empty() || one_word_ascii || has_emoji {
        None
    } else {
        Some(title)
    }
}

#[cfg(test)]
mod auto_title_tests {
    use super::normalize_auto_title;

    #[test]
    fn trims_model_wrapping_and_rejects_blank_output() {
        assert_eq!(
            normalize_auto_title("  “  Short   chat title  ” "),
            Some("Short chat title".into())
        );
        assert_eq!(normalize_auto_title(" \n '  '  "), None);
        assert_eq!(normalize_auto_title("OK."), None);
        assert_eq!(normalize_auto_title("Emoji title here 😀"), None);
        assert_eq!(
            normalize_auto_title(&format!("Long title {}", "x".repeat(90)))
                .unwrap()
                .chars()
                .count(),
            80
        );
    }
}

pub(crate) fn redact_and_limit(message: &str) -> String {
    let mut safe = message.to_string();
    for marker in ["sk-", "Bearer "] {
        while let Some(start) = safe.find(marker) {
            let end = safe[start..]
                .find(char::is_whitespace)
                .map(|index| start + index)
                .unwrap_or(safe.len());
            safe.replace_range(start..end, "[credential redacted]");
        }
    }
    for marker in ["access_token=", "refresh_token=", "device_code=", "code="] {
        while let Some(start) = safe.to_ascii_lowercase().find(marker) {
            let end = safe[start..]
                .find(|character: char| {
                    character.is_whitespace() || character == '&' || character == '#'
                })
                .map(|index| start + index)
                .unwrap_or(safe.len());
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
            extensions: resources
                .iter()
                .map(|(path, enabled)| PackageResourceRecord {
                    path: (*path).into(),
                    name: (*path).into(),
                    enabled: *enabled,
                })
                .collect(),
            skills: Vec::new(),
            prompts: Vec::new(),
            themes: Vec::new(),
            errors: Vec::new(),
            trusted_at: if trusted {
                "2026-01-01T00:00:00Z".into()
            } else {
                String::new()
            },
            installed_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn provider() -> ProviderRecord {
        ProviderRecord {
            id: "p".into(),
            name: "P".into(),
            kind: ProviderKind::Custom,
            base_url: "https://example.test/v1".into(),
            api_format: "openai-completions".into(),
            models: Vec::new(),
            created_at: "now".into(),
            updated_at: "now".into(),
            has_api_key: true,
            connected: true,
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
        let packages = vec![package(
            "npm:x",
            true,
            &[("/pkg/x/on.ts", true), ("/pkg/x/off.ts", false)],
        )];
        let extensions = resource_paths(&packages)["extensions"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(extensions, vec!["/pkg/x/on.ts"]);
    }

    #[test]
    fn an_untrusted_package_contributes_nothing_even_with_every_resource_enabled() {
        let packages = vec![package(
            "npm:appeared-somehow",
            false,
            &[("/pkg/a.ts", true), ("/pkg/b.ts", true)],
        )];
        let paths = resource_paths(&packages);
        for kind in ["extensions", "skills", "prompts", "themes"] {
            assert!(
                paths[kind].as_array().unwrap().is_empty(),
                "{kind} leaked from an untrusted package"
            );
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
    fn skill_folders_reach_workers_live_and_never_restart_them() {
        // The fingerprint has no room for the skill folders: switching one must not respawn a
        // chat mid-run. They travel in `init` and `set_skills` instead.
        let resources = resource_paths(&[]);
        let fingerprint = fingerprint(&provider(), "m", &resources).unwrap();
        let mut config = crate::models::SkillsConfig::default();
        let before = crate::skills::payload(&config, std::path::Path::new("/Users/test"));
        config.folders.push(crate::models::SkillFolderRecord {
            id: "claude".into(),
            path: None,
            enabled: true,
        });
        let after = crate::skills::payload(&config, std::path::Path::new("/Users/test"));
        assert_ne!(before, after);
        assert_eq!(
            fingerprint,
            super::fingerprint(&provider(), "m", &resources).unwrap()
        );
    }

    #[test]
    fn command_settings_reach_workers_live_and_never_restart_them() {
        // Like skills: the fingerprint has no room for the commands payload, so toggling a
        // command never respawns a chat mid-run. It travels in `init` and `set_commands`.
        let resources = resource_paths(&[]);
        let fingerprint = fingerprint(&provider(), "m", &resources).unwrap();
        let dir = std::path::Path::new("/app/data/commands");
        let mut config = crate::models::CommandsConfig::default();
        let before = crate::slash_commands::payload(&config, dir);
        config.disabled.push("app:copy".into());
        let after = crate::slash_commands::payload(&config, dir);
        assert_ne!(before, after);
        assert_eq!(
            fingerprint,
            super::fingerprint(&provider(), "m", &resources).unwrap()
        );
    }

    #[test]
    fn manager_noise_is_bounded_and_redacted() {
        let message = with_context(
            "Install failed".into(),
            "npm warn deprecated\n",
            "sk-abcdefghijklmnop leaked",
        );
        assert!(message.starts_with("Install failed\n"));
        assert!(message.contains("[credential redacted]"));
        assert!(!message.contains("sk-abcdefghijklmnop"));
    }

    #[test]
    fn idle_reap_plan_stops_the_oldest_workers_beyond_four() {
        let base = Instant::now();
        let now = base + Duration::from_secs(3_600);
        let candidates: Vec<(String, Instant)> = (0..10)
            .map(|i| (format!("task-{i}"), base + Duration::from_secs(i * 60)))
            .collect();
        assert_eq!(
            idle_reap_plan(&candidates, now),
            vec!["task-0", "task-1", "task-2", "task-3", "task-4", "task-5"]
        );
    }

    #[test]
    fn idle_reap_plan_never_takes_recent_workers() {
        let base = Instant::now();
        let now = base + Duration::from_secs(3_600);
        let candidates = vec![
            ("ancient".to_string(), base),                        // 60 min idle
            ("old".to_string(), base + Duration::from_secs(600)), // 50 min idle
            ("fresh-1".to_string(), base + Duration::from_secs(3_000)), // 10 min idle
            ("fresh-2".to_string(), base + Duration::from_secs(3_100)),
            ("fresh-3".to_string(), base + Duration::from_secs(3_200)),
        ];
        assert_eq!(idle_reap_plan(&candidates, now), vec!["ancient"]);
    }

    #[test]
    fn idle_reap_plan_keeps_a_warm_pool_of_four() {
        let base = Instant::now();
        let now = base + Duration::from_secs(3_600);
        let candidates: Vec<(String, Instant)> = (0..3)
            .map(|i| (format!("task-{i}"), base + Duration::from_secs(i)))
            .collect();
        assert!(idle_reap_plan(&candidates, now).is_empty());
    }

    #[test]
    fn idle_reap_plan_breaks_activity_ties_by_task_id() {
        let base = Instant::now();
        let now = base + Duration::from_secs(3_600);
        let candidates: Vec<(String, Instant)> = ["b", "a", "c", "d", "e", "f", "g"]
            .iter()
            .map(|id| (id.to_string(), base))
            .collect();
        assert_eq!(idle_reap_plan(&candidates, now), vec!["a", "b", "c"]);
    }
}
