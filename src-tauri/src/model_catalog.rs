//! WackCode's metadata catalogue, independent of Pi's runtime catalogue. Listing is disk-only;
//! only the explicit refresh command downloads from SOURCE. Imported URLs are matching hints,
//! never destinations, credentials, provider configuration or request-compatibility settings.
//! Cache publication is atomic and separate from MetadataState, so it cannot affect a chat.

use crate::models::{ModelCatalog, ModelCatalogEntry, ModelCatalogProfile};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    time::Duration,
};
use tauri::Manager;

pub const SOURCE: &str = "https://models.dev/catalog.json";
const VERSION: u32 = 1;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const FORMATS: [&str; 4] = [
    "openai-completions",
    "openai-responses",
    "anthropic-messages",
    "google-generative-ai",
];
const BUNDLED: &str = include_str!("../catalog/models.json");

#[derive(Default)]
pub struct ModelCatalogState(tokio::sync::Mutex<()>);

fn invalid() -> String {
    "The model catalogue has an unsupported or invalid format.".into()
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)?
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
}

fn eligible(model: &Value) -> bool {
    matches!(
        model.get("type").and_then(Value::as_str),
        None | Some("chat")
    ) && model.get("tool_call").and_then(Value::as_bool) == Some(true)
        && ["input", "output"].iter().all(|kind| {
            model
                .pointer(&format!("/modalities/{kind}"))
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(|v| v.as_str() == Some("text")))
        })
}

fn positive(value: Option<&Value>) -> Result<Option<u64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= 1_000_000_000)
            .map(|n| (n > 0).then_some(n))
            .ok_or_else(invalid),
    }
}

/// A boolean is not a list of efforts. Keep toggle/budget-only and unfamiliar controls manual.
fn efforts(
    model: &Value,
    reasoning: bool,
) -> (Option<Vec<String>>, BTreeMap<String, Option<String>>) {
    if !reasoning {
        return (
            Some(vec!["off".into()]),
            BTreeMap::from([("off".into(), None)]),
        );
    }
    let Some(options) = model.get("reasoning_options").and_then(Value::as_array) else {
        return (None, BTreeMap::new());
    };
    let mut map = BTreeMap::new();
    let mut explicit = false;
    for option in options {
        match option.get("type").and_then(Value::as_str) {
            Some("toggle") => {
                // Pi treats null as unsupported, including off. Its toggle-aware adapters
                // interpret off themselves; a non-null map keeps the choice available.
                map.entry("off".into()).or_insert(Some("none".into()));
            }
            Some("budget_tokens") => {}
            Some("effort") => {
                let Some(values) = option.get("values").and_then(Value::as_array) else {
                    return (None, BTreeMap::new());
                };
                for value in values {
                    let Some(value) = value.as_str() else {
                        return (None, BTreeMap::new());
                    };
                    let level = if value == "none" { "off" } else { value };
                    if value == "off" || !LEVELS.contains(&level) {
                        return (None, BTreeMap::new());
                    }
                    map.insert(level.into(), Some(value.into()));
                    explicit = true;
                }
            }
            _ => return (None, BTreeMap::new()),
        }
    }
    if !explicit {
        return (None, BTreeMap::new());
    }
    (
        Some(
            LEVELS
                .iter()
                .filter(|level| map.contains_key(**level))
                .map(|level| (*level).into())
                .collect(),
        ),
        map,
    )
}

/// v1 publishes SDK hints rather than a complete protocol map. Unknown SDKs remain unmatched.
/// Native endpoint hints are code-owned; these strings are never used to send a request.
fn endpoint(provider_id: &str, provider: &Value, model: &Value) -> (Option<String>, Vec<String>) {
    let npm = model
        .pointer("/provider/npm")
        .and_then(Value::as_str)
        .or_else(|| provider.get("npm").and_then(Value::as_str));
    let format = match npm {
        Some("@ai-sdk/anthropic") => Some("anthropic-messages"),
        Some("@ai-sdk/google") => Some("google-generative-ai"),
        Some("@ai-sdk/openai") => Some("openai-responses"),
        Some(
            "@ai-sdk/openai-compatible"
            | "@openrouter/ai-sdk-provider"
            | "@ai-sdk/deepseek"
            | "@ai-sdk/groq"
            | "@ai-sdk/xai"
            | "@ai-sdk/mistral"
            | "@ai-sdk/cerebras",
        ) => Some("openai-completions"),
        _ => None,
    };
    let hint = model
        .pointer("/provider/api")
        .and_then(Value::as_str)
        .or_else(|| provider.get("api").and_then(Value::as_str))
        .or_else(|| match provider_id {
            "openai" => Some("https://api.openai.com/v1"),
            "anthropic" => Some("https://api.anthropic.com"),
            "google" => Some("https://generativelanguage.googleapis.com/v1beta"),
            _ => None,
        });
    let base = hint
        .and_then(|value| reqwest::Url::parse(value).ok())
        .filter(|url| {
            matches!(url.scheme(), "https" | "http")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .map(|url| url.to_string());
    // The first-party OpenAI API exposes both supported surfaces.
    let formats = if provider_id == "openai" {
        vec!["openai-completions".into(), "openai-responses".into()]
    } else {
        format.into_iter().map(str::to_string).collect()
    };
    (base, formats)
}

fn profile(
    provider_id: &str,
    provider: &Value,
    id: &str,
    model: &Value,
) -> Result<ModelCatalogProfile, String> {
    let reasoning = model
        .get("reasoning")
        .and_then(Value::as_bool)
        .ok_or_else(invalid)?;
    let (thinking_levels, thinking_level_map) = efforts(model, reasoning);
    let (base_url, api_formats) = endpoint(provider_id, provider, model);
    Ok(ModelCatalogProfile {
        key: format!("{provider_id}:{id}"),
        provider: provider_id.into(),
        provider_name: text(provider, "name").unwrap_or(provider_id).into(),
        id: id.into(),
        base_url,
        api_formats,
        context_window: positive(model.pointer("/limit/context"))?,
        max_tokens: positive(model.pointer("/limit/output"))?,
        reasoning,
        thinking_levels,
        thinking_level_map,
        vision: model
            .pointer("/modalities/input")
            .and_then(Value::as_array)
            .is_some_and(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|value| value == "image" || value.starts_with("image/"))
            }),
    })
}

pub fn normalize(body: &[u8], captured_at: &str) -> Result<ModelCatalog, String> {
    if body.len() > MAX_BYTES {
        return Err("The model catalogue exceeds the 16 MiB download limit.".into());
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let providers = value
        .get("providers")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let models = value
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let mut entries: BTreeMap<String, ModelCatalogEntry> = BTreeMap::new();
    for (provider_id, provider) in providers {
        let listed = provider
            .get("models")
            .and_then(Value::as_object)
            .ok_or_else(invalid)?;
        for (id, model) in listed {
            if !eligible(model) {
                continue;
            }
            let native_id = format!("{provider_id}/{id}");
            let canonical = text(model, "canonical_model_id")
                .filter(|id| models.contains_key(*id))
                .or_else(|| {
                    models
                        .contains_key(&native_id)
                        .then_some(native_id.as_str())
                });
            let key = canonical
                .map(str::to_string)
                .unwrap_or_else(|| format!("provider:{provider_id}:{id}"));
            let original = canonical
                .and_then(|id| id.split_once('/').map(|(lab, _)| lab))
                .unwrap_or(provider_id);
            let name = canonical
                .and_then(|id| models.get(id))
                .and_then(|model| text(model, "name"))
                .or_else(|| text(model, "name"))
                .ok_or_else(invalid)?;
            let entry = entries
                .entry(key.clone())
                .or_insert_with(|| ModelCatalogEntry {
                    id: key,
                    name: name.into(),
                    original_provider: original.into(),
                    profiles: Vec::new(),
                });
            entry
                .profiles
                .push(profile(provider_id, provider, id, model)?);
        }
    }
    // A model-only record is still useful, but cannot claim an endpoint or exact effort levels.
    for (id, model) in models {
        if entries.contains_key(id) || !eligible(model) {
            continue;
        }
        let Some((lab, bare_id)) = id.split_once('/') else {
            return Err(invalid());
        };
        let mut source = profile(lab, &Value::Null, bare_id, model)?;
        source.key = format!("model:{id}");
        source.base_url = None;
        source.api_formats.clear();
        entries.insert(
            id.clone(),
            ModelCatalogEntry {
                id: id.clone(),
                name: text(model, "name").ok_or_else(invalid)?.into(),
                original_provider: lab.into(),
                profiles: vec![source],
            },
        );
    }
    let catalog = ModelCatalog {
        schema_version: VERSION,
        source: SOURCE.into(),
        captured_at: captured_at.into(),
        entries: entries.into_values().collect(),
        bundled: false,
    };
    validate(&catalog)?;
    Ok(catalog)
}

fn validate(catalog: &ModelCatalog) -> Result<(), String> {
    if catalog.schema_version != VERSION
        || catalog.source != SOURCE
        || DateTime::parse_from_rfc3339(&catalog.captured_at).is_err()
        || catalog.entries.is_empty()
        || catalog.entries.len() > 20_000
    {
        return Err(invalid());
    }
    let mut ids = HashSet::new();
    let mut count = 0;
    for entry in &catalog.entries {
        if !ids.insert(&entry.id)
            || entry.id.is_empty()
            || entry.id.len() > 1100
            || entry.name.trim().is_empty()
            || entry.name.len() > 512
            || entry.original_provider.is_empty()
            || entry.profiles.is_empty()
        {
            return Err(invalid());
        }
        let mut keys = HashSet::new();
        for profile in &entry.profiles {
            count += 1;
            if count > 30_000
                || !keys.insert(&profile.key)
                || profile.id.trim().is_empty()
                || profile.id.len() > 512
                || profile.provider.is_empty()
                || profile.provider.len() > 512
                || profile.provider_name.len() > 512
                || profile
                    .context_window
                    .is_some_and(|n| n == 0 || n > 1_000_000_000)
                || profile
                    .max_tokens
                    .is_some_and(|n| n == 0 || n > 1_000_000_000)
                || profile
                    .api_formats
                    .iter()
                    .any(|f| !FORMATS.contains(&f.as_str()))
            {
                return Err(invalid());
            }
            if let Some(levels) = &profile.thinking_levels {
                let unique: HashSet<_> = levels.iter().collect();
                if levels.is_empty()
                    || unique.len() != levels.len()
                    || levels.iter().any(|l| !LEVELS.contains(&l.as_str()))
                {
                    return Err(invalid());
                }
            }
            if profile.thinking_level_map.iter().any(|(level, value)| {
                !LEVELS.contains(&level.as_str())
                    || value
                        .as_deref()
                        .is_some_and(|v| v != "none" && !LEVELS.contains(&v))
            }) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn read_snapshot(bytes: &[u8]) -> Result<ModelCatalog, String> {
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    let snapshot = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    validate(&snapshot)?;
    Ok(snapshot)
}

fn load(path: &Path) -> Result<ModelCatalog, String> {
    let mut bundled = read_snapshot(BUNDLED.as_bytes())?;
    bundled.bundled = true;
    // Check size before reading as well: a corrupt cache must not consume unbounded memory.
    let cached = fs::metadata(path)
        .ok()
        .filter(|m| m.len() <= MAX_BYTES as u64)
        .and_then(|_| fs::read(path).ok())
        .and_then(|bytes| read_snapshot(&bytes).ok());
    if let Some(mut cached) = cached {
        if DateTime::parse_from_rfc3339(&cached.captured_at).ok()
            > DateTime::parse_from_rfc3339(&bundled.captured_at).ok()
        {
            cached.bundled = false;
            return Ok(cached);
        }
    }
    Ok(bundled)
}

fn publish(path: &Path, catalog: &ModelCatalog) -> Result<(), String> {
    validate(catalog)?;
    let parent = path.parent().ok_or_else(invalid)?;
    fs::create_dir_all(parent)
        .map_err(|_| "Could not create the model catalogue cache folder.".to_string())?;
    let bytes = serde_json::to_vec_pretty(catalog).map_err(|_| invalid())?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Could not prepare the model catalogue cache.".to_string())?;
    use std::io::Write;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "Could not write the model catalogue cache.".to_string())?;
    temporary.persist(path).map_err(|_| {
        "Could not save the model catalogue cache. The previous catalogue is still available."
            .to_string()
    })?;
    Ok(())
}

async fn download(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    // A fresh client has no provider headers or cookie store. The production caller supplies
    // only SOURCE; the URL/timeout arguments let local HTTP fixtures exercise failure paths.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|_| "Could not prepare the model catalogue download.".to_string())?;
    let mut response = client.get(url).send().await.map_err(download_error)?;
    if response.status().is_redirection() {
        return Err(
            "The model catalogue download redirected. The previous catalogue is still available."
                .into(),
        );
    }
    if !response.status().is_success() {
        return Err(format!(
            "The model catalogue download failed ({}). The previous catalogue is still available.",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err("The model catalogue exceeds the 16 MiB download limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(download_error)? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("The model catalogue exceeds the 16 MiB download limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn download_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "The model catalogue download timed out. The previous catalogue is still available.".into()
    } else {
        "Could not download the model catalogue. The previous catalogue is still available.".into()
    }
}

#[tauri::command]
pub fn list_model_catalog(app: tauri::AppHandle) -> Result<ModelCatalog, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "Could not find the app data folder.".to_string())?;
    load(&dir.join("model-catalog.json"))
}

#[tauri::command]
pub async fn refresh_model_catalog(
    app: tauri::AppHandle,
    state: tauri::State<'_, ModelCatalogState>,
) -> Result<ModelCatalog, String> {
    let _guard = state.0.lock().await;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "Could not find the app data folder.".to_string())?;
    refresh(
        &dir.join("model-catalog.json"),
        SOURCE,
        Duration::from_secs(20),
    )
    .await
}

async fn refresh(path: &Path, url: &str, timeout: Duration) -> Result<ModelCatalog, String> {
    let bytes = download(url, timeout).await?;
    let catalog = normalize(&bytes, &Utc::now().to_rfc3339())?;
    publish(path, &catalog)?;
    Ok(catalog)
}

/// Explicit maintainer action only; normal builds embed the checked-in, normalized snapshot.
pub async fn update_bundle(input: Option<&Path>) -> Result<(), String> {
    let bytes = match input {
        Some(path) => {
            fs::read(path).map_err(|_| "Could not read the supplied catalogue file.".to_string())?
        }
        None => download(SOURCE, Duration::from_secs(20)).await?,
    };
    let mut catalog = normalize(&bytes, &Utc::now().to_rfc3339())?;
    catalog.bundled = true;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog/models.json");
    publish(&path, &catalog)?;
    println!(
        "Bundled {} model entries from {SOURCE}.",
        catalog.entries.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn model() -> Value {
        json!({ "name": "Model", "reasoning": true, "tool_call": true,
            "limit": { "context": 128000, "output": 16000 },
            "modalities": { "input": ["text", "image"], "output": ["text"] },
            "reasoning_options": [{ "type": "effort", "values": ["none", "low", "high", "max"] }] })
    }

    fn fixture() -> Value {
        let mut hosted = model();
        hosted["canonical_model_id"] = json!("lab/model");
        hosted["limit"]["output"] = json!(8000);
        json!({ "models": { "lab/model": model() }, "providers": {
            "lab": { "name": "Lab", "npm": "@ai-sdk/openai-compatible", "api": "https://example.test/v1", "models": { "model": model() } },
            "gateway": { "name": "Gateway", "npm": "@ai-sdk/openai-compatible", "api": "https://example.test/plan/v1", "models": { "vendor/model": hosted } }
        } })
    }

    fn snapshot(value: &Value) -> ModelCatalog {
        normalize(&serde_json::to_vec(value).unwrap(), "2099-01-01T00:00:00Z").unwrap()
    }

    #[test]
    fn groups_canonical_identity_but_preserves_host_facts_and_ids() {
        let catalog = snapshot(&fixture());
        assert_eq!(catalog.entries.len(), 1);
        let entry = &catalog.entries[0];
        assert_eq!(entry.id, "lab/model");
        assert_eq!(entry.original_provider, "lab");
        assert_eq!(entry.profiles.len(), 2);
        let native = entry.profiles.iter().find(|p| p.provider == "lab").unwrap();
        let gateway = entry
            .profiles
            .iter()
            .find(|p| p.provider == "gateway")
            .unwrap();
        assert_eq!(native.max_tokens, Some(16000));
        assert_eq!(gateway.max_tokens, Some(8000));
        assert_eq!(gateway.id, "vendor/model");
        assert_eq!(
            native.thinking_levels.as_ref().unwrap(),
            &["off", "low", "high", "max"]
        );
        assert_eq!(native.thinking_level_map["off"].as_deref(), Some("none"));
    }

    #[test]
    fn unlinked_equal_names_stay_separate_and_model_only_facts_survive() {
        let mut value = fixture();
        value["providers"]["gateway"]["models"]["vendor/model"]
            .as_object_mut()
            .unwrap()
            .remove("canonical_model_id");
        value["models"]["other/unhosted"] = model();
        let catalog = snapshot(&value);
        assert_eq!(catalog.entries.len(), 3);
        let entry = catalog
            .entries
            .iter()
            .find(|e| e.id == "other/unhosted")
            .unwrap();
        assert_eq!(entry.profiles[0].id, "unhosted");
        assert_eq!(entry.profiles[0].base_url, None);
        assert!(entry.profiles[0].api_formats.is_empty());
    }

    #[test]
    fn unknown_toggle_and_budget_controls_do_not_invent_efforts() {
        for options in [
            Value::Null,
            json!([{ "type": "toggle" }]),
            json!([{ "type": "budget_tokens", "min": 0, "max": 8000 }]),
            json!([{ "type": "effort", "values": ["high", "future"] }]),
            json!([{ "type": "effort", "values": [null, "high"] }]),
        ] {
            let mut value = model();
            value["reasoning_options"] = options;
            assert_eq!(efforts(&value, true), (None, BTreeMap::new()));
        }
        let mut toggle = model();
        toggle["reasoning_options"] =
            json!([{ "type": "toggle" }, { "type": "effort", "values": ["low", "high"] }]);
        let (levels, map) = efforts(&toggle, true);
        assert_eq!(levels.unwrap(), ["off", "low", "high"]);
        assert_eq!(map["off"].as_deref(), Some("none"));
        assert_eq!(efforts(&model(), false).0.unwrap(), ["off"]);
    }

    #[test]
    fn uses_image_modalities_not_attachments_and_keeps_missing_limits_manual() {
        let mut value = fixture();
        let native = &mut value["providers"]["lab"]["models"]["model"];
        native["attachment"] = json!(true);
        native["modalities"]["input"] = json!(["text", "pdf"]);
        native["limit"] = json!({ "context": 0 });
        let catalog = snapshot(&value);
        let native = catalog.entries[0]
            .profiles
            .iter()
            .find(|p| p.provider == "lab")
            .unwrap();
        assert!(!native.vision);
        assert_eq!(native.context_window, None);
        assert_eq!(native.max_tokens, None);
        value["providers"]["lab"]["models"]["model"]["modalities"]["input"] =
            json!(["text", "image/png"]);
        assert!(
            snapshot(&value).entries[0]
                .profiles
                .iter()
                .find(|p| p.provider == "lab")
                .unwrap()
                .vision
        );
    }

    #[test]
    fn strips_request_configuration_and_tracks_per_model_protocol_hints() {
        let mut value = fixture();
        value["providers"]["lab"]["env"] = json!(["NEVER_COPY_KEY"]);
        let native = &mut value["providers"]["lab"]["models"]["model"];
        native["headers"] = json!({ "Authorization": "never-copy" });
        native["compat"] = json!({ "supportsStore": true });
        native["provider"] =
            json!({ "npm": "@ai-sdk/anthropic", "api": "https://example.test/messages" });
        let catalog = snapshot(&value);
        let serialized = serde_json::to_string(&catalog).unwrap();
        for forbidden in [
            "NEVER_COPY_KEY",
            "never-copy",
            "Authorization",
            "supportsStore",
        ] {
            assert!(!serialized.contains(forbidden));
        }
        let native = catalog.entries[0]
            .profiles
            .iter()
            .find(|p| p.provider == "lab")
            .unwrap();
        assert_eq!(native.api_formats, ["anthropic-messages"]);
        assert_eq!(
            native.base_url.as_deref(),
            Some("https://example.test/messages")
        );
    }

    #[test]
    fn validates_envelope_values_and_excludes_non_agent_models() {
        for body in [
            b"not json".as_slice(),
            b"{}",
            b"{\"providers\":{},\"models\":{}}",
        ] {
            assert!(normalize(body, "2099-01-01T00:00:00Z").is_err());
        }
        let mut value = fixture();
        value["providers"]["lab"]["models"]["model"]["limit"]["context"] = json!(-1);
        assert!(normalize(&serde_json::to_vec(&value).unwrap(), "2099-01-01T00:00:00Z").is_err());
        value = fixture();
        value["providers"]["lab"]["models"]["embedding"] =
            json!({ "name": "Embedding", "type": "embedding", "tool_call": false });
        assert_eq!(snapshot(&value).entries.len(), 1);
    }

    #[test]
    fn offline_load_uses_newest_valid_snapshot_and_recovers_bad_caches() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-catalog.json");
        assert!(load(&path).unwrap().bundled);
        let catalog = snapshot(&fixture());
        publish(&path, &catalog).unwrap();
        assert_eq!(load(&path).unwrap(), catalog);
        fs::write(&path, b"broken").unwrap();
        assert!(load(&path).unwrap().bundled);
        let mut older = catalog.clone();
        older.captured_at = "2000-01-01T00:00:00Z".into();
        publish(&path, &older).unwrap();
        assert!(load(&path).unwrap().bundled);
        let mut incompatible = catalog.clone();
        incompatible.schema_version = VERSION + 1;
        fs::write(&path, serde_json::to_vec(&incompatible).unwrap()).unwrap();
        assert!(load(&path).unwrap().bundled);
        let mut bad = catalog;
        bad.entries.clear();
        let previous = fs::read(&path).unwrap();
        assert!(publish(&path, &bad).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
    }

    fn server(response: Vec<u8>, delay: Duration) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/catalog", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            thread::sleep(delay);
            let _ = socket.write_all(&response);
            String::from_utf8_lossy(&buffer[..n]).into_owned()
        });
        (url, handle)
    }

    fn response(body: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn refresh_publishes_only_valid_metadata_and_preserves_unrelated_app_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-catalog.json");
        let settings = temp.path().join("wackcode.json");
        fs::write(&settings, "saved models and running chats").unwrap();
        let (url, handle) = server(
            response(&serde_json::to_vec(&fixture()).unwrap()),
            Duration::ZERO,
        );
        let catalog = runtime()
            .block_on(refresh(&path, &url, Duration::from_secs(5)))
            .unwrap();
        assert_eq!(read_snapshot(&fs::read(&path).unwrap()).unwrap(), catalog);
        let request = handle.join().unwrap().to_lowercase();
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("cookie:"));
        let previous = fs::read(&path).unwrap();
        let (url, handle) = server(response(b"{\"changed-format\":true}"), Duration::ZERO);
        assert!(runtime()
            .block_on(refresh(&path, &url, Duration::from_secs(5)))
            .is_err());
        handle.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(
            fs::read_to_string(&settings).unwrap(),
            "saved models and running chats"
        );
    }

    #[test]
    fn downloads_refuse_redirects_errors_timeouts_and_oversized_bodies() {
        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BYTES + 1
        )
        .into_bytes();
        for (bytes, delay, timeout, message) in [
            (b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/must-not-follow\r\nContent-Length: 0\r\n\r\n".to_vec(), Duration::ZERO, Duration::from_secs(5), "redirected"),
            (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec(), Duration::ZERO, Duration::from_secs(5), "503"),
            (oversized, Duration::ZERO, Duration::from_secs(5), "16 MiB"),
            (response(b"{}"), Duration::from_millis(300), Duration::from_millis(100), "timed out"),
        ] {
            let (url, handle) = server(bytes, delay);
            assert!(runtime().block_on(download(&url, timeout)).unwrap_err().contains(message));
            handle.join().unwrap();
        }
        // Enforce the bound while streaming too, rather than trusting Content-Length.
        let mut chunked = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n",
            MAX_BYTES + 1
        )
        .into_bytes();
        chunked.resize(chunked.len() + MAX_BYTES + 1, b'x');
        chunked.extend_from_slice(b"\r\n0\r\n\r\n");
        let (url, handle) = server(chunked, Duration::ZERO);
        assert!(runtime()
            .block_on(download(&url, Duration::from_secs(5)))
            .unwrap_err()
            .contains("16 MiB"));
        handle.join().unwrap();
    }
}
