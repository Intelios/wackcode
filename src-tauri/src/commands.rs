use crate::{
    backgrounds, checkpoints, files, git, glass,
    models::{
        AppearanceConfig, BackdropMode, BootstrapPayload, BuiltinModelSuggestion, CheckpointChange, CheckpointRef, CreateTaskInput, ExportPlanInput,
        ForkTaskInput, GitChanges, ImageContent, ModelRecord, NavigateResult, NavigateTaskInput, NavigateTaskResult,
        ProjectRecord, PromptConfig, PromptInput, ExtensionUiResponseInput, InstallPackageInput, PackageRecord,
        PackageSearchResult, ProviderKind, ProviderRecord, ResendInput, RestoreCheckpointInput, RestoreResult,
        SaveProviderInput, SearchPackagesInput, SetPackageResourcesInput, SetTaskModeInput, SetToolConfigInput,
        SlashCommand, SubagentConfig, AutoTitleConfig, TaskMode, TaskRecord, TaskStatus, ToolConfig, WorkspaceFiles,
    },
    storage::MetadataState,
    worker::{self, WorkerOptions}, subagents, subscriptions,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

/// One lock per chat, held by every command that sends it work, moves its conversation, or
/// touches its checkpoints. Keeps a rewind from slipping in behind a prompt that is still being
/// sent, and keeps two Git operations off the same checkpoint store.
#[derive(Default)]
pub struct TaskLocks(Mutex<HashMap<String, Arc<AsyncMutex<()>>>>);

impl TaskLocks {
    fn for_task(&self, task_id: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.entry(task_id.to_string()).or_default().clone()
    }
}

fn task_lock(app: &AppHandle, task_id: &str) -> Arc<AsyncMutex<()>> {
    app.state::<TaskLocks>().for_task(task_id)
}

/// Moving in the session tree waits behind a worker that may still be starting up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

const THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];
/// Image limits for one prompt. `src/attachment-utils.ts` enforces the same numbers in the composer.
const MAX_PROMPT_IMAGES: usize = 8;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const IMAGE_MIME_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

#[tauri::command]
pub fn bootstrap(app: AppHandle, state: State<'_, MetadataState>) -> Result<BootstrapPayload, String> {
    let mut data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?.clone();
    for provider in &mut data.providers {
        if provider.kind == ProviderKind::Subscription {
            provider.has_api_key = false;
            provider.connected = subscriptions::has_credential(&app, &provider.id);
        } else {
            provider.has_api_key = state.secrets.get(&provider.id).is_ok();
            provider.connected = provider.has_api_key;
        }
    }
    for project in &mut data.projects {
        project.branch = git::current_branch(Path::new(&project.path));
    }
    Ok(BootstrapPayload {
        data,
        app_data_path: state.data_path.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned(),
        glass_supported: glass::is_supported(),
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
    if state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().any(|provider| provider.id == id && provider.kind == ProviderKind::Subscription) {
        return Err("Subscription connections are managed through sign-in".into());
    }
    if let Some(api_key) = input.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
        if api_key.starts_with("http://") || api_key.starts_with("https://") {
            return Err("That looks like a URL, not an API key — paste the key your provider issued".into());
        }
        state.secrets.set(&id, api_key)?;
    }
    let has_api_key = state.secrets.get(&id).is_ok();
    let record = ProviderRecord {
        id: id.clone(),
        name,
        kind: ProviderKind::Custom,
        base_url,
        api_format: input.api_format,
        models: input.models,
        created_at,
        updated_at: now,
        has_api_key,
        connected: has_api_key,
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
    // Other chats' sub-agents may use this connection with its old key or models.
    worker::broadcast_subagents(&app).await?;
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
    let agents = subagents::agents_using(&state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?.subagents, &provider_id);
    if !agents.is_empty() {
        return Err(format!(
            "The sub-agent {} uses this connection. Pick another model for it in Settings → Sub-agents before deleting it.",
            agents.join(", ")
        ));
    }
    let kind = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == provider_id).map(|provider| provider.kind)
        .ok_or_else(|| "Connection not found".to_string())?;
    if kind == ProviderKind::Subscription { subscriptions::remove_credential(&app, &provider_id)?; }
    else { state.secrets.remove(&provider_id)?; }
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
    if provider.kind == ProviderKind::Subscription { return Err("Subscription models are supplied by Pi".into()); }
    let api_key = state.secrets.get(&provider.id)?;
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
pub async fn list_builtin_models(app: AppHandle) -> Result<Vec<BuiltinModelSuggestion>, String> {
    worker::list_builtin_models(&app).await
}

const NPM_REGISTRY: &str = "https://registry.npmjs.org";
const SEARCH_PAGE_SIZE: u32 = 25;

/// Search the public npm registry for Pi packages.
///
/// pi.dev's own catalogue is exactly this: npm packages carrying the `pi-package` keyword. Its
/// site has no API (every /api route answers 501), so the registry is queried directly. This is
/// the only non-provider host WackCode contacts, and only when the user searches.
#[tauri::command]
pub async fn search_packages(input: SearchPackagesInput) -> Result<Vec<PackageSearchResult>, String> {
    let query = input.query.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let text = match query {
        Some(query) => format!("keywords:pi-package {query}"),
        None => "keywords:pi-package".to_string(),
    };
    let url = format!(
        "{NPM_REGISTRY}/-/v1/search?text={}&size={SEARCH_PAGE_SIZE}&from={}",
        urlencoding(&text),
        input.from.unwrap_or(0)
    );
    let response = reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .send().await.map_err(|error| format!("Could not reach the npm registry: {error}"))?;
    let status = response.status();
    let body: Value = response.json().await
        .map_err(|error| format!("The npm registry did not return JSON: {error}"))?;
    if !status.is_success() {
        return Err(format!("Package search failed ({status})."));
    }
    Ok(body.get("objects").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|item| search_result(item.get("package")?))
        .collect())
}

fn search_result(package: &Value) -> Option<PackageSearchResult> {
    let name = package.get("name").and_then(Value::as_str)?.to_string();
    let links = package.get("links");
    Some(PackageSearchResult {
        npm_url: links.and_then(|links| links.get("npm")).and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://www.npmjs.com/package/{name}")),
        repository: links.and_then(|links| links.get("repository")).and_then(Value::as_str).map(str::to_string),
        version: package.get("version").and_then(Value::as_str).unwrap_or("").to_string(),
        description: package.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        publisher: package.pointer("/publisher/username").and_then(Value::as_str).unwrap_or("").to_string(),
        published_at: package.get("date").and_then(Value::as_str).unwrap_or("").to_string(),
        declares: Vec::new(),
        name,
    })
}

/// Fetch one package's manifest so the user can see what installing it actually adds before
/// they accept the trust warning.
#[tauri::command]
pub async fn package_details(name: String) -> Result<PackageSearchResult, String> {
    let name = required(&name, "Package name")?;
    if name.contains("..") || name.contains(' ') {
        return Err("That is not a valid npm package name.".into());
    }
    let response = reqwest::Client::new()
        .get(format!("{NPM_REGISTRY}/{}/latest", urlencoding(&name)))
        .timeout(std::time::Duration::from_secs(20))
        .send().await.map_err(|error| format!("Could not reach the npm registry: {error}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("No package named {name} was found on npm."));
    }
    let body: Value = response.json().await
        .map_err(|error| format!("The npm registry did not return JSON: {error}"))?;
    if !status.is_success() {
        return Err(format!("Could not load {name} ({status})."));
    }
    let mut result = search_result(&body).ok_or_else(|| "The npm registry returned an unexpected response.".to_string())?;
    // The `pi` manifest says which resource kinds the package contributes.
    if let Some(manifest) = body.get("pi").and_then(Value::as_object) {
        result.declares = ["extensions", "skills", "prompts", "themes"].into_iter()
            .filter(|kind| manifest.contains_key(*kind))
            .map(str::to_string)
            .collect();
    }
    // The manifest spells these differently from the search index: no `links` object, the
    // publisher under `_npmUser`, and the repository as an object rather than a URL string.
    result.publisher = body.pointer("/_npmUser/name").and_then(Value::as_str)
        .or_else(|| body.pointer("/author/name").and_then(Value::as_str))
        .unwrap_or("").to_string();
    result.repository = body.pointer("/repository/url").and_then(Value::as_str)
        .map(|url| url.trim_start_matches("git+").trim_end_matches(".git").to_string())
        .or_else(|| body.get("homepage").and_then(Value::as_str).map(str::to_string));
    Ok(result)
}

/// Minimal percent-encoding for query values. The registry only ever sees package names and
/// search words, so the unreserved set plus a few safe characters is enough.
fn urlencoding(value: &str) -> String {
    value.chars().map(|character| match character {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | ':' | '@' | '/' => character.to_string(),
        ' ' => "+".to_string(),
        other => other.to_string().bytes().map(|byte| format!("%{byte:02X}")).collect(),
    }).collect()
}

/// Answer a dialog an extension raised. The worker routes this past its command queue, because
/// the extension is usually waiting inside an in-flight prompt.
#[tauri::command]
pub async fn respond_extension_ui(app: AppHandle, input: ExtensionUiResponseInput) -> Result<(), String> {
    let mut payload = json!({
        "id": Uuid::new_v4().to_string(),
        "type": "extension_ui_response",
        "requestId": required(&input.request_id, "Dialog id")?,
    });
    if let Some(value) = input.value { payload["value"] = Value::String(value); }
    if let Some(confirmed) = input.confirmed { payload["confirmed"] = Value::Bool(confirmed); }
    if input.cancelled == Some(true) { payload["cancelled"] = Value::Bool(true); }
    if let Some(answers) = input.answers {
        payload["answers"] = serde_json::to_value(answers).map_err(|error| error.to_string())?;
    }
    if input.wrap_up == Some(true) { payload["wrapUp"] = Value::Bool(true); }
    worker::send(&app, &input.task_id, &payload).await
}

/// Read the installed packages back from the shared store and reconcile them with the trust
/// receipts in `wackcode.json`. A package present on disk but never trusted here stays untrusted
/// and so never loads.
async fn sync_packages(
    app: &AppHandle,
    state: &State<'_, MetadataState>,
    catalog: Value,
    // Source the user just accepted the warning for. Only this one may gain trust here.
    newly_trusted: Option<&str>,
) -> Result<Vec<PackageRecord>, String> {
    let now = Utc::now().to_rfc3339();
    let existing: Vec<PackageRecord> = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.packages.clone()
    };

    let empty = Vec::new();
    let mut records = Vec::new();
    for entry in catalog.as_array().unwrap_or(&empty) {
        let source = entry.get("source").and_then(Value::as_str).unwrap_or_default().to_string();
        if source.is_empty() { continue; }
        let previous = existing.iter().find(|record| record.source == source);
        records.push(PackageRecord {
            source: source.clone(),
            display_name: entry.get("displayName").and_then(Value::as_str).unwrap_or(&source).to_string(),
            kind: entry.get("kind").and_then(Value::as_str).unwrap_or("local").to_string(),
            version: entry.get("version").and_then(Value::as_str).map(str::to_string),
            installed_path: entry.get("installedPath").and_then(Value::as_str).map(str::to_string),
            extensions: resources(entry, "extensions"),
            skills: resources(entry, "skills"),
            prompts: resources(entry, "prompts"),
            themes: resources(entry, "themes"),
            errors: entry.get("errors").and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            // Trust is only ever granted by an explicit install the user confirmed. A package
            // that turns up in the shared store some other way stays untrusted, so nothing it
            // contains can reach a worker until the user reviews it.
            trusted_at: previous
                .map(|record| record.trusted_at.clone())
                .unwrap_or_else(|| if newly_trusted == Some(source.as_str()) { now.clone() } else { String::new() }),
            installed_at: previous.map(|record| record.installed_at.clone()).unwrap_or_else(|| now.clone()),
        });
    }

    state.mutate(|data| { data.packages = records.clone(); Ok(()) })?;
    // Loaded resources are baked into a worker at spawn time, so every worker must restart.
    // Collect first: the metadata guard must not be held across an await.
    let task_ids: Vec<String> = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.tasks.iter().map(|task| task.id.clone()).collect()
    };
    for task_id in task_ids {
        worker::terminate_worker(app, &task_id, true).await?;
    }
    Ok(records)
}

fn resources(entry: &Value, kind: &str) -> Vec<crate::models::PackageResourceRecord> {
    entry.get(kind).and_then(Value::as_array).map(|items| {
        items.iter().filter_map(|item| Some(crate::models::PackageResourceRecord {
            path: item.get("path").and_then(Value::as_str)?.to_string(),
            name: item.get("name").and_then(Value::as_str)?.to_string(),
            enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        })).collect()
    }).unwrap_or_default()
}

#[tauri::command]
pub fn list_packages(state: State<'_, MetadataState>) -> Result<Vec<PackageRecord>, String> {
    Ok(state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?.packages.clone())
}

#[tauri::command]
pub async fn refresh_packages(app: AppHandle, state: State<'_, MetadataState>) -> Result<Vec<PackageRecord>, String> {
    let catalog = worker::run_manager(&app, json!({ "type": "list" })).await?;
    sync_packages(&app, &state, catalog, None).await
}

#[tauri::command]
pub async fn install_package(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: InstallPackageInput,
) -> Result<Vec<PackageRecord>, String> {
    let source = validate_package_source(&input.source)?;
    if !input.trusted {
        return Err("Accept the installation warning before installing this package.".into());
    }
    refuse_while_busy(&state)?;
    let catalog = worker::run_manager(&app, json!({ "type": "install", "source": source })).await?;
    sync_packages(&app, &state, catalog, Some(&source)).await
}

#[tauri::command]
pub async fn remove_package(
    app: AppHandle,
    state: State<'_, MetadataState>,
    source: String,
) -> Result<Vec<PackageRecord>, String> {
    let source = validate_package_source(&source)?;
    refuse_while_busy(&state)?;
    let catalog = worker::run_manager(&app, json!({ "type": "remove", "source": source })).await?;
    sync_packages(&app, &state, catalog, None).await
}

#[tauri::command]
pub async fn update_packages(
    app: AppHandle,
    state: State<'_, MetadataState>,
    source: Option<String>,
) -> Result<Vec<PackageRecord>, String> {
    refuse_while_busy(&state)?;
    let mut command = json!({ "type": "update" });
    if let Some(source) = source {
        command["source"] = Value::String(validate_package_source(&source)?);
    }
    let catalog = worker::run_manager(&app, command).await?;
    sync_packages(&app, &state, catalog, None).await
}

#[tauri::command]
pub async fn set_package_resources(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: SetPackageResourcesInput,
) -> Result<Vec<PackageRecord>, String> {
    let source = validate_package_source(&input.source)?;
    refuse_while_busy(&state)?;
    let mut command = json!({ "type": "set_resources", "source": source });
    for (kind, selection) in [
        ("extensions", input.extensions),
        ("skills", input.skills),
        ("prompts", input.prompts),
        ("themes", input.themes),
    ] {
        if let Some(selection) = selection {
            command[kind] = json!(selection);
        }
    }
    let catalog = worker::run_manager(&app, command).await?;
    sync_packages(&app, &state, catalog, None).await
}

/// Grant trust to a package already present in the store but never confirmed here, so the user
/// can review and accept it without reinstalling.
#[tauri::command]
pub async fn trust_package(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: InstallPackageInput,
) -> Result<Vec<PackageRecord>, String> {
    let source = validate_package_source(&input.source)?;
    if !input.trusted {
        return Err("Accept the installation warning before enabling this package.".into());
    }
    refuse_while_busy(&state)?;
    let now = Utc::now().to_rfc3339();
    let found = state.mutate(|data| {
        let Some(record) = data.packages.iter_mut().find(|record| record.source == source) else {
            return Ok(false);
        };
        if record.trusted_at.is_empty() {
            record.trusted_at = now.clone();
        }
        Ok(true)
    })?;
    if !found {
        return Err(format!("That package is not installed: {source}"));
    }
    let task_ids: Vec<String> = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.tasks.iter().map(|task| task.id.clone()).collect()
    };
    for task_id in task_ids {
        worker::terminate_worker(&app, &task_id, true).await?;
    }
    list_packages(state)
}

/// Appearance settings are purely cosmetic: the renderer themes itself and the native window
/// follows the backdrop, so no worker hears about them.
#[tauri::command]
pub async fn set_appearance_config(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: AppearanceConfig,
) -> Result<AppearanceConfig, String> {
    let config = state.mutate(|data| {
        let config = validate_appearance_config(input, &data.appearance, glass::is_supported())?;
        data.appearance = config.clone();
        Ok(config)
    })?;
    glass::sync(&app, &config)?;
    Ok(config)
}

/// Opens the native picker (so the renderer never supplies a path), stores a validated copy of
/// the chosen image and switches the backdrop to it. `None` when the user cancels.
#[tauri::command]
pub async fn choose_background_image(app: AppHandle, state: State<'_, MetadataState>) -> Result<Option<AppearanceConfig>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(picked) = app.dialog().file().set_title("Choose a background image").add_filter("Images", &["png", "jpg", "jpeg", "webp"]).blocking_pick_file() else {
        return Ok(None);
    };
    let source = picked.into_path().map_err(|_| "That image could not be read.".to_string())?;
    let folder = backgrounds::directory(&app.path().app_data_dir().map_err(|error| error.to_string())?);
    let name = backgrounds::import(&source, &folder)?;
    let config = state.mutate(|data| {
        data.appearance.background_image = Some(name.clone());
        data.appearance.backdrop = BackdropMode::Image;
        Ok(data.appearance.clone())
    })?;
    backgrounds::prune(&folder, Some(&name));
    glass::sync(&app, &config)?;
    Ok(Some(config))
}

/// Forgets the background image and deletes WackCode's copy (never the user's original).
#[tauri::command]
pub async fn remove_background_image(app: AppHandle, state: State<'_, MetadataState>) -> Result<AppearanceConfig, String> {
    let config = state.mutate(|data| {
        data.appearance.background_image = None;
        if data.appearance.backdrop == BackdropMode::Image {
            data.appearance.backdrop = BackdropMode::Solid;
        }
        Ok(data.appearance.clone())
    })?;
    backgrounds::prune(&backgrounds::directory(&app.path().app_data_dir().map_err(|error| error.to_string())?), None);
    glass::sync(&app, &config)?;
    Ok(config)
}

/// Custom prompts apply to running chats on their next turn, like tool changes: workers are
/// updated in place, never restarted (the overrides deliberately stay out of the fingerprint).
#[tauri::command]
pub async fn set_prompt_config(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: PromptConfig,
) -> Result<PromptConfig, String> {
    let config = validate_prompt_config(input)?;
    state.mutate(|data| {
        data.prompts = config.clone();
        Ok(())
    })?;
    worker::broadcast(&app, &json!({
        "id": Uuid::new_v4().to_string(), "type": "set_prompts", "prompts": config
    })).await?;
    Ok(config)
}

/// Tool changes take effect on the next agent turn, so running workers are updated in place
/// instead of being restarted the way a connection change restarts them.
#[tauri::command]
pub async fn set_tool_config(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: SetToolConfigInput,
) -> Result<ToolConfig, String> {
    let disabled = validate_tool_names(&input.disabled)?;
    let config = ToolConfig { disabled: disabled.clone() };
    state.mutate(|data| {
        data.tool_config = config.clone();
        Ok(())
    })?;
    worker::broadcast(&app, &json!({
        "id": Uuid::new_v4().to_string(), "type": "set_tools", "disabledTools": disabled
    })).await?;
    Ok(config)
}

/// Sub-agent settings apply to running chats on their next turn, like tool changes: nothing
/// restarts. Turning the feature off also withdraws every sub-agent credential from workers.
#[tauri::command]
pub async fn set_subagent_config(
    app: AppHandle,
    state: State<'_, MetadataState>,
    input: SubagentConfig,
) -> Result<SubagentConfig, String> {
    let config = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        subagents::validate(&input, &data.providers)?
    };
    state.mutate(|data| {
        data.subagents = config.clone();
        Ok(())
    })?;
    worker::broadcast_subagents(&app).await?;
    Ok(config)
}

#[tauri::command]
pub fn set_auto_title_config(state: State<'_, MetadataState>, input: AutoTitleConfig) -> Result<AutoTitleConfig, String> {
    if input.enabled {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let provider = data.providers.iter().find(|item| Some(item.id.as_str()) == input.provider_id.as_deref())
            .ok_or_else(|| "Choose a connection for automatic titles".to_string())?;
        validate_selected_model(provider, input.model_id.as_deref().unwrap_or(""))?;
        if !provider.connected { return Err("Connect the title model before enabling automatic titles".into()); }
    }
    state.mutate(|data| { data.auto_title = input.clone(); Ok(input) })
}

#[tauri::command]
pub fn add_project(state: State<'_, MetadataState>, path: String) -> Result<ProjectRecord, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() { return Err("Choose an existing folder".into()); }
    let canonical = path.canonicalize().map_err(|error| format!("Could not open that folder: {error}"))?;
    let canonical_string = canonical.to_string_lossy().into_owned();
    if let Some(existing) = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .projects.iter().find(|project| project.path == canonical_string).cloned() {
        let mut existing = existing;
        existing.branch = git::current_branch(&canonical);
        return Ok(existing);
    }
    let git_info = git::inspect_project(&canonical);
    let record = ProjectRecord {
        id: Uuid::new_v4().to_string(),
        name: canonical.file_name().and_then(|name| name.to_str()).unwrap_or("Project").to_string(),
        path: canonical_string.clone(),
        git_root: git_info.root.map(|root| root.to_string_lossy().into_owned()),
        git_has_head: git_info.has_head,
        branch: git::current_branch(&canonical),
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
    let name = input.name.as_deref().map(str::trim).filter(|value| !value.is_empty())
        .unwrap_or("New chat").to_string();
    let (project, provider) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let project = input.project_id.as_deref()
            .map(|project_id| data.projects.iter().find(|project| project.id == project_id).cloned()
                .ok_or_else(|| "Project not found".to_string()))
            .transpose()?;
        let provider = data.providers.iter().find(|provider| provider.id == input.provider_id).cloned()
            .ok_or_else(|| "Connection not found".to_string())?;
        (project, provider)
    };
    validate_model_selection(&provider, &input.model_id, &input.thinking_level)?;
    let id = Uuid::new_v4().to_string();
    let mut workspace_path = project.as_ref().map(|project| PathBuf::from(&project.path));
    let mut worktree_path = None;
    let mut branch = project.as_ref().and_then(|project| git::current_branch(Path::new(&project.path)));
    if input.use_worktree {
        let project = project.as_ref().ok_or_else(|| "Worktrees require a project".to_string())?;
        if !project.git_has_head { return Err("Worktrees require a Git repository with at least one commit".into()); }
        let git_root = project.git_root.as_deref().ok_or_else(|| "This project is not inside a Git repository".to_string())?;
        let destination = app.path().app_data_dir().map_err(|error| error.to_string())?
            .join("worktrees").join(&id);
        let branch_name = format!("wackcode/{}-{}", slug(&name), &id[..8]);
        workspace_path = Some(git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name, "HEAD")?);
        worktree_path = Some(destination.to_string_lossy().into_owned());
        branch = Some(branch_name);
    }
    let workspace_path = match workspace_path {
        Some(path) => path,
        None => {
            let scratch = scratch_dir(&app, &id)?;
            std::fs::create_dir_all(&scratch).map_err(|error| format!("Could not create scratch folder: {error}"))?;
            scratch
        }
    };
    let now = Utc::now().to_rfc3339();
    let record = TaskRecord {
        id,
        project_id: project.as_ref().map(|project| project.id.clone()),
        name,
        auto_title_eligible: true,
        auto_title_attempt_id: None,
        workspace_path: workspace_path.to_string_lossy().into_owned(),
        worktree_path,
        branch,
        uses_worktree: input.use_worktree,
        provider_id: provider.id,
        model_id: input.model_id,
        thinking_level: input.thinking_level,
        session_file: None,
        status: TaskStatus::Idle,
        mode: TaskMode::Build,
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
    let api_key = credential_for(&app, &state, &provider)?;
    {
        let lock = task_lock(&app, &task_id);
        let _guard = lock.lock().await;
        worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    }
    worker::send(&app, &task.id, &json!({ "id": Uuid::new_v4().to_string(), "type": "snapshot" })).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteCommandInput {
    task_id: String,
    command_id: String,
    args: String,
    started_at: u64,
    #[serde(default)]
    images: Vec<crate::models::ImageContent>,
}

#[tauri::command]
pub async fn list_commands(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<Vec<SlashCommand>, String> {
    let lock = task_lock(&app, &task_id);
    let _guard = lock.lock().await;
    let (task, provider) = task_and_provider(&state, &task_id)?;
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
        return Err("Wait for this chat to finish before loading commands.".into());
    }
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    let value = worker::request(&app, &task_id, json!({ "id": Uuid::new_v4().to_string(), "type": "list_commands" }), REQUEST_TIMEOUT).await?;
    serde_json::from_value(value).map_err(|_| "Pi returned an invalid command list.".to_string())
}

#[tauri::command]
pub async fn execute_command(app: AppHandle, state: State<'_, MetadataState>, input: ExecuteCommandInput) -> Result<String, String> {
    let command_id = required(&input.command_id, "Command")?;
    if input.args.len() > 100_000 { return Err("Command arguments are too long.".into()); }
    validate_images(&input.images)?;
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    let (task, provider) = task_and_provider(&state, &input.task_id)?;
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) { return Err("Wait for this chat to finish before running a command.".into()); }
    if !input.images.is_empty() { require_vision(&provider, &task.model_id)?; }
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    set_status(&state, &task.id, TaskStatus::Running)?;
    let checkpoint = match checkpoint_location(&app, &state, &task) {
        Ok(location) => snapshot_quietly(&app, &task.id, &location).await,
        Err(_) => None,
    };
    let run_id = Uuid::new_v4().to_string();
    let result = worker::request(&app, &task.id, json!({
        "id": Uuid::new_v4().to_string(), "type": "execute_command", "commandId": command_id,
        "args": input.args, "startedAt": input.started_at, "runId": run_id,
        "images": input.images, "checkpoint": checkpoint
    }), REQUEST_TIMEOUT).await;
    if let Err(error) = result {
        let _ = set_status(&state, &task.id, TaskStatus::Idle);
        return Err(error);
    }
    Ok(run_id)
}

#[tauri::command]
pub async fn init_agents(app: AppHandle, state: State<'_, MetadataState>, task_id: String, started_at: u64) -> Result<String, String> {
    let lock = task_lock(&app, &task_id);
    let _guard = lock.lock().await;
    let (task, provider) = task_and_provider(&state, &task_id)?;
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
        return Err("Wait for this chat to finish before running /init.".into());
    }
    validate_init_agents_task(task.project_id.as_deref(), task.mode)?;
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    set_status(&state, &task.id, TaskStatus::Running)?;
    let checkpoint = match checkpoint_location(&app, &state, &task) {
        Ok(location) => snapshot_quietly(&app, &task.id, &location).await,
        Err(_) => None,
    };
    let run_id = Uuid::new_v4().to_string();
    let result = worker::request(&app, &task.id, json!({
        "id": Uuid::new_v4().to_string(), "type": "init_agents",
        "startedAt": started_at, "runId": run_id, "checkpoint": checkpoint
    }), REQUEST_TIMEOUT).await;
    if let Err(error) = result {
        let _ = set_status(&state, &task.id, TaskStatus::Idle);
        return Err(error);
    }
    Ok(run_id)
}

#[tauri::command]
pub async fn compact_task(app: AppHandle, state: State<'_, MetadataState>, task_id: String, instructions: String, started_at: u64) -> Result<String, String> {
    if instructions.len() > 100_000 { return Err("Compaction instructions are too long.".into()); }
    let lock = task_lock(&app, &task_id);
    let _guard = lock.lock().await;
    let (task, provider) = task_and_provider(&state, &task_id)?;
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) { return Err("Wait for this chat to finish before compacting.".into()); }
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    set_status(&state, &task.id, TaskStatus::Running)?;
    let run_id = Uuid::new_v4().to_string();
    let result = worker::request(&app, &task.id, json!({
        "id": Uuid::new_v4().to_string(), "type": "compact", "instructions": instructions,
        "startedAt": started_at, "runId": run_id
    }), REQUEST_TIMEOUT).await;
    if let Err(error) = result {
        let _ = set_status(&state, &task.id, TaskStatus::Idle);
        return Err(error);
    }
    Ok(run_id)
}

#[tauri::command]
pub async fn prompt(app: AppHandle, state: State<'_, MetadataState>, input: PromptInput) -> Result<String, String> {
    let message = required(&input.message, "Message")?;
    validate_images(&input.images)?;
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    // A chat gets one chance, consumed durably even when the extension is off or dispatch fails.
    // This also moves the free fallback rename out of the renderer's asynchronous path.
    let (title_attempt, title_config, fallback_name) = state.mutate(|data| {
        let config = data.auto_title.clone();
        let task = data.tasks.iter_mut().find(|task| task.id == input.task_id)
            .ok_or_else(|| "Chat not found".to_string())?;
        let first = task.auto_title_eligible;
        if first && task.name == "New chat" {
            task.name = limit(&message.split_whitespace().collect::<Vec<_>>().join(" "), 48);
        }
        let attempt = consume_auto_title_eligibility(task, config.enabled);
        Ok((attempt, config, if first { Some(task.name.clone()) } else { None }))
    })?;
    if let Some(name) = fallback_name {
        let _ = app.emit("worker-event", json!({ "type": "title_changed", "taskId": input.task_id, "name": name }));
    }
    let configured = configure_task(app.clone(), state.clone(), ConfigureTaskInput {
        task_id: input.task_id.clone(),
        provider_id: input.provider_id,
        model_id: input.model_id,
        thinking_level: input.thinking_level,
    }).await?;
    // A prompt carries the composer's mode (drafts have no worker yet, so this is also the
    // only way a task's first message can start in Plan mode). The worker's `plan_state`
    // keeps the record in sync afterwards.
    let configured = match input.mode {
        Some(mode) if mode != configured.mode => {
            state.mutate(|data| {
                let task = data.tasks.iter_mut().find(|task| task.id == configured.id)
                    .ok_or_else(|| "Task not found".to_string())?;
                task.mode = mode;
                task.updated_at = Utc::now().to_rfc3339();
                Ok(task.clone())
            })?
        }
        _ => configured,
    };
    let provider = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == configured.provider_id).cloned()
        .ok_or_else(|| "Connection not found".to_string())?;
    if !input.images.is_empty() { require_vision(&provider, &configured.model_id)?; }
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &configured, &provider, api_key.as_deref()).await?;
    // Running from here on, so nothing that moves the conversation can queue up behind this
    // prompt and run after it. The worker's `run_state` takes over once it starts.
    set_status(&state, &configured.id, TaskStatus::Running)?;
    let checkpoint = match checkpoint_location(&app, &state, &configured) {
        Ok(location) => snapshot_quietly(&app, &configured.id, &location).await,
        Err(_) => None,
    };
    let run_id = Uuid::new_v4().to_string();
    let started_at = input.started_at.unwrap_or_else(|| Utc::now().timestamp_millis() as u64);
    let had_title_attempt = title_attempt.is_some();
    let auto_title = title_attempt.and_then(|attempt_id| {
        let data = state.data.lock().ok()?;
        let provider = data.providers.iter().find(|item| Some(item.id.as_str()) == title_config.provider_id.as_deref())?;
        let model_id = title_config.model_id.as_deref()?;
        if !provider.connected || validate_selected_model(provider, model_id).is_err() { return None; }
        let credential = credential_for(&app, &state, provider).ok()?;
        let auth_path = if provider.kind == ProviderKind::Subscription {
            Some(subscriptions::auth_path(&app, &provider.id).ok()?.to_string_lossy().into_owned())
        } else { None };
        Some(json!({ "attemptId": attempt_id, "provider": worker::worker_provider_json(provider),
            "modelId": model_id, "apiKey": credential, "authPath": auth_path }))
    });
    if had_title_attempt && auto_title.is_none() {
        let _ = state.mutate(|data| {
            if let Some(task) = data.tasks.iter_mut().find(|task| task.id == input.task_id) {
                task.auto_title_attempt_id = None;
            }
            Ok(())
        });
        let _ = app.emit("worker-event", json!({ "type": "extension_notice", "taskId": input.task_id,
            "message": "Automatic title could not be generated. Check its model connection in Settings.", "level": "warning" }));
    }
    let sent = worker::send(&app, &configured.id, &json!({
        "id": Uuid::new_v4().to_string(), "type": "prompt", "runId": run_id, "message": message,
        "startedAt": started_at, "mode": configured.mode, "images": input.images, "checkpoint": checkpoint,
        "literal": input.literal, "autoTitle": auto_title
    })).await;
    if let Err(error) = sent {
        let _ = set_status(&state, &configured.id, TaskStatus::Idle);
        return Err(error);
    }
    Ok(run_id)
}

fn consume_auto_title_eligibility(task: &mut TaskRecord, enabled: bool) -> Option<String> {
    if !task.auto_title_eligible { return None; }
    task.auto_title_eligible = false;
    if !enabled { return None; }
    let id = Uuid::new_v4().to_string();
    task.auto_title_attempt_id = Some(id.clone());
    Some(id)
}

/// Send a message again as a new version of itself: unchanged (retry) or with new text (edit).
/// Files can first be put back to how they were before the message; if the worker then refuses,
/// they are put back again.
#[tauri::command]
pub async fn resend_message(app: AppHandle, state: State<'_, MetadataState>, input: ResendInput) -> Result<String, String> {
    validate_entry_id(&input.entry_id)?;
    let message = input.message.as_deref().map(|message| required(message, "Message")).transpose()?;
    if let Some(selection) = &input.restore { validate_checkpoint_id(&selection.checkpoint_id)?; }
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    let configured = configure_task(app.clone(), state.clone(), ConfigureTaskInput {
        task_id: input.task_id.clone(),
        provider_id: input.provider_id.clone(),
        model_id: input.model_id.clone(),
        thinking_level: input.thinking_level.clone(),
    }).await?;
    let provider = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .providers.iter().find(|provider| provider.id == configured.provider_id).cloned()
        .ok_or_else(|| "Connection not found".to_string())?;
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &configured, &provider, api_key.as_deref()).await?;
    set_status(&state, &configured.id, TaskStatus::Running)?;
    let location = checkpoint_location(&app, &state, &configured).ok();
    let leave = match &location { Some(location) => snapshot_quietly(&app, &configured.id, location).await, None => None };
    let mut undo = None;
    let outcome = async {
        let checkpoint = match (&input.restore, &location) {
            (Some(selection), Some(location)) => {
                let result = restore_files(location, &selection.checkpoint_id, selection.paths.clone()).await?;
                undo = Some(result.undo);
                // A fresh snapshot, not the restored id: a partial restore leaves other changes.
                snapshot_quietly(&app, &configured.id, location).await
            }
            (Some(_), None) => return Err("Checkpoints are not available for this chat.".to_string()),
            _ => leave.clone(),
        };
        let run_id = Uuid::new_v4().to_string();
        let started_at = input.started_at.unwrap_or_else(|| Utc::now().timestamp_millis() as u64);
        worker::request(&app, &configured.id, json!({
            "id": Uuid::new_v4().to_string(), "type": "resend", "runId": run_id, "startedAt": started_at,
            "entryId": input.entry_id, "message": message, "removeImages": input.remove_images,
            "checkpoint": checkpoint, "leave": leave
        }), REQUEST_TIMEOUT).await?;
        Ok(run_id)
    }.await;
    if outcome.is_err() {
        if let (Some(undo), Some(location)) = (&undo, &location) {
            let _ = restore_files(location, &undo.id, None).await;
        }
        let _ = set_status(&state, &configured.id, TaskStatus::Idle);
    }
    outcome
}

/// Move the conversation to another point in its session tree: rewind to before a message,
/// switch to another version, or undo a rewind. Files are restored afterwards when asked; if
/// that fails the conversation still moved, and the result says so.
#[tauri::command]
pub async fn navigate_task(app: AppHandle, state: State<'_, MetadataState>, input: NavigateTaskInput) -> Result<NavigateTaskResult, String> {
    validate_entry_id(&input.entry_id)?;
    if !matches!(input.target.as_str(), "before" | "latest") { return Err("Unknown place to move the conversation to.".into()); }
    if !matches!(input.kind.as_str(), "rewind" | "switch" | "undo") { return Err("Unknown way to move the conversation.".into()); }
    if let Some(selection) = &input.restore { validate_checkpoint_id(&selection.checkpoint_id)?; }
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    let (task, provider) = task_and_provider(&state, &input.task_id)?;
    refuse_busy(&task)?;
    let api_key = credential_for(&app, &state, &provider)?;
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    let location = checkpoint_location(&app, &state, &task).ok();
    let leave = match &location { Some(location) => snapshot_quietly(&app, &task.id, location).await, None => None };
    let result = worker::request(&app, &task.id, json!({
        "id": Uuid::new_v4().to_string(), "type": "navigate", "entryId": input.entry_id,
        "target": input.target, "kind": input.kind, "leave": leave
    }), REQUEST_TIMEOUT).await?;
    let navigate: NavigateResult = serde_json::from_value(result).unwrap_or_default();
    let (restore, restore_error) = match (&input.restore, &location) {
        (Some(selection), Some(location)) => match restore_files(location, &selection.checkpoint_id, selection.paths.clone()).await {
            Ok(result) => (Some(result), None),
            Err(error) => (None, Some(error)),
        },
        (Some(_), None) => (None, Some("Checkpoints are not available for this chat.".to_string())),
        _ => (None, None),
    };
    Ok(NavigateTaskResult { navigate, restore, restore_error })
}

/// Put the chat's files back to a checkpoint, optionally only some of them.
#[tauri::command]
pub async fn restore_checkpoint(app: AppHandle, state: State<'_, MetadataState>, input: RestoreCheckpointInput) -> Result<RestoreResult, String> {
    validate_checkpoint_id(&input.checkpoint_id)?;
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    let task = find_task(&state, &input.task_id)?;
    refuse_busy(&task)?;
    let location = checkpoint_location(&app, &state, &task)?;
    refuse_shared_run(&app, &state, &task, &location.root)?;
    restore_files(&location, &input.checkpoint_id, input.paths).await
}

/// The files restoring a checkpoint would change, for the user to review first.
#[tauri::command]
pub async fn checkpoint_changes(app: AppHandle, state: State<'_, MetadataState>, task_id: String, checkpoint_id: String) -> Result<Vec<CheckpointChange>, String> {
    validate_checkpoint_id(&checkpoint_id)?;
    let lock = task_lock(&app, &task_id);
    let _guard = lock.lock().await;
    let task = find_task(&state, &task_id)?;
    let location = checkpoint_location(&app, &state, &task)?;
    blocking(move || checkpoints::changes(&location.shadow, &location.root, &location.app_data, &checkpoint_id)).await
}

/// Start a new chat from a point in this one. The new chat's session is the path to that point
/// (Pi's fork); its files depend on where the source works: a Local chat shares its folder, a
/// worktree or scratch chat gets its own copy of the files as they were after that turn.
#[tauri::command]
pub async fn fork_task(app: AppHandle, state: State<'_, MetadataState>, input: ForkTaskInput) -> Result<TaskRecord, String> {
    if let Some(entry_id) = &input.entry_id { validate_entry_id(entry_id)?; }
    if let Some(checkpoint) = &input.checkpoint { validate_checkpoint_id(&checkpoint.id)?; }
    let lock = task_lock(&app, &input.task_id);
    let _guard = lock.lock().await;
    let (source, provider) = task_and_provider(&state, &input.task_id)?;
    refuse_busy(&source)?;
    let session_file = source.session_file.clone().filter(|file| Path::new(file).exists())
        .ok_or_else(|| "Wait for the first reply before forking this chat.".to_string())?;
    let project = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        source.project_id.as_deref().and_then(|id| data.projects.iter().find(|project| project.id == id).cloned())
    };
    let api_key = credential_for(&app, &state, &provider)?;
    let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let source_location = checkpoint_location(&app, &state, &source).ok();
    let id = Uuid::new_v4().to_string();
    let name = limit(&format!("{} (fork)", source.name), 120);
    let now = Utc::now().to_rfc3339();
    let mut record = TaskRecord {
        id: id.clone(),
        project_id: source.project_id.clone(),
        name: name.clone(),
        auto_title_eligible: false,
        auto_title_attempt_id: None,
        workspace_path: source.workspace_path.clone(),
        worktree_path: None,
        branch: source.branch.clone(),
        uses_worktree: source.uses_worktree,
        provider_id: source.provider_id.clone(),
        model_id: source.model_id.clone(),
        thinking_level: source.thinking_level.clone(),
        session_file: None,
        status: TaskStatus::Idle,
        mode: source.mode,
        archived: false,
        last_error: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let git_root = project.as_ref().and_then(|project| project.git_root.clone());
    let outcome = async {
        // The files to copy: as they were after that turn, or as they are now at the tip.
        let tree = match (&input.checkpoint, &source_location) {
            (Some(checkpoint), _) => Some(checkpoint.id.clone()),
            (None, Some(location)) if source.project_id.is_none() || source.uses_worktree => {
                snapshot_quietly(&app, &source.id, location).await.map(|checkpoint| checkpoint.id)
            }
            _ => None,
        };
        let fork_store = checkpoints::shadow_dir(&app_data, &id);
        if let Some(location) = &source_location {
            let (from, to) = (location.shadow.clone(), fork_store.clone());
            blocking(move || checkpoints::copy_shadow(&from, &to)).await?;
        }
        let mut materialize_into = None;
        if source.uses_worktree {
            let project = project.as_ref().ok_or_else(|| "This chat's project no longer exists".to_string())?;
            let git_root = project.git_root.as_deref().ok_or_else(|| "This project is not inside a Git repository".to_string())?;
            let destination = app_data.join("worktrees").join(&id);
            let branch_name = format!("wackcode/{}-{}", slug(&name), &id[..8]);
            let base = input.checkpoint.as_ref().and_then(|checkpoint| checkpoint.head.clone())
                .filter(|head| git::has_commit(Path::new(git_root), head))
                .or_else(|| source.worktree_path.as_deref().and_then(|path| git::head_commit(Path::new(path))))
                .unwrap_or_else(|| "HEAD".into());
            record.worktree_path = Some(destination.to_string_lossy().into_owned());
            let workspace = git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name, &base)?;
            record.workspace_path = workspace.to_string_lossy().into_owned();
            record.branch = Some(branch_name);
            materialize_into = Some(destination);
        } else if source.project_id.is_none() {
            let scratch = scratch_dir(&app, &id)?;
            std::fs::create_dir_all(&scratch).map_err(|error| format!("Could not create scratch folder: {error}"))?;
            record.workspace_path = scratch.to_string_lossy().into_owned();
            materialize_into = Some(scratch);
        }
        if let (Some(target), Some(tree)) = (materialize_into, tree) {
            let store = fork_store.clone();
            blocking(move || checkpoints::materialize(&store, &tree, &target).map(|_| ())).await?;
        }
        state.mutate(|data| { data.tasks.push(record.clone()); Ok(()) })?;
        worker::ensure_worker_with(&app, &record, &provider, api_key.as_deref(), WorkerOptions {
            fork_from: Some(json!({ "sessionFile": session_file, "entryId": input.entry_id })),
            wait_ready: true,
        }).await?;
        find_task(&state, &id)
    }.await;
    if outcome.is_err() {
        let _ = worker::terminate_worker(&app, &id, false).await;
        let _ = state.mutate(|data| { data.tasks.retain(|task| task.id != id); Ok(()) });
        cleanup_task_files(&app, &record, git_root.as_deref());
    }
    outcome
}

/// Switch a task between Build and Plan mode. Persisted immediately and pushed to the worker
/// when one is running; otherwise the mode rides along on `init`/`prompt` next time.
#[tauri::command]
pub async fn set_task_mode(app: AppHandle, state: State<'_, MetadataState>, input: SetTaskModeInput) -> Result<TaskRecord, String> {
    let task = state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == input.task_id)
            .ok_or_else(|| "Task not found".to_string())?;
        if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
            return Err("Wait for this task to stop before changing modes".into());
        }
        task.mode = input.mode;
        task.updated_at = Utc::now().to_rfc3339();
        Ok(task.clone())
    })?;
    // No worker is fine — `init` and `prompt` both carry the record's mode.
    let _ = worker::send(&app, &task.id, &json!({
        "id": Uuid::new_v4().to_string(), "type": "set_mode", "mode": task.mode
    })).await;
    Ok(task)
}

/// Save the proposed plan as `PLAN.md` in the task workspace. Refuses to overwrite: the file
/// may already contain something the user cares about.
#[tauri::command]
pub async fn export_plan(state: State<'_, MetadataState>, input: ExportPlanInput) -> Result<String, String> {
    let content = required(&input.content, "Plan")?;
    let workspace = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        data.tasks.iter().find(|task| task.id == input.task_id)
            .map(|task| task.workspace_path.clone())
            .ok_or_else(|| "Task not found".to_string())?
    };
    export_plan_to_workspace(&workspace, &content)
}

fn export_plan_to_workspace(workspace: &str, content: &str) -> Result<String, String> {
    let path = std::path::Path::new(workspace).join("PLAN.md");
    if path.exists() {
        return Err("PLAN.md already exists in this workspace. Rename or remove it first.".into());
    }
    std::fs::write(&path, format!("{content}\n")).map_err(|error| format!("Could not write PLAN.md: {error}"))?;
    Ok(path.to_string_lossy().to_string())
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
pub fn unarchive_task(state: State<'_, MetadataState>, task_id: String) -> Result<TaskRecord, String> {
    state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == task_id).ok_or_else(|| "Task not found".to_string())?;
        task.archived = false;
        task.updated_at = Utc::now().to_rfc3339();
        Ok(task.clone())
    })
}

#[tauri::command]
pub fn rename_task(state: State<'_, MetadataState>, task_id: String, name: String) -> Result<TaskRecord, String> {
    let name = required(&name, "Chat name")?;
    state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == task_id).ok_or_else(|| "Chat not found".to_string())?;
        task.name = name.clone();
        task.auto_title_attempt_id = None;
        task.updated_at = Utc::now().to_rfc3339();
        Ok(task.clone())
    })
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<(), String> {
    let lock = task_lock(&app, &task_id);
    let _guard = lock.lock().await;
    worker::terminate_worker(&app, &task_id, true).await?;
    let (task, git_root) = state.mutate(|data| {
        let index = data.tasks.iter().position(|task| task.id == task_id).ok_or_else(|| "Chat not found".to_string())?;
        let task = data.tasks.remove(index);
        let git_root = task.project_id.as_deref()
            .and_then(|project_id| data.projects.iter().find(|project| project.id == project_id))
            .and_then(|project| project.git_root.clone());
        Ok((task, git_root))
    })?;
    cleanup_task_files(&app, &task, git_root.as_deref());
    Ok(())
}

#[tauri::command]
pub async fn convert_task_to_worktree(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<TaskRecord, String> {
    let (task, project) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let task = data.tasks.iter().find(|task| task.id == task_id).cloned().ok_or_else(|| "Chat not found".to_string())?;
        let project = task.project_id.as_deref()
            .and_then(|project_id| data.projects.iter().find(|project| project.id == project_id)).cloned()
            .ok_or_else(|| "This chat has no project".to_string())?;
        (task, project)
    };
    if task.uses_worktree { return Ok(task); }
    if task.session_file.is_some() {
        return Err("Worktrees can only be enabled before the first message".into());
    }
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
        return Err("Wait for this chat to stop before moving it".into());
    }
    if !project.git_has_head {
        return Err("Worktrees require a Git repository with at least one commit".into());
    }
    let git_root = project.git_root.as_deref().ok_or_else(|| "This project is not inside a Git repository".to_string())?;
    let destination = app.path().app_data_dir().map_err(|error| error.to_string())?
        .join("worktrees").join(&task.id);
    let branch_name = format!("wackcode/{}-{}", slug(&task.name), &task.id[..8]);
    let workspace = git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name, "HEAD")?;
    let updated = state.mutate(|data| {
        let record = data.tasks.iter_mut().find(|item| item.id == task_id).ok_or_else(|| "Chat not found".to_string())?;
        record.workspace_path = workspace.to_string_lossy().into_owned();
        record.worktree_path = Some(destination.to_string_lossy().into_owned());
        record.branch = Some(branch_name.clone());
        record.uses_worktree = true;
        record.updated_at = Utc::now().to_rfc3339();
        Ok(record.clone())
    })?;
    worker::terminate_worker(&app, &task_id, true).await?;
    Ok(updated)
}

#[tauri::command]
pub async fn remove_project(app: AppHandle, state: State<'_, MetadataState>, project_id: String) -> Result<(), String> {
    let tasks: Vec<TaskRecord> = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        if data.tasks.iter().any(|task| task.project_id.as_deref() == Some(project_id.as_str()) && !task.archived) {
            return Err("This project still has chats. Delete or archive them first.".into());
        }
        if !data.projects.iter().any(|project| project.id == project_id) {
            return Err("Project not found".into());
        }
        data.tasks.iter().filter(|task| task.project_id.as_deref() == Some(project_id.as_str())).cloned().collect()
    };
    for task in &tasks { worker::terminate_worker(&app, &task.id, true).await?; }
    let (git_root, removed) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let git_root = data.projects.iter().find(|project| project.id == project_id)
            .and_then(|project| project.git_root.clone());
        (git_root, tasks.iter().map(|task| task.id.clone()).collect::<HashSet<_>>())
    };
    state.mutate(|data| {
        data.projects.retain(|project| project.id != project_id);
        data.tasks.retain(|task| !removed.contains(&task.id));
        Ok(())
    })?;
    for task in &tasks { cleanup_task_files(&app, task, git_root.as_deref()); }
    Ok(())
}

fn scratch_dir(app: &AppHandle, task_id: &str) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|error| error.to_string())?.join("scratch").join(task_id))
}

fn cleanup_task_files(app: &AppHandle, task: &TaskRecord, git_root: Option<&str>) {
    if let Ok(app_data) = app.path().app_data_dir() {
        let _ = std::fs::remove_dir_all(app_data.join("agent").join(&task.id));
        let _ = std::fs::remove_dir_all(app_data.join("sessions").join(&task.id));
        let _ = std::fs::remove_dir_all(app_data.join("scratch").join(&task.id));
        let _ = std::fs::remove_dir_all(checkpoints::shadow_dir(&app_data, &task.id));
    }
    if let Some(worktree_path) = task.worktree_path.as_deref() {
        let repo = git_root.map(Path::new).unwrap_or_else(|| Path::new(&task.workspace_path));
        let _ = git::remove_worktree(repo, Path::new(worktree_path));
    }
}

#[tauri::command]
pub fn git_changes(state: State<'_, MetadataState>, task_id: String) -> Result<GitChanges, String> {
    let workspace = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == task_id).map(|task| task.workspace_path.clone())
        .ok_or_else(|| "Task not found".to_string())?;
    git::changes(Path::new(&workspace))
}

/// The files `@` mentions can pick from: a chat's workspace, or a draft's project folder.
#[tauri::command]
pub async fn list_workspace_files(state: State<'_, MetadataState>, task_id: Option<String>, project_id: Option<String>) -> Result<WorkspaceFiles, String> {
    let root = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        if let Some(task_id) = task_id {
            data.tasks.iter().find(|task| task.id == task_id).map(|task| task.workspace_path.clone())
                .ok_or_else(|| "Chat not found".to_string())?
        } else if let Some(project_id) = project_id {
            data.projects.iter().find(|project| project.id == project_id).map(|project| project.path.clone())
                .ok_or_else(|| "Project not found".to_string())?
        } else {
            return Err("Pick a project to mention its files.".into());
        }
    };
    tauri::async_runtime::spawn_blocking(move || files::list(Path::new(&root))).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn reveal_task(state: State<'_, MetadataState>, task_id: String) -> Result<(), String> {
    let workspace = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == task_id).map(|task| task.workspace_path.clone())
        .ok_or_else(|| "Task not found".to_string())?;
    let status = Command::new("open").arg(&workspace).status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("macOS could not reveal this workspace".into()) }
}

#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let status = Command::new("open").arg(&path).status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("macOS could not reveal that path".into()) }
}

fn credential_for(app: &AppHandle, state: &State<'_, MetadataState>, provider: &ProviderRecord) -> Result<Option<String>, String> {
    if provider.kind == ProviderKind::Subscription {
        if !subscriptions::has_credential(app, &provider.id) {
            return Err("Sign in to this subscription again in Settings".into());
        }
        Ok(None)
    } else {
        state.secrets.get(&provider.id).map(Some)
    }
}

fn find_task(state: &State<'_, MetadataState>, task_id: &str) -> Result<TaskRecord, String> {
    state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().find(|task| task.id == task_id).cloned()
        .ok_or_else(|| "Chat not found".to_string())
}

fn set_status(state: &State<'_, MetadataState>, task_id: &str, status: TaskStatus) -> Result<(), String> {
    state.mutate(|data| {
        let task = data.tasks.iter_mut().find(|task| task.id == task_id).ok_or_else(|| "Chat not found".to_string())?;
        if status == TaskStatus::Running { task.last_error = None; }
        task.status = status;
        task.updated_at = Utc::now().to_rfc3339();
        Ok(())
    })
}

fn refuse_busy(task: &TaskRecord) -> Result<(), String> {
    if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
        return Err("Wait for this chat to finish first.".into());
    }
    Ok(())
}

/// Where a chat's checkpoints live and which folder they cover.
#[derive(Clone)]
struct CheckpointLocation {
    shadow: PathBuf,
    root: PathBuf,
    app_data: PathBuf,
}

/// A worktree's root, the project's repository root (so its `.gitignore` applies as it does
/// in Git), or the chat's own folder. Scratch and non-Git folders never walk up to a parent.
fn checkpoint_root(task: &TaskRecord, project: Option<&ProjectRecord>) -> PathBuf {
    if let Some(worktree) = task.worktree_path.as_deref() { return PathBuf::from(worktree); }
    if let Some(root) = project.and_then(|project| project.git_root.as_deref()) { return PathBuf::from(root); }
    PathBuf::from(&task.workspace_path)
}

fn checkpoint_location(app: &AppHandle, state: &State<'_, MetadataState>, task: &TaskRecord) -> Result<CheckpointLocation, String> {
    let app_data = app.path().app_data_dir().map_err(|error| error.to_string())?;
    let project = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        task.project_id.as_deref().and_then(|id| data.projects.iter().find(|project| project.id == id).cloned())
    };
    Ok(CheckpointLocation {
        shadow: checkpoints::shadow_dir(&app_data, &task.id),
        root: checkpoint_root(task, project.as_ref()),
        app_data,
    })
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|error| error.to_string())?
}

/// Take a checkpoint, or say once per chat per session why none can be taken. A chat always
/// keeps working without checkpoints; only its "restore files" options go away.
async fn snapshot_quietly(app: &AppHandle, task_id: &str, location: &CheckpointLocation) -> Option<CheckpointRef> {
    let location = location.clone();
    match blocking(move || checkpoints::snapshot(&location.shadow, &location.root, &location.app_data)).await {
        Ok(checkpoint) => Some(checkpoint),
        Err(message) => {
            static NOTIFIED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
            let first = NOTIFIED.get_or_init(Mutex::default).lock()
                .map(|mut notified| notified.insert(task_id.to_string())).unwrap_or(false);
            if first {
                let _ = app.emit("worker-event", json!({ "type": "checkpoint_unavailable", "taskId": task_id, "message": message }));
            }
            None
        }
    }
}

async fn restore_files(location: &CheckpointLocation, checkpoint_id: &str, paths: Option<Vec<String>>) -> Result<RestoreResult, String> {
    let location = location.clone();
    let checkpoint_id = checkpoint_id.to_string();
    blocking(move || checkpoints::restore(&location.shadow, &location.root, &location.app_data, &checkpoint_id, paths.as_deref())).await
}

/// A restore rewrites the whole folder the checkpoints cover, so it waits for any other chat
/// working there.
fn refuse_shared_run(app: &AppHandle, state: &State<'_, MetadataState>, task: &TaskRecord, root: &Path) -> Result<(), String> {
    let others: Vec<TaskRecord> = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter()
        .filter(|other| other.id != task.id && !other.archived && matches!(other.status, TaskStatus::Running | TaskStatus::Stopping))
        .cloned().collect();
    for other in others {
        if checkpoint_location(app, state, &other).is_ok_and(|location| location.root == root) {
            return Err(format!("“{}” is working in this folder. Wait for it to finish before restoring files.", other.name));
        }
    }
    Ok(())
}

fn validate_entry_id(entry_id: &str) -> Result<(), String> {
    if entry_id.is_empty() || entry_id.len() > 64 || !entry_id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_') {
        return Err("That message could not be found in this chat.".into());
    }
    Ok(())
}

fn validate_checkpoint_id(checkpoint_id: &str) -> Result<(), String> {
    if checkpoints::valid_checkpoint_id(checkpoint_id) { Ok(()) } else { Err("That checkpoint id is not valid.".into()) }
}

fn validate_init_agents_task(project_id: Option<&str>, mode: TaskMode) -> Result<(), String> {
    if project_id.is_none() { return Err("/init needs a project chat. Start a new chat and select a project folder.".into()); }
    if mode != TaskMode::Build { return Err("Switch to Build mode before running /init.".into()); }
    Ok(())
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
    if provider.kind == ProviderKind::Subscription && !provider.connected {
        return Err("Sign in to this subscription again in Settings".into());
    }
    validate_thinking(thinking_level)?;
    validate_selected_model(provider, model_id)?;
    let model = provider.models.iter().find(|model| model.id == model_id)
        .ok_or_else(|| "The selected model is no longer configured".to_string())?;
    if !model.thinking_levels.iter().any(|level| level == thinking_level) {
        return Err(format!("The selected model does not support {thinking_level} reasoning"));
    }
    Ok(())
}

/// Pi would quietly swap images for an "image omitted" placeholder on a text-only model, so an
/// attachment the model can never see is refused up front instead.
fn require_vision(provider: &ProviderRecord, model_id: &str) -> Result<(), String> {
    let model = provider.models.iter().find(|model| model.id == model_id)
        .ok_or_else(|| "The selected model is no longer configured".to_string())?;
    if model.vision { Ok(()) } else {
        Err("This model doesn't accept images. Turn on Vision for it in Settings, or remove the attachments.".into())
    }
}

/// Checks attachments without decoding them: the worker resizes and re-encodes each one with
/// Pi's own image pipeline, so this only has to keep oversized or non-image payloads out.
fn validate_images(images: &[ImageContent]) -> Result<(), String> {
    if images.len() > MAX_PROMPT_IMAGES {
        return Err(format!("Attach at most {MAX_PROMPT_IMAGES} images to one message."));
    }
    for image in images {
        if image.kind != "image" { return Err("An attachment is not an image.".into()); }
        if !IMAGE_MIME_TYPES.contains(&image.mime_type.as_str()) {
            return Err(format!("Images must be PNG, JPEG, GIF, or WebP (got {}).", limit(&image.mime_type, 40)));
        }
        let data = image.data.trim_end_matches('=');
        if data.is_empty() || !data.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/') {
            return Err("An attached image could not be read.".into());
        }
        if data.len() / 4 * 3 > MAX_IMAGE_BYTES {
            return Err(format!("Each image must be {} MB or smaller.", MAX_IMAGE_BYTES / 1024 / 1024));
        }
    }
    Ok(())
}

/// Accepts the source forms `pi install` accepts. Traversal is rejected outright: a package
/// source arrives from the UI, and a relative escape has no legitimate use here.
fn validate_package_source(source: &str) -> Result<String, String> {
    let source = required(source, "Package source")?;
    if source.contains("..") {
        return Err("A package source cannot contain \"..\".".into());
    }
    let recognised = source.starts_with("npm:")
        || source.starts_with("git:")
        || source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("ssh://")
        || source.starts_with("git://")
        || source.starts_with('/');
    if !recognised {
        return Err("Enter a package source such as npm:pi-web-access, git:github.com/user/repo, or an absolute path.".into());
    }
    Ok(source)
}

/// Which resources load is fixed when a worker spawns, so package changes are refused while a
/// chat is mid-run rather than silently applying on the next restart.
fn refuse_while_busy(state: &State<'_, MetadataState>) -> Result<(), String> {
    let busy = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?
        .tasks.iter().any(|task| matches!(task.status, TaskStatus::Running | TaskStatus::Stopping));
    if busy {
        return Err("Wait for running chats to finish before changing installed packages.".into());
    }
    Ok(())
}

fn validate_tool_names(names: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for name in names {
        let name = required(name, "Tool name")?;
        if seen.insert(name.clone()) { result.push(name); }
    }
    result.sort();
    Ok(result)
}

fn validate_thinking(level: &str) -> Result<(), String> {
    if THINKING_LEVELS.contains(&level) { Ok(()) } else { Err(format!("Unsupported reasoning effort: {level}")) }
}

/// Same size budget as a sub-agent's instructions: generous for a prompt, small for `wackcode.json`.
const MAX_PROMPT_OVERRIDE_CHARS: usize = 20_000;

/// Trim one override; blank means "back to the shipped default" and is stored as absent.
fn normalize_override(label: &str, value: Option<String>) -> Result<Option<String>, String> {
    let value = value.map(|text| text.trim().to_string()).filter(|text| !text.is_empty());
    if let Some(text) = &value {
        if text.chars().count() > MAX_PROMPT_OVERRIDE_CHARS {
            return Err(format!("Keep the {label} under {MAX_PROMPT_OVERRIDE_CHARS} characters."));
        }
    }
    Ok(value)
}

fn validate_prompt_config(input: PromptConfig) -> Result<PromptConfig, String> {
    Ok(PromptConfig {
        system_prompt: normalize_override("default system prompt", input.system_prompt)?,
        plan_prompt: normalize_override("Plan mode prompt", input.plan_prompt)?,
        ultra_plan_prompt: normalize_override("Ultra Plan prompt", input.ultra_plan_prompt)?,
    })
}

/// The background image is not the renderer's to set: it keeps the stored file, which only
/// `choose_background_image` and `remove_background_image` change.
fn validate_appearance_config(input: AppearanceConfig, current: &AppearanceConfig, glass_supported: bool) -> Result<AppearanceConfig, String> {
    let colour = |label: &str, value: Option<String>| -> Result<Option<String>, String> {
        match value {
            None => Ok(None),
            Some(value) => glass::parse_hex(&value.to_ascii_lowercase())
                .map(|_| Some(value.to_ascii_lowercase()))
                .ok_or_else(|| format!("The {label} colour must look like #1a2b3c.")),
        }
    };
    let percent = |label: &str, value: u8| -> Result<u8, String> {
        if value <= 90 { Ok(value) } else { Err(format!("{label} must be between 0 and 90%.")) }
    };
    let config = AppearanceConfig {
        thinking_preview: input.thinking_preview,
        accent: colour("accent", input.accent)?,
        background: colour("background", input.background)?,
        backdrop: input.backdrop,
        background_image: current.background_image.clone(),
        image_dim: percent("Image dimming", input.image_dim)?,
        image_blur: if input.image_blur <= 40 { input.image_blur } else { return Err("Image blur must be between 0 and 40 px.".into()) },
        glass_style: input.glass_style,
        glass_tint: percent("Glass tint", input.glass_tint)?,
    };
    if config.backdrop == BackdropMode::Glass && !glass_supported {
        return Err("Liquid Glass needs macOS 26 or later.".into());
    }
    if config.backdrop == BackdropMode::Image && config.background_image.is_none() {
        return Err("Choose an image first.".into());
    }
    Ok(config)
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
    fn title_eligibility_is_consumed_once_even_while_disabled() {
        let json = r#"{"id":"t","projectId":null,"name":"New chat","workspacePath":"/tmp","worktreePath":null,"branch":null,"usesWorktree":false,"providerId":"p","modelId":"m","thinkingLevel":"off","sessionFile":null,"status":"idle","archived":false,"lastError":null,"createdAt":"c","updatedAt":"u"}"#;
        let mut legacy: TaskRecord = serde_json::from_str(json).unwrap();
        assert!(consume_auto_title_eligibility(&mut legacy, true).is_none());
        let mut task = legacy.clone();
        task.auto_title_eligible = true;
        assert!(consume_auto_title_eligibility(&mut task, false).is_none());
        assert!(!task.auto_title_eligible);
        assert!(consume_auto_title_eligibility(&mut task, true).is_none());
        task.auto_title_eligible = true;
        let attempt = consume_auto_title_eligibility(&mut task, true).unwrap();
        assert_eq!(task.auto_title_attempt_id.as_deref(), Some(attempt.as_str()));
        assert!(consume_auto_title_eligibility(&mut task, true).is_none());
    }

    #[test]
    fn appearance_colours_are_normalised_and_checked() {
        let current = AppearanceConfig::default();
        let input = AppearanceConfig { accent: Some("#6CC4FF".into()), background: Some("#0F1218".into()), ..current.clone() };
        let config = validate_appearance_config(input, &current, true).unwrap();
        assert_eq!((config.accent.as_deref(), config.background.as_deref()), (Some("#6cc4ff"), Some("#0f1218")));
        for bad in ["6cc4ff", "#6cc4f", "#6cc4ffaa", "blue", "#gggggg"] {
            let input = AppearanceConfig { accent: Some(bad.into()), ..current.clone() };
            assert!(validate_appearance_config(input, &current, true).unwrap_err().contains("#1a2b3c"), "{bad}");
        }
    }

    #[test]
    fn appearance_sliders_are_range_checked() {
        let current = AppearanceConfig::default();
        assert!(validate_appearance_config(AppearanceConfig { image_dim: 91, ..current.clone() }, &current, true).is_err());
        assert!(validate_appearance_config(AppearanceConfig { glass_tint: 200, ..current.clone() }, &current, true).is_err());
        assert!(validate_appearance_config(AppearanceConfig { image_blur: 41, ..current.clone() }, &current, true).is_err());
        assert!(validate_appearance_config(AppearanceConfig { image_dim: 90, image_blur: 40, glass_tint: 0, ..current.clone() }, &current, true).is_ok());
    }

    #[test]
    fn the_renderer_cannot_point_the_background_at_another_file() {
        let current = AppearanceConfig { background_image: Some("kept.png".into()), ..AppearanceConfig::default() };
        let input = AppearanceConfig { background_image: Some("../../secrets.json".into()), ..current.clone() };
        assert_eq!(validate_appearance_config(input, &current, true).unwrap().background_image.as_deref(), Some("kept.png"));
        let input = AppearanceConfig { background_image: None, ..current.clone() };
        assert_eq!(validate_appearance_config(input, &current, true).unwrap().background_image.as_deref(), Some("kept.png"));
    }

    #[test]
    fn backdrops_need_their_prerequisites() {
        let current = AppearanceConfig::default();
        let glass = AppearanceConfig { backdrop: BackdropMode::Glass, ..current.clone() };
        assert!(validate_appearance_config(glass.clone(), &current, false).unwrap_err().contains("macOS 26"));
        assert!(validate_appearance_config(glass, &current, true).is_ok());
        let image = AppearanceConfig { backdrop: BackdropMode::Image, ..current.clone() };
        assert!(validate_appearance_config(image.clone(), &current, true).unwrap_err().contains("Choose an image"));
        let with_image = AppearanceConfig { background_image: Some("a.png".into()), ..current };
        assert!(validate_appearance_config(image, &with_image, true).is_ok());
    }

    #[test]
    fn init_requires_a_project_and_build_mode() {
        assert!(validate_init_agents_task(Some("project"), TaskMode::Build).is_ok());
        assert!(validate_init_agents_task(None, TaskMode::Build).unwrap_err().contains("project"));
        assert!(validate_init_agents_task(Some("project"), TaskMode::Plan).unwrap_err().contains("Build mode"));
        assert!(validate_init_agents_task(Some("project"), TaskMode::UltraPlan).unwrap_err().contains("Build mode"));
    }

    #[test]
    fn provider_validation_allows_discovered_models_with_unknown_limits() {
        let models = vec![ModelRecord {
            id: "same/model".into(), name: "Same model".into(), context_window: None,
            max_tokens: None, reasoning: false, thinking_levels: vec!["off".into()],
            thinking_level_map: std::collections::BTreeMap::from([("off".into(), None)]),
            vision: false,
        }];
        assert!(validate_models(&models).is_ok());
    }

    fn image(mime_type: &str, data: &str) -> ImageContent {
        ImageContent { kind: "image".into(), data: data.into(), mime_type: mime_type.into() }
    }

    #[test]
    fn prompt_images_are_checked_for_count_type_encoding_and_size() {
        assert!(validate_images(&[]).is_ok());
        assert!(validate_images(&[image("image/png", "iVBORw0KGgo="), image("image/webp", "UklGRg==")]).is_ok());
        assert!(validate_images(&vec![image("image/png", "AAAA"); MAX_PROMPT_IMAGES + 1]).is_err());
        assert!(validate_images(&[image("image/svg+xml", "AAAA")]).is_err());
        assert!(validate_images(&[image("image/png", "")]).is_err());
        assert!(validate_images(&[image("image/png", "data:image/png;base64,AAAA")]).is_err());
        let mut wrong_kind = image("image/png", "AAAA");
        wrong_kind.kind = "text".into();
        assert!(validate_images(&[wrong_kind]).is_err());
        let oversized = "A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 8);
        assert!(validate_images(&[image("image/jpeg", &oversized)]).is_err());
    }

    #[test]
    fn images_are_refused_for_a_model_without_vision() {
        let model = |id: &str, vision: bool| ModelRecord {
            id: id.into(), name: id.into(), context_window: Some(8_000), max_tokens: Some(1_000),
            reasoning: false, thinking_levels: vec!["off".into()], thinking_level_map: Default::default(), vision,
        };
        let provider = ProviderRecord {
            id: "p".into(), name: "P".into(), kind: ProviderKind::Custom, base_url: "https://example.test/v1".into(),
            api_format: "openai-completions".into(), models: vec![model("sees", true), model("blind", false)],
            created_at: "now".into(), updated_at: "now".into(), has_api_key: true, connected: true,
        };
        assert!(require_vision(&provider, "sees").is_ok());
        assert!(require_vision(&provider, "blind").unwrap_err().contains("Vision"));
    }

    #[test]
    fn old_connection_records_remain_custom_connections() {
        let json = r#"{"id":"p","name":"P","baseUrl":"https://example.test/v1","apiFormat":"openai-completions","models":[],"createdAt":"now","updatedAt":"now","hasApiKey":true}"#;
        let provider: ProviderRecord = serde_json::from_str(json).unwrap();
        assert_eq!(provider.kind, ProviderKind::Custom);
        assert_eq!(provider.api_format, "openai-completions");
    }

    #[test]
    fn model_records_written_before_vision_existed_default_to_text_only() {
        let json = r#"{"id":"m","name":"M","contextWindow":8000,"maxTokens":1000,"reasoning":false,"thinkingLevels":["off"],"thinkingLevelMap":{"off":null}}"#;
        let model: ModelRecord = serde_json::from_str(json).unwrap();
        assert!(!model.vision);
    }

    #[test]
    fn package_sources_accept_every_form_pi_install_takes_and_reject_traversal() {
        for source in ["npm:pi-web-access", "npm:@scope/pkg@1.2.3", "git:github.com/u/r@v1", "https://github.com/u/r", "/abs/path"] {
            assert!(validate_package_source(source).is_ok(), "{source} should be accepted");
        }
        for source in ["", "   ", "pi-web-access", "../escape", "npm:../evil"] {
            assert!(validate_package_source(source).is_err(), "{source} should be rejected");
        }
    }

    #[test]
    fn search_queries_are_encoded_without_breaking_the_keyword_filter() {
        assert_eq!(urlencoding("keywords:pi-package mcp"), "keywords:pi-package+mcp");
        assert_eq!(urlencoding("@scope/name"), "@scope/name");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn tool_names_are_trimmed_deduplicated_and_sorted() {
        let names = vec!["write".into(), " read ".into(), "write".into(), "bash".into()];
        assert_eq!(validate_tool_names(&names).unwrap(), vec!["bash", "read", "write"]);
        assert!(validate_tool_names(&["  ".to_string()]).is_err());
    }

    #[test]
    fn task_branch_slug_is_safe() {
        assert_eq!(slug("Fix: Sidebar & Chat"), "fix-sidebar-chat");
    }

    #[test]
    fn task_records_written_before_mode_existed_default_to_build() {
        // Old wackcode.json files have no `mode`; the serde default keeps them loadable.
        let json = r#"{"id":"t","projectId":null,"name":"n","workspacePath":"/tmp","worktreePath":null,"branch":null,"usesWorktree":false,"providerId":"p","modelId":"m","thinkingLevel":"off","sessionFile":null,"status":"idle","archived":false,"lastError":null,"createdAt":"c","updatedAt":"u"}"#;
        let task: TaskRecord = serde_json::from_str(json).unwrap();
        assert_eq!(task.mode, TaskMode::Build);
        let planning = json.replace("\"archived\":false", "\"archived\":false,\"mode\":\"plan\"");
        let task: TaskRecord = serde_json::from_str(&planning).unwrap();
        assert_eq!(task.mode, TaskMode::Plan);
        let ultra = json.replace("\"archived\":false", "\"archived\":false,\"mode\":\"ultraplan\"");
        let task: TaskRecord = serde_json::from_str(&ultra).unwrap();
        assert_eq!(task.mode, TaskMode::UltraPlan);
        // The worker's `plan_state` and the frontend both spell it "ultraplan".
        assert_eq!(serde_json::to_value(TaskMode::UltraPlan).unwrap(), json!("ultraplan"));
    }

    #[test]
    fn export_plan_writes_plan_md_once_and_refuses_to_clobber() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().to_string_lossy().to_string();
        let written = export_plan_to_workspace(&workspace, "# Plan\n\n- step").unwrap();
        assert!(written.ends_with("PLAN.md"));
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "# Plan\n\n- step\n");
        let error = export_plan_to_workspace(&workspace, "different").unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        // The refusal must not have touched the original.
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "# Plan\n\n- step\n");
    }
}
