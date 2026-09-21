use crate::{
    git,
    models::{
        BootstrapPayload, CreateTaskInput, GitChanges, ModelRecord, ProjectRecord, PromptInput,
        ProviderRecord, SaveProviderInput, TaskRecord, TaskStatus,
    },
    storage::MetadataState,
    worker::{self},
};
use chrono::Utc;
use keyring::Entry;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashSet, path::{Path, PathBuf}, process::Command};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "com.wackcode.desktop.providers";
const THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

#[tauri::command]
pub fn bootstrap(state: State<'_, MetadataState>) -> Result<BootstrapPayload, String> {
    let mut data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?.clone();
    for provider in &mut data.providers {
        provider.has_api_key = read_api_key(&provider.id).is_ok();
    }
    Ok(BootstrapPayload {
        data,
        app_data_path: state.data_path.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn save_provider(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: SaveProviderInput,
) -> Result<ProviderRecord, String> {
    let name = required(&input.name, "Connection name")?;
    let base_url = validate_base_url(&input.base_url)?;
    if input.api_format != "openai-completions" && input.api_format != "openai-responses" {
        return Err("Choose a supported OpenAI-compatible API format".into());
    }
    validate_models(&input.models)?;
    let id = input.id.clone().unwrap_or_else(|| format!("custom-{}", Uuid::new_v4().simple()));
    let active_task = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().any(|task| task.provider_id == id && matches!(task.status, TaskStatus::Running | TaskStatus::Stopping));
    if active_task {
        return Err("Wait for tasks using this connection to finish before changing it".into());
    }
    let now = Utc::now().to_rfc3339();
    let created_at = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == id).map(|provider| provider.created_at.clone())
        .unwrap_or_else(|| now.clone());
    if let Some(api_key) = input.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
        keychain_entry(&id)?.set_password(api_key).map_err(keychain_error)?;
    }
    let has_api_key = read_api_key(&id).is_ok();
    let record = ProviderRecord {
        id: id.clone(),
        name,
        base_url,
        api_format: input.api_format,
        models: input.models,
        created_at,
        updated_at: now,
        has_api_key,
    };
    state.mutate(|data| {
        if let Some(existing) = data.providers.iter_mut().find(|provider| provider.id == id) {
            *existing = record.clone();
        } else {
            data.providers.push(record.clone());
        }
        Ok(())
    })?;

    let task_ids: Vec<String> = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().filter(|task| task.provider_id == id).map(|task| task.id.clone()).collect();
    for task_id in task_ids { worker::terminate_worker(&app, &task_id, true).await?; }
    Ok(record)
}

#[tauri::command]
pub async fn delete_provider(
    app: AppHandle,
    state: State<'_, MetadataState>,
    provider_id: String,
) -> Result<(), String> {
    let task_ids: Vec<String> = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().filter(|task| task.provider_id == provider_id).map(|task| task.id.clone()).collect();
    if !task_ids.is_empty() {
        return Err("This connection is still used by a saved task. Change those tasks to another connection before deleting it.".into());
    }
    worker::terminate_worker(&app, &provider_id, true).await.ok();
    if let Ok(entry) = keychain_entry(&provider_id) {
        if let Err(error) = entry.delete_credential() {
            if !matches!(error, keyring::Error::NoEntry) { return Err(keychain_error(error)); }
        }
    }
    state.mutate(|data| {
        data.providers.retain(|provider| provider.id != provider_id);
        Ok(())
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverModelsInput {
    pub provider_id: String,
}

#[tauri::command]
pub async fn discover_models(
    state: State<'_, MetadataState>,
    input: DiscoverModelsInput,
) -> Result<Vec<String>, String> {
    let provider = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == input.provider_id).cloned()
        .ok_or_else(|| "Connection not found".to_string())?;
    let api_key = read_api_key(&provider.id)?;
    let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_secs(20))
        .send().await.map_err(|error| format!("Could not fetch models: {error}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|error| format!("The model endpoint did not return JSON: {error}"))?;
    if !status.is_success() {
        let message = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("The provider rejected the model request");
        return Err(format!("Model discovery failed ({status}): {}", limit(message, 400)));
    }
    let mut ids: Vec<String> = body.get("data").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() { return Err("The endpoint returned no model IDs. You can still add one manually.".into()); }
    Ok(ids)
}

#[tauri::command]
pub fn add_project(state: State<'_, MetadataState>, path: String) -> Result<ProjectRecord, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() { return Err("Choose an existing folder".into()); }
    let canonical = path.canonicalize().map_err(|error| format!("Could not open that folder: {error}"))?;
    let canonical_string = canonical.to_string_lossy().into_owned();
    if let Some(existing) = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .projects.iter().find(|project| project.path == canonical_string).cloned() {
        return Ok(existing);
    }
    let git_info = git::inspect_project(&canonical);
    let record = ProjectRecord {
        id: Uuid::new_v4().to_string(),
        name: canonical.file_name().and_then(|name| name.to_str()).unwrap_or("Project").to_string(),
        path: canonical_string,
        git_root: git_info.root.map(|root| root.to_string_lossy().into_owned()),
        git_has_head: git_info.has_head,
        created_at: Utc::now().to_rfc3339(),
    };
    state.mutate(|data| { data.projects.push(record.clone()); Ok(()) })?;
    Ok(record)
}

#[tauri::command]
pub fn create_task(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: CreateTaskInput,
) -> Result<TaskRecord, String> {
    let name = required(&input.name, "Task name")?;
    let (project, provider) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let project = data.projects.iter().find(|project| project.id == input.project_id).cloned()
            .ok_or_else(|| "Project not found".to_string())?;
        let provider = data.providers.iter().find(|provider| provider.id == input.provider_id).cloned()
            .ok_or_else(|| "Connection not found".to_string())?;
        (project, provider)
    };
    validate_model_selection(&provider, &input.model_id, &input.thinking_level)?;
    let id = Uuid::new_v4().to_string();
    let mut workspace_path = PathBuf::from(&project.path);
    let mut worktree_path = None;
    let mut branch = git::current_branch(Path::new(&project.path));
    if input.use_worktree {
        if !project.git_has_head { return Err("Worktrees require a Git repository with at least one commit".into()); }
        let git_root = project.git_root.as_deref().ok_or_else(|| "This project is not inside a Git repository".to_string())?;
        let destination = app.path().app_data_dir().map_err(|error| error.to_string())?
            .join("worktrees").join(&id);
        let branch_name = format!("wackcode/{}-{}", slug(&name), &id[..8]);
        workspace_path = git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name)?;
        worktree_path = Some(destination.to_string_lossy().into_owned());
        branch = Some(branch_name);
    }
    let now = Utc::now().to_rfc3339();
    let record = TaskRecord {
        id,
        project_id: project.id,
        name,
        workspace_path: workspace_path.to_string_lossy().into_owned(),
        worktree_path,
        branch,
        uses_worktree: input.use_worktree,
        provider_id: provider.id,
        model_id: input.model_id,
        thinking_level: input.thinking_level,
        session_file: None,
        status: TaskStatus::Idle,
        archived: false,
        last_error: None,
        created_at: now.clone(),
        updated_at: now,
    };
    state.mutate(|data| { data.tasks.push(record.clone()); Ok(()) })?;
    Ok(record)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureTaskInput {
    pub task_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
}

#[tauri::command]
pub async fn configure_task(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: ConfigureTaskInput,
) -> Result<TaskRecord, String> {
    let provider = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == input.provider_id).cloned()
        .ok_or_else(|| "Connection not found".to_string())?;
    validate_model_selection(&provider, &input.model_id, &input.thinking_level)?;
    let (task, changed) = state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == input.task_id)
            .ok_or_else(|| "Task not found".to_string())?;
        if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
            return Err("Wait for this task to stop before changing its model".into());
        }
        let changed = task.provider_id != input.provider_id
            || task.model_id != input.model_id
            || task.thinking_level != input.thinking_level;
        task.provider_id = input.provider_id.clone();
        task.model_id = input.model_id.clone();
        task.thinking_level = input.thinking_level.clone();
        task.updated_at = Utc::now().to_rfc3339();
        Ok((task.clone(), changed))
    })?;
    if changed { worker::terminate_worker(&app, &task.id, true).await?; }
    Ok(task)
}

#[tauri::command]
pub async fn open_task(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<(), String> {
    let (task, provider) = task_and_provider(&state, &task_id)?;
    let api_key = read_api_key(&provider.id)?;
    worker::ensure_worker(&app, &task, &provider, &api_key).await?;
    worker::send(&app, &task.id, &json!({ "id": Uuid::new_v4().to_string(), "type": "snapshot" })).await
}

#[tauri::command]
pub async fn prompt(app: AppHandle, state: State<'_, MetadataState>, input: PromptInput) -> Result<String, String> {
    let message = required(&input.message, "Message")?;
    let configured = configure_task(app.clone(), state.clone(), ConfigureTaskInput {
        task_id: input.task_id.clone(),
        provider_id: input.provider_id,
        model_id: input.model_id,
        thinking_level: input.thinking_level,
    }).await?;
    let provider = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == configured.provider_id).cloned()
        .ok_or_else(|| "Connection not found".to_string())?;
    let api_key = read_api_key(&provider.id)?;
    worker::ensure_worker(&app, &configured, &provider, &api_key).await?;
    let run_id = Uuid::new_v4().to_string();
    worker::send(&app, &configured.id, &json!({
        "id": Uuid::new_v4().to_string(), "type": "prompt", "runId": run_id, "message": message
    })).await?;
    Ok(run_id)
}

#[tauri::command]
pub async fn stop_task(app: AppHandle, task_id: String) -> Result<(), String> {
    worker::send(&app, &task_id, &json!({ "id": Uuid::new_v4().to_string(), "type": "abort" })).await
}

#[tauri::command]
pub async fn archive_task(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<TaskRecord, String> {
    worker::terminate_worker(&app, &task_id, true).await?;
    state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == task_id).ok_or_else(|| "Task not found".to_string())?;
        task.archived = true;
        task.status = TaskStatus::Idle;
        task.updated_at = Utc::now().to_rfc3339();
        Ok(task.clone())
    })
}

#[tauri::command]
pub fn git_changes(state: State<'_, MetadataState>, task_id: String) -> Result<GitChanges, String> {
    let workspace = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == task_id).map(|task| task.workspace_path.clone())
        .ok_or_else(|| "Task not found".to_string())?;
    git::changes(Path::new(&workspace))
}

#[tauri::command]
pub fn reveal_task(state: State<'_, MetadataState>, task_id: String) -> Result<(), String> {
    let workspace = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == task_id).map(|task| task.workspace_path.clone())
        .ok_or_else(|| "Task not found".to_string())?;
    let status = Command::new("open").arg(&workspace).status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("macOS could not reveal this workspace".into()) }
}

fn task_and_provider(state: &State<'_, MetadataState>, task_id: &str) -> Result<(TaskRecord, ProviderRecord), String> {
    let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
    let task = data.tasks.iter().find(|task| task.id == task_id).cloned().ok_or_else(|| "Task not found".to_string())?;
    let provider = data.providers.iter().find(|provider| provider.id == task.provider_id).cloned()
        .ok_or_else(|| "This task's connection no longer exists".to_string())?;
    validate_model_selection(&provider, &task.model_id, &task.thinking_level)?;
    Ok((task, provider))
}

fn validate_models(models: &[ModelRecord]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for model in models {
        required(&model.id, "Model ID")?;
        if !ids.insert(model.id.trim()) { return Err(format!("Model ID is duplicated: {}", model.id)); }
        if model.context_window.is_some_and(|value| value == 0) { return Err("Context limits must be positive".into()); }
        if model.max_tokens.is_some_and(|value| value == 0) { return Err("Output limits must be positive".into()); }
        for level in &model.thinking_levels { validate_thinking(level)?; }
        for (level, mapped) in &model.thinking_level_map {
            validate_thinking(level)?;
            if mapped.as_ref().is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("Reasoning mapping for {level} cannot be empty; use null to omit it"));
            }
        }
    }
    Ok(())
}

fn validate_selected_model(provider: &ProviderRecord, model_id: &str) -> Result<(), String> {
    let model = provider.models.iter().find(|model| model.id == model_id)
        .ok_or_else(|| "The selected model is no longer configured".to_string())?;
    if model.context_window.is_none() || model.max_tokens.is_none() {
        return Err("Confirm this model's context and output limits in Settings before using it".into());
    }
    Ok(())
}

fn validate_model_selection(provider: &ProviderRecord, model_id: &str, thinking_level: &str) -> Result<(), String> {
    validate_thinking(thinking_level)?;
    validate_selected_model(provider, model_id)?;
    let model = provider.models.iter().find(|model| model.id == model_id)
        .ok_or_else(|| "The selected model is no longer configured".to_string())?;
    if !model.thinking_levels.iter().any(|level| level == thinking_level) {
        return Err(format!("The selected model does not support {thinking_level} reasoning"));
    }
    Ok(())
}

fn validate_thinking(level: &str) -> Result<(), String> {
    if THINKING_LEVELS.contains(&level) { Ok(()) } else { Err(format!("Unsupported reasoning effort: {level}")) }
}

fn validate_base_url(value: &str) -> Result<String, String> {
    let value = required(value, "Base URL")?.trim_end_matches('/').to_string();
    let parsed = reqwest::Url::parse(&value).map_err(|_| "Enter a valid base URL".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" { return Err("The base URL must use http or https".into()); }
    Ok(value)
}

fn required(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() { Err(format!("{label} is required")) } else { Ok(value.to_string()) }
}

fn keychain_entry(provider_id: &str) -> Result<Entry, String> {
    Entry::new(KEYCHAIN_SERVICE, provider_id).map_err(keychain_error)
}

fn read_api_key(provider_id: &str) -> Result<String, String> {
    keychain_entry(provider_id)?.get_password().map_err(|error| match error {
        keyring::Error::NoEntry => "Add an API key for this connection in Settings".into(),
        other => keychain_error(other),
    })
}

fn keychain_error(error: keyring::Error) -> String {
    format!("macOS Keychain error: {error}")
}

fn slug(value: &str) -> String {
    let slug: String = value.chars().map(|character| {
        if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '-' }
    }).collect();
    let slug = slug.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    if slug.is_empty() { "task".into() } else { slug.chars().take(32).collect() }
}

fn limit(value: &str, count: usize) -> String { value.chars().take(count).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_validation_allows_discovered_models_with_unknown_limits() {
        let models = vec![ModelRecord {
            id: "same/model".into(), name: "Same model".into(), context_window: None,
            max_tokens: None, reasoning: false, thinking_levels: vec!["off".into()],
            thinking_level_map: std::collections::BTreeMap::from([("off".into(), None)]),
        }];
        assert!(validate_models(&models).is_ok());
    }

    #[test]
    fn task_branch_slug_is_safe() {
        assert_eq!(slug("Fix: Sidebar & Chat"), "fix-sidebar-chat");
    }
}
