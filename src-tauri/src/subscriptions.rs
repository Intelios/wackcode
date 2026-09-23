use crate::{models::{ModelRecord, ProviderKind, ProviderRecord, TaskStatus}, storage::MetadataState, worker};
use chrono::Utc;
use nix::{sys::signal::{killpg, Signal}, unistd::Pid};
use serde::Serialize;
use serde_json::{json, Value};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Stdio, sync::{Arc, Mutex}};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, process::{ChildStdin, Command}, sync::Mutex as AsyncMutex};
use uuid::Uuid;

const PROVIDERS: &[(&str, &str, &str)] = &[
    ("openai-codex", "OpenAI Codex", "ChatGPT Plus or Pro subscription."),
    ("github-copilot", "GitHub Copilot", "A Copilot subscription is required. Enterprise accounts may need a GitHub domain."),
    ("anthropic", "Anthropic", "Claude Pro or Max sign-in may use separately billed usage credits in third-party apps. Review Anthropic's current terms."),
    ("xai", "xAI", "Grok or X subscription access; Pi will offer the supported login choices."),
    ("meta", "Meta", "Meta sign-in for Muse subscription access."),
    ("kimi-coding", "Kimi For Coding", "Kimi For Coding subscription access."),
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionProviderInfo { id: &'static str, name: &'static str, guidance: &'static str }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedLogin { login_id: String, provider: ProviderRecord }

#[derive(Clone)]
struct LiveLogin { id: String, provider_id: String, pid: u32, stdin: Arc<AsyncMutex<ChildStdin>> }

#[derive(Default)]
pub struct SubscriptionState { current: Mutex<Option<LiveLogin>> }

impl SubscriptionState {
    fn get(&self, id: &str) -> Result<LiveLogin, String> {
        self.current.lock().map_err(|_| "Subscription login lock was poisoned".to_string())?
            .as_ref().filter(|login| login.id == id).cloned()
            .ok_or_else(|| "This sign-in is no longer active".to_string())
    }

    fn remove(&self, id: &str) -> Result<Option<LiveLogin>, String> {
        let mut current = self.current.lock().map_err(|_| "Subscription login lock was poisoned".to_string())?;
        Ok(if current.as_ref().is_some_and(|login| login.id == id) { current.take() } else { None })
    }

    pub fn terminate_all(&self) {
        if let Ok(mut current) = self.current.lock() {
            if let Some(login) = current.take() { let _ = killpg(Pid::from_raw(login.pid as i32), Signal::SIGTERM); }
        }
    }
}

fn allowed(id: &str) -> bool { PROVIDERS.iter().any(|(candidate, _, _)| *candidate == id) }

pub fn auth_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    if !allowed(id) { return Err("This subscription provider is unavailable".into()); }
    Ok(app.path().app_data_dir().map_err(|error| error.to_string())?.join("subscriptions").join(id).join("auth.json"))
}

pub fn has_credential(app: &AppHandle, id: &str) -> bool {
    let Ok(path) = auth_path(app, id) else { return false; };
    has_credential_at(&path, id)
}

fn has_credential_at(path: &std::path::Path, id: &str) -> bool {
    if ![path.parent(), path.parent().and_then(std::path::Path::parent)].into_iter().flatten()
        .all(|directory| fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.file_type().is_dir())) { return false; }
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) { return false; }
    if fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).is_err() { return false; }
    let Ok(content) = fs::read(&path) else { return false; };
    let Ok(data) = serde_json::from_slice::<Value>(&content) else { return false; };
    data.get(id).and_then(|entry| entry.get("type")).and_then(Value::as_str) == Some("oauth")
}

pub fn remove_credential(app: &AppHandle, id: &str) -> Result<(), String> {
    let path = auth_path(app, id)?;
    if fs::symlink_metadata(&path).is_ok() { fs::remove_file(path).map_err(|error| format!("Could not remove subscription sign-in: {error}"))?; }
    Ok(())
}

#[tauri::command]
pub fn list_subscription_providers() -> Vec<SubscriptionProviderInfo> {
    PROVIDERS.iter().map(|(id, name, guidance)| SubscriptionProviderInfo { id, name, guidance }).collect()
}

fn entry_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker/dist/subscription-auth.js"))
    } else {
        Ok(app.path().resource_dir().map_err(|error| error.to_string())?.join("resources/worker/dist/subscription-auth.js"))
    }
}

#[tauri::command]
pub async fn start_subscription_login(app: AppHandle, state: State<'_, MetadataState>, provider_id: String) -> Result<StartedLogin, String> {
    if !allowed(&provider_id) { return Err("This subscription provider is unavailable".into()); }
    {
        let active = app.state::<SubscriptionState>();
        if active.current.lock().map_err(|_| "Subscription login lock was poisoned".to_string())?.is_some() {
            return Err("Finish or cancel the current sign-in first".into());
        }
    }
    let (name, _) = PROVIDERS.iter().find(|(id, _, _)| *id == provider_id).map(|(_, name, guidance)| (*name, *guidance)).unwrap();
    {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        if data.tasks.iter().any(|task| task.provider_id == provider_id && matches!(task.status, TaskStatus::Running | TaskStatus::Stopping)) {
            return Err("Wait for tasks using this subscription to finish before reconnecting".into());
        }
        if data.providers.iter().any(|provider| provider.id == provider_id && provider.kind != ProviderKind::Subscription) {
            return Err("This provider ID is already used by another connection".into());
        }
    }
    let task_ids = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.tasks.iter().filter(|task| task.provider_id == provider_id).map(|task| task.id.clone()).collect::<Vec<_>>()
    };
    for task_id in task_ids {
        worker::terminate_worker(&app, &task_id, true).await?;
    }
    let now = Utc::now().to_rfc3339();
    let record = state.mutate(|data| {
        if !data.providers.iter().any(|provider| provider.id == provider_id) {
            data.providers.push(ProviderRecord {
                id: provider_id.clone(), name: name.to_string(), kind: ProviderKind::Subscription,
                base_url: String::new(), api_format: String::new(), models: Vec::new(),
                created_at: now.clone(), updated_at: now.clone(), has_api_key: false, connected: false,
            });
        }
        data.providers.iter().find(|provider| provider.id == provider_id).cloned()
            .ok_or_else(|| "Subscription connection was not saved".to_string())
    })?;
    let auth_path = auth_path(&app, &provider_id)?;
    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("Could not create subscription credential directory: {error}"))?;
        for directory in [parent.parent(), Some(parent)].into_iter().flatten() {
            if !fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.file_type().is_dir()) {
                return Err("Subscription credential directory must be a private directory".into());
            }
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("Could not secure subscription credential directory: {error}"))?;
        }
    }
    if fs::symlink_metadata(&auth_path).is_ok_and(|metadata| !metadata.file_type().is_file()) {
        return Err("Subscription credential path must be a regular file".into());
    }
    let mut command = Command::new(worker::node_executable_path()?);
    command.arg(entry_path(&app)?)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true)
        .env("PI_TELEMETRY", "0").env("PI_SKIP_VERSION_CHECK", "1").env("PI_OFFLINE", "1")
        .env_remove("PI_OAUTH_CALLBACK_HOST");
    worker::strip_provider_env(&mut command);
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().map_err(|error| format!("Could not start subscription sign-in: {error}"))?;
    let pid = child.id().ok_or_else(|| "Subscription sign-in has no process id".to_string())?;
    let stdin = child.stdin.take().ok_or_else(|| "Subscription sign-in has no input pipe".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "Subscription sign-in has no output pipe".to_string())?;
    let id = Uuid::new_v4().to_string();
    let login = LiveLogin { id: id.clone(), provider_id: provider_id.clone(), pid, stdin: Arc::new(AsyncMutex::new(stdin)) };
    {
        let login_state = app.state::<SubscriptionState>();
        let mut current = login_state.current.lock().map_err(|_| "Subscription login lock was poisoned".to_string())?;
        if current.is_some() { let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM); return Err("Finish or cancel the current sign-in first".into()); }
        *current = Some(login.clone());
    }
    let init = json!({ "type": "start", "providerId": provider_id, "authPath": auth_path });
    if let Err(error) = send(&login, &init).await {
        let _ = app.state::<SubscriptionState>().remove(&id);
        let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGTERM);
        return Err(error);
    }
    let app_for_reader = app.clone();
    let id_for_reader = id.clone();
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut terminal = false;
        while let Ok(Some(line)) = lines.next_line().await {
            if line.len() > 2_000_000 { break; }
            if handle_line(&app_for_reader, &id_for_reader, &line) { terminal = true; break; }
        }
        if tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await.is_err() {
            let _ = killpg(Pid::from_raw(login.pid as i32), Signal::SIGTERM);
            let _ = child.wait().await;
        }
        if app_for_reader.state::<SubscriptionState>().remove(&id_for_reader).ok().flatten().is_some() && !terminal {
            emit(&app_for_reader, &id_for_reader, &login.provider_id, json!({ "type": "error", "message": "Subscription sign-in stopped before it completed." }));
        }
    });
    Ok(StartedLogin { login_id: id, provider: record })
}

async fn send(login: &LiveLogin, value: &Value) -> Result<(), String> {
    let mut stdin = login.stdin.lock().await;
    let mut line = serde_json::to_vec(value).map_err(|_| "Could not encode sign-in response".to_string())?;
    line.push(b'\n');
    stdin.write_all(&line).await.map_err(|_| "Could not reach subscription sign-in".to_string())?;
    stdin.flush().await.map_err(|_| "Could not reach subscription sign-in".to_string())
}

fn emit(app: &AppHandle, id: &str, provider_id: &str, value: Value) {
    let mut output = json!({ "loginId": id, "providerId": provider_id });
    if let (Some(target), Some(source)) = (output.as_object_mut(), value.as_object()) {
        target.extend(source.clone());
    }
    let _ = app.emit("subscription-login-event", output);
}

fn handle_line(app: &AppHandle, id: &str, line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else { return false; };
    let provider_id = match app.state::<SubscriptionState>().get(id) { Ok(login) => login.provider_id, Err(_) => return true };
    match value.get("type").and_then(Value::as_str) {
        Some("complete") => {
            let models: Vec<ModelRecord> = match serde_json::from_value::<Vec<ModelRecord>>(value.get("models").cloned().unwrap_or(Value::Null)) {
                Ok(models) if models.len() <= 1_000 => models,
                _ => { emit(app, id, &provider_id, json!({ "type": "error", "message": "Pi returned an invalid model list." })); return true; }
            };
            let result = app.state::<MetadataState>().mutate(|data| {
                let provider = data.providers.iter_mut().find(|provider| provider.id == provider_id)
                    .ok_or_else(|| "Subscription connection was removed".to_string())?;
                provider.models = models;
                provider.connected = true;
                provider.updated_at = Utc::now().to_rfc3339();
                Ok(provider.clone())
            });
            match result {
                Ok(provider) => {
                    // Sub-agents on this subscription can run again, with its fresh model list.
                    let broadcast_app = app.clone();
                    tauri::async_runtime::spawn(async move { let _ = worker::broadcast_subagents(&broadcast_app).await; });
                    emit(app, id, &provider_id, json!({ "type": "complete", "provider": provider }))
                }
                Err(_) => emit(app, id, &provider_id, json!({ "type": "error", "message": "Sign-in succeeded, but WackCode could not save the connection." })),
            }
            true
        }
        Some("error") | Some("cancelled") => {
            // The helper reports fixed error copy. Do not allow provider output to become a
            // token-bearing event if an unexpected error ever reaches this bridge.
            if value.get("type").and_then(Value::as_str) == Some("error") {
                emit(app, id, &provider_id, json!({ "type": "error", "message": "Subscription sign-in failed. Check the browser or device code, then try again." }));
            } else {
                emit(app, id, &provider_id, json!({ "type": "cancelled" }));
            }
            true
        }
        Some("prompt") | Some("auth_url") | Some("device_code") | Some("info") | Some("progress") => {
            emit(app, id, &provider_id, value);
            false
        }
        _ => false,
    }
}

#[tauri::command]
pub async fn respond_subscription_login(app: AppHandle, login_id: String, prompt_id: String, value: Option<String>, cancelled: bool) -> Result<(), String> {
    let login = app.state::<SubscriptionState>().get(&login_id)?;
    send(&login, &json!({ "type": "response", "promptId": prompt_id, "value": value, "cancelled": cancelled })).await
}

#[tauri::command]
pub fn cancel_subscription_login(app: AppHandle, login_id: String) -> Result<(), String> {
    let Some(login) = app.state::<SubscriptionState>().remove(&login_id)? else { return Ok(()); };
    let _ = killpg(Pid::from_raw(login.pid as i32), Signal::SIGTERM);
    emit(&app, &login_id, &login.provider_id, json!({ "type": "cancelled" }));
    Ok(())
}

#[tauri::command]
pub async fn sign_out_subscription(app: AppHandle, state: State<'_, MetadataState>, provider_id: String) -> Result<ProviderRecord, String> {
    if !allowed(&provider_id) { return Err("This subscription provider is unavailable".into()); }
    if app.state::<SubscriptionState>().current.lock().map_err(|_| "Subscription login lock was poisoned".to_string())?
        .as_ref().is_some_and(|login| login.provider_id == provider_id) {
        return Err("Finish or cancel this sign-in before signing out".into());
    }
    let task_ids = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let provider = data.providers.iter().find(|provider| provider.id == provider_id && provider.kind == ProviderKind::Subscription)
            .ok_or_else(|| "Subscription connection not found".to_string())?;
        let _ = provider;
        if data.tasks.iter().any(|task| task.provider_id == provider_id && matches!(task.status, TaskStatus::Running | TaskStatus::Stopping)) {
            return Err("Wait for tasks using this subscription to finish before signing out".into());
        }
        data.tasks.iter().filter(|task| task.provider_id == provider_id).map(|task| task.id.clone()).collect::<Vec<_>>()
    };
    for task_id in task_ids { worker::terminate_worker(&app, &task_id, true).await?; }
    remove_credential(&app, &provider_id)?;
    let record = state.mutate(|data| {
        let provider = data.providers.iter_mut().find(|provider| provider.id == provider_id).ok_or_else(|| "Subscription connection not found".to_string())?;
        provider.connected = false;
        provider.updated_at = Utc::now().to_rfc3339();
        Ok(provider.clone())
    })?;
    // Withdraw the auth file from sub-agents in other chats; theirs now reports signed out.
    worker::broadcast_subagents(&app).await?;
    Ok(record)
}

#[tauri::command]
pub fn open_subscription_auth_url(url: String) -> Result<(), String> {
    validate_auth_url(&url)?;
    let status = std::process::Command::new("open").arg(url).status().map_err(|error| format!("Could not open the sign-in link: {error}"))?;
    if status.success() { Ok(()) } else { Err("Could not open the sign-in link in your browser".into()) }
}

fn validate_auth_url(url: &str) -> Result<(), String> {
    if url.len() > 8_192 { return Err("The sign-in link is too long".into()); }
    let parsed = reqwest::Url::parse(&url).map_err(|_| "The sign-in link is invalid".to_string())?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Only secure HTTPS sign-in links can be opened".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_are_fixed() {
        assert_eq!(PROVIDERS.len(), 6);
        assert!(allowed("openai-codex"));
        assert!(!allowed("openrouter"));
        assert!(!allowed("../auth.json"));
    }

    #[test]
    fn only_private_oauth_credentials_count_as_connected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(&path, r#"{"openai-codex":{"type":"api_key","key":"secret"}}"#).unwrap();
        assert!(!has_credential_at(&path, "openai-codex"));
        fs::write(&path, r#"{"openai-codex":{"type":"oauth","access":"secret","refresh":"secret","expires":9999999999999}}"#).unwrap();
        assert!(has_credential_at(&path, "openai-codex"));
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn credential_symlink_is_not_followed() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("pi-auth.json");
        let link = directory.path().join("auth.json");
        fs::write(&target, r#"{"openai-codex":{"type":"oauth"}}"#).unwrap();
        let original_mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        symlink(&target, &link).unwrap();
        assert!(!has_credential_at(&link, "openai-codex"));
        assert_eq!(fs::metadata(target).unwrap().permissions().mode() & 0o777, original_mode);
    }

    #[test]
    fn browser_opener_rejects_untrusted_urls() {
        assert!(validate_auth_url("https://github.com/login/device").is_ok());
        for url in ["http://localhost:1455/auth/callback", "file:///tmp/auth", "https://user:secret@example.com", "javascript:alert(1)"] {
            assert!(validate_auth_url(url).is_err());
        }
    }
}
