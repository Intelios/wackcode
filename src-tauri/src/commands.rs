use crate::{
    git,
    models::{
        BootstrapPayload, BuiltinModelSuggestion, CreateTaskInput, ExportPlanInput, GitChanges, ImageContent, ModelRecord, ProjectRecord,
        PromptInput, ExtensionUiResponseInput, InstallPackageInput, PackageRecord,
        PackageSearchResult, ProviderKind, ProviderRecord, SaveProviderInput, SearchPackagesInput,
        SetPackageResourcesInput, SetTaskModeInput, SetToolConfigInput, TaskMode, TaskRecord,
        TaskStatus, ToolConfig,
    },
    storage::MetadataState,
    worker::{self}, subscriptions,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashSet, path::{Path, PathBuf}, process::Command};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

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
        workspace_path = Some(git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name)?);
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
    worker::ensure_worker(&app, &task, &provider, api_key.as_deref()).await?;
    worker::send(&app, &task.id, &json!({ "id": Uuid::new_v4().to_string(), "type": "snapshot" })).await
}

#[tauri::command]
pub async fn prompt(app: AppHandle, state: State<'_, MetadataState>, input: PromptInput) -> Result<String, String> {
    let message = required(&input.message, "Message")?;
    validate_images(&input.images)?;
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
    let run_id = Uuid::new_v4().to_string();
    let started_at = input.started_at.unwrap_or_else(|| Utc::now().timestamp_millis() as u64);
    worker::send(&app, &configured.id, &json!({
        "id": Uuid::new_v4().to_string(), "type": "prompt", "runId": run_id, "message": message,
        "startedAt": started_at, "mode": configured.mode, "images": input.images
    })).await?;
    Ok(run_id)
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
        task.updated_at = Utc::now().to_rfc3339();
        Ok(task.clone())
    })
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, state: State<'_, MetadataState>, task_id: String) -> Result<(), String> {
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
    let workspace = git::create_worktree(Path::new(&project.path), Path::new(git_root), &destination, &branch_name)?;
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
