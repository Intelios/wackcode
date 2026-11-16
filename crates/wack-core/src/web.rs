//! Settings › Web: whether the agent may read pages and search, the search providers, and which
//! one is active. The tools themselves are `wack_web`'s; this is what the store keeps and what a
//! run is told. Ported from version 2's `web.rs` commands, with its validation and tests.
//!
//! Invariants:
//! - A search key lives in `secrets.json` (`search:<id>`) and leaves it only inside a
//!   [`SearchClient`] built here: no function returns a key. (Version 2's "reveal" button is
//!   gone for that reason: a key is typed once, like a connection's.)
//! - A linked key is a Settings › Providers connection's own key, used only when that connection
//!   is switched on and on the preset's own origin, so it never reaches another host.
//! - A provider that sends a key does so only over https, except to this Mac.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use url::Url;
use wack_web::{
    SearchClient,
    search::{CustomSpec, HttpMethod, KeySource, SearchKind, SearchProvider, needs_key, preset, render_body_template, secret_id, supports_fetch, validate_json_path},
};

use crate::{
    store::{Connection, Store},
    web_usage::{Counters, Recorder},
};

pub use wack_web::search;

pub const MAX_PROVIDERS: usize = 16;

/// What `state.json` keeps for Settings › Web.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebConfig {
    /// Web Fetch is on unless the user turns it off, as in version 2.
    #[serde(default = "on")]
    pub fetch: bool,
    /// The Web Search switch: `web_search` is offered only while this is on.
    #[serde(default)]
    pub search: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<SearchProvider>,
    /// The one provider every search (and, with `route_fetch`, every page read) goes to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Read every page through the active provider's own fetch endpoint.
    #[serde(default)]
    pub route_fetch: bool,
}

fn on() -> bool {
    true
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { fetch: true, search: false, providers: Vec::new(), active: None, route_fetch: false }
    }
}

/// A provider as Settings › Web is about to save it (or test it).
#[derive(Clone, Debug, PartialEq)]
pub struct SearchDraft {
    /// The provider being edited; `None` for a new one.
    pub id: Option<String>,
    pub name: String,
    pub kind: SearchKind,
    pub key_source: KeySource,
    pub base_url: Option<String>,
    pub custom: Option<CustomSpec>,
    pub price_search_per_1k: Option<f64>,
    pub price_fetch_per_1k: Option<f64>,
}

impl SearchDraft {
    /// A new provider of `kind`, as the preset suggests it.
    pub fn new(kind: SearchKind) -> Self {
        let preset = preset(kind);
        Self {
            id: None,
            name: preset.label.to_string(),
            kind,
            key_source: if kind == SearchKind::Searxng { KeySource::None } else { KeySource::Own },
            base_url: None,
            custom: (kind == SearchKind::Custom).then(|| CustomSpec { results_path: "results".into(), title_path: "title".into(), url_path: "url".into(), ..CustomSpec::default() }),
            price_search_per_1k: preset.default_price,
            price_fetch_per_1k: if supports_fetch(kind) { preset.default_price } else { None },
        }
    }

    pub fn of(provider: &SearchProvider) -> Self {
        Self {
            id: Some(provider.id.clone()),
            name: provider.name.clone(),
            kind: provider.kind,
            key_source: provider.key_source.clone(),
            base_url: provider.base_url.clone(),
            custom: provider.custom.clone(),
            price_search_per_1k: provider.price_search_per_1k,
            price_fetch_per_1k: provider.price_fetch_per_1k,
        }
    }
}

/// One provider as Settings › Web lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderSummary {
    pub provider: SearchProvider,
    pub active: bool,
    /// Why it can't search now (no key, a switched-off connection), or `None` when it can.
    pub problem: Option<String>,
    pub usage: Counters,
}

impl Store {
    /// Settings › Web as it is saved.
    pub fn web_config(&self) -> &WebConfig {
        self.web_state()
    }

    /// Settings › Web: whether the agent may read public web pages.
    pub fn web_fetch_enabled(&self) -> bool {
        self.web_state().fetch
    }

    pub fn set_web_fetch(&mut self, enabled: bool) -> Result<(), String> {
        if self.web_state().fetch == enabled {
            return Ok(());
        }
        self.web_state_mut().fetch = enabled;
        self.save_state()
    }

    pub fn set_web_search(&mut self, enabled: bool) -> Result<(), String> {
        if self.web_state().search == enabled {
            return Ok(());
        }
        self.web_state_mut().search = enabled;
        self.save_state()
    }

    pub fn set_route_fetch(&mut self, enabled: bool) -> Result<(), String> {
        if self.web_state().route_fetch == enabled {
            return Ok(());
        }
        self.web_state_mut().route_fetch = enabled;
        self.save_state()
    }

    pub fn set_active_search_provider(&mut self, id: &str) -> Result<(), String> {
        if !self.web_state().providers.iter().any(|provider| provider.id == id) {
            return Err("That search provider no longer exists.".into());
        }
        self.web_state_mut().active = Some(id.to_string());
        self.save_state()
    }

    /// The providers with what Settings › Web shows beside each.
    pub fn search_providers(&self) -> Vec<ProviderSummary> {
        let config = self.web_state();
        config
            .providers
            .iter()
            .map(|provider| ProviderSummary {
                provider: provider.clone(),
                active: config.active.as_deref() == Some(provider.id.as_str()),
                problem: self.search_key(provider).err(),
                usage: self.web_usage_meter().this_month(&provider.id),
            })
            .collect()
    }

    /// The connections a provider of `kind` may borrow a key from: switched on, with a key, on
    /// the preset's own origin. Only DeepSeek can borrow.
    pub fn lendable_connections(&self, kind: SearchKind) -> Vec<&Connection> {
        let Some(origin) = preset(kind).link_origin else { return Vec::new() };
        self.enabled_connections().filter(|connection| same_origin(&connection.base_url, origin) && self.has_key(&connection.id)).collect()
    }

    /// Whether the provider being edited already has its own key saved.
    pub fn search_key_saved(&self, id: &str) -> bool {
        self.secret(&secret_id(id)).is_some()
    }

    /// Saves a provider and, when one is typed, its own key. A blank key keeps the saved one.
    /// The first provider saved becomes the active one. Returns its id.
    pub fn save_search_provider(&mut self, draft: &SearchDraft, key: Option<&str>) -> Result<String, String> {
        let key = clean_key(key)?;
        let existing = draft.id.as_deref().and_then(|id| self.web_state().providers.iter().find(|provider| provider.id == id)).cloned();
        if existing.is_none() && self.web_state().providers.len() >= MAX_PROVIDERS {
            return Err(format!("Settings › Web keeps at most {MAX_PROVIDERS} search providers."));
        }
        let provider = self.prepare(draft, existing.as_ref())?;
        if provider.key_source == KeySource::Own && key.is_none() && !self.search_key_saved(&provider.id) {
            return Err(format!("Add an API key for {}.", provider.name));
        }
        match (&provider.key_source, key) {
            (KeySource::Own, Some(key)) => self.set_secret(&secret_id(&provider.id), &key)?,
            (KeySource::Own, None) => {}
            // A provider that no longer uses its own key lets it go.
            _ => self.remove_secret(&secret_id(&provider.id))?,
        }
        let id = provider.id.clone();
        let config = self.web_state_mut();
        match config.providers.iter_mut().find(|saved| saved.id == id) {
            Some(saved) => *saved = provider,
            None => config.providers.push(provider),
        }
        if config.active.is_none() {
            config.active = Some(id.clone());
        }
        self.save_state()?;
        Ok(id)
    }

    pub fn delete_search_provider(&mut self, id: &str) -> Result<(), String> {
        let config = self.web_state_mut();
        config.providers.retain(|provider| provider.id != id);
        if config.active.as_deref() == Some(id) {
            config.active = config.providers.first().map(|provider| provider.id.clone());
        }
        self.save_state()?;
        self.remove_secret(&secret_id(id))?;
        self.web_usage_meter().reset(Some(id))
    }

    /// Clears this month's and every earlier month's counts for one provider, or all of them.
    pub fn reset_web_usage(&self, id: Option<&str>) -> Result<(), String> {
        self.web_usage_meter().reset(id)
    }

    /// The names of the search providers borrowing this connection's key.
    pub(crate) fn search_providers_borrowing(&self, connection: &str) -> Vec<String> {
        self.web_state()
            .providers
            .iter()
            .filter(|provider| matches!(&provider.key_source, KeySource::Connection { connection: id } if id == connection))
            .map(|provider| provider.name.clone())
            .collect()
    }

    /// A client for a draft, to run Settings' Test search before saving. A typed key is used as
    /// is; otherwise the saved or linked one. Not counted by the usage meter unless saved.
    pub fn draft_search_client(&self, draft: &SearchDraft, key: Option<&str>) -> Result<SearchClient, String> {
        let existing = draft.id.as_deref().and_then(|id| self.web_state().providers.iter().find(|provider| provider.id == id));
        let provider = self.prepare(draft, existing)?;
        let key = match (clean_key(key)?, &provider.key_source) {
            (Some(key), KeySource::Own) => Some(key),
            _ => self.search_key(&provider)?,
        };
        let meter = existing.map(|_| Arc::new(Recorder(self.web_usage_meter().clone())) as Arc<dyn search::Meter>);
        SearchClient::new(provider, key, meter)
    }

    /// The web tools a run offers.
    pub fn web_setup(&self) -> wack_web::WebSetup {
        let config = self.web_state();
        let search = config
            .search
            .then_some(config.active.as_deref())
            .flatten()
            .and_then(|id| config.providers.iter().find(|provider| provider.id == id))
            .and_then(|provider| {
                let key = self.search_key(provider).ok()?;
                SearchClient::new(provider.clone(), key, Some(Arc::new(Recorder(self.web_usage_meter().clone())))).ok()
            })
            .map(Arc::new);
        wack_web::WebSetup { fetch: config.fetch, search, route_fetch: config.route_fetch }
    }

    /// The key a provider sends, `None` when it needs none. The error is the user's sentence.
    fn search_key(&self, provider: &SearchProvider) -> Result<Option<String>, String> {
        let missing = || format!("Add an API key for {} in Settings › Web.", provider.name);
        match &provider.key_source {
            KeySource::None if needs_key(provider) => Err(missing()),
            KeySource::None => Ok(None),
            KeySource::Own => self.secret(&secret_id(&provider.id)).map(|key| Some(key.to_string())).ok_or_else(missing),
            KeySource::Connection { connection } => {
                let lender = self.linkable(provider.kind, connection)?;
                self.secret(&lender.id)
                    .map(|key| Some(key.to_string()))
                    .ok_or_else(|| format!("The connection ‘{}’ has no API key. Add one in Settings › Providers.", lender.name))
            }
        }
    }

    /// The connection a provider may borrow a key from, or why it can't.
    fn linkable(&self, kind: SearchKind, id: &str) -> Result<&Connection, String> {
        let preset = preset(kind);
        let origin = preset.link_origin.ok_or_else(|| format!("{} can't use a connection's key. Give it its own key.", preset.label))?;
        let connection = self
            .connections()
            .iter()
            .find(|connection| connection.id == id)
            .ok_or("The connection this provider borrows its key from no longer exists. Pick another key in Settings › Web.")?;
        if !connection.enabled {
            return Err(format!("The connection ‘{}’ is switched off. Switch it on in Settings › Providers, or give this provider its own key.", connection.name));
        }
        if !same_origin(&connection.base_url, origin) {
            return Err(format!("The connection ‘{}’ isn't on {origin}, so its key can't be used here.", connection.name));
        }
        Ok(connection)
    }

    /// Validates a draft into the provider it would store.
    fn prepare(&self, draft: &SearchDraft, existing: Option<&SearchProvider>) -> Result<SearchProvider, String> {
        let name = draft.name.trim();
        if name.is_empty() {
            return Err("Give the provider a name.".into());
        }
        if name.chars().count() > 60 {
            return Err("Keep the provider's name under 60 characters.".into());
        }
        if existing.is_some_and(|existing| existing.kind != draft.kind) {
            return Err("A provider's type can't change. Add a new provider instead.".into());
        }
        let (base_url, custom) = match draft.kind {
            SearchKind::Searxng => (Some(validate_endpoint(draft.base_url.as_deref(), "SearXNG instance")?), None),
            SearchKind::Custom => {
                let spec = draft.custom.as_ref().ok_or("Fill in how to call this search API.")?;
                (Some(validate_endpoint(draft.base_url.as_deref(), "search API")?), Some(validate_custom(spec)?))
            }
            _ => (None, None),
        };
        let mut provider = SearchProvider {
            id: existing.map(|existing| existing.id.clone()).unwrap_or_else(|| format!("search-{}", uuid::Uuid::new_v4().simple())),
            name: name.to_string(),
            kind: draft.kind,
            key_source: draft.key_source.clone(),
            base_url,
            custom,
            price_search_per_1k: validate_price(draft.price_search_per_1k)?,
            price_fetch_per_1k: validate_price(draft.price_fetch_per_1k)?,
        };
        if !needs_key(&provider) {
            provider.key_source = KeySource::None;
        } else if provider.key_source == KeySource::None {
            provider.key_source = KeySource::Own;
        }
        if let KeySource::Connection { connection } = &provider.key_source {
            self.linkable(provider.kind, connection)?;
        }
        // A key goes only over https, except to this Mac.
        if needs_key(&provider)
            && let Some(url) = &provider.base_url
            && url.starts_with("http://")
            && !is_loopback(url)
        {
            return Err("This provider sends a key, so its URL must use https:// (plain http:// is allowed only to this Mac).".into());
        }
        Ok(provider)
    }
}

pub fn same_origin(left: &str, right: &str) -> bool {
    match (Url::parse(left.trim()), Url::parse(right.trim())) {
        (Ok(left), Ok(right)) => left.origin() == right.origin(),
        _ => false,
    }
}

fn validate_endpoint(raw: Option<&str>, label: &str) -> Result<String, String> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty()).ok_or_else(|| format!("Enter the {label} URL."))?;
    let url = Url::parse(raw).map_err(|_| format!("Enter a valid {label} URL, starting with https://."))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!("The {label} URL must start with http:// or https://."));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Put credentials in the key field, not the URL.".into());
    }
    if raw.chars().count() > 2048 {
        return Err(format!("The {label} URL is too long."));
    }
    Ok(raw.trim_end_matches('/').to_string())
}

fn is_loopback(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn validate_header_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let valid = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c));
    if valid { Ok(name.to_string()) } else { Err("The auth header should be a plain header name, such as Authorization or X-API-Key.".into()) }
}

fn validate_price(price: Option<f64>) -> Result<Option<f64>, String> {
    match price {
        Some(price) if !price.is_finite() || !(0.0..=10_000.0).contains(&price) => Err("Enter a price between 0 and 10,000 per 1,000 requests.".into()),
        other => Ok(other),
    }
}

fn validate_custom(spec: &CustomSpec) -> Result<CustomSpec, String> {
    let trim = |value: &Option<String>| value.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    let auth_header = trim(&spec.auth_header).map(|name| validate_header_name(&name)).transpose()?;
    let auth_prefix = spec.auth_prefix.as_deref().filter(|prefix| !prefix.is_empty()).map(str::to_string);
    if auth_prefix.as_deref().is_some_and(|prefix| prefix.len() > 40 || prefix.chars().any(char::is_control)) {
        return Err("The key prefix should be short text such as “Bearer ”.".into());
    }
    let query_param = trim(&spec.query_param);
    let body_template = trim(&spec.body_template);
    match spec.method {
        HttpMethod::Get if query_param.is_none() => return Err("Enter the query parameter name, such as q.".into()),
        HttpMethod::Get => {}
        HttpMethod::Post => {
            let template = body_template.as_deref().ok_or("Enter a JSON body with {{query}} where the query goes.")?;
            if !template.contains("{{query}}") {
                return Err("The body template needs {{query}} where the query goes.".into());
            }
            render_body_template(template, "test", 5)?;
        }
    }
    Ok(CustomSpec {
        method: spec.method,
        auth_header,
        auth_prefix,
        query_param,
        count_param: trim(&spec.count_param),
        body_template,
        results_path: validate_json_path(&spec.results_path, "the results list", true)?,
        title_path: validate_json_path(&spec.title_path, "a result's title", false)?,
        url_path: validate_json_path(&spec.url_path, "a result's URL", false)?,
        snippet_path: trim(&spec.snippet_path).map(|path| validate_json_path(&path, "a result's snippet", false)).transpose()?,
        date_path: trim(&spec.date_path).map(|path| validate_json_path(&path, "a result's date", false)).transpose()?,
    })
}

fn clean_key(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(key) = raw.map(str::trim).filter(|key| !key.is_empty()) else { return Ok(None) };
    if key.starts_with("http://") || key.starts_with("https://") {
        return Err("That looks like a URL, not an API key. Paste the key your provider issued.".into());
    }
    if key.chars().any(char::is_control) || key.len() > 4096 {
        return Err("That API key has characters a key can't contain.".into());
    }
    Ok(Some(key.to_string()))
}

#[cfg(test)]
mod tests {
    use wack_llm::WireFormat;

    use super::*;
    use crate::store::NewConnection;

    fn store() -> (tempfile::TempDir, Store) {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(folder.path()).unwrap();
        (folder, store)
    }

    fn connection(store: &mut Store, name: &str, base_url: &str, key: &str) -> String {
        store
            .add_connection(NewConnection { name: name.into(), format: WireFormat::OpenaiChat, base_url: base_url.into(), models: vec![crate::store::Model::new("m", None)] }, key)
            .unwrap()
            .id
            .clone()
    }

    #[test]
    fn web_fetch_is_on_by_default_and_its_switch_persists() {
        let (folder, mut store) = store();
        assert!(store.web_fetch_enabled());
        assert!(store.web_setup().fetch);
        store.set_web_fetch(false).unwrap();
        let store = Store::open(folder.path()).unwrap();
        assert!(!store.web_fetch_enabled());
    }

    #[test]
    fn a_settings_file_from_before_web_loads_with_fetch_on_and_search_off() {
        let config: WebConfig = serde_json::from_str("{}").unwrap();
        assert!(config.fetch && !config.search && config.providers.is_empty());
    }

    #[test]
    fn search_is_offered_only_when_on_active_and_keyed() {
        let (_folder, mut store) = store();
        assert!(store.web_setup().search.is_none());
        let id = store.save_search_provider(&SearchDraft::new(SearchKind::Brave), Some("bk-secret-key")).unwrap();
        // The first provider saved becomes the active one, but search is still off.
        assert_eq!(store.web_config().active.as_deref(), Some(id.as_str()));
        assert!(store.web_setup().search.is_none());
        store.set_web_search(true).unwrap();
        let setup = store.web_setup();
        assert_eq!(setup.search.as_ref().map(|client| client.name()), Some("Brave Search"));
        // Nothing a run is handed shows the key.
        assert!(!format!("{setup:?}").contains("bk-secret-key"));
        // The key is in secrets.json and nowhere in state.json.
        let state = std::fs::read_to_string(store.folder().join("state.json")).unwrap();
        assert!(!state.contains("bk-secret-key"));
    }

    #[test]
    fn own_keys_are_required_kept_when_blank_and_dropped_with_the_provider() {
        let (_folder, mut store) = store();
        assert!(store.save_search_provider(&SearchDraft::new(SearchKind::Tavily), None).unwrap_err().contains("Add an API key"));
        let id = store.save_search_provider(&SearchDraft::new(SearchKind::Tavily), Some("tk-first")).unwrap();
        let mut draft = SearchDraft::of(&store.web_config().providers[0]);
        draft.name = "My Tavily".into();
        store.save_search_provider(&draft, Some("  ")).unwrap();
        assert!(store.search_key_saved(&id));
        assert_eq!(store.web_config().providers[0].name, "My Tavily");
        store.delete_search_provider(&id).unwrap();
        assert!(!store.search_key_saved(&id));
        assert!(store.web_config().active.is_none());
        assert!(clean_key(Some("https://api.deepseek.com")).is_err());
    }

    #[test]
    fn linked_keys_only_reach_their_own_origin() {
        let (_folder, mut store) = store();
        let deepseek = connection(&mut store, "DeepSeek chat", "https://api.deepseek.com/v1", "sk-linked");
        let other = connection(&mut store, "Other", "https://api.openai.com/v1", "sk-other");
        assert_eq!(store.lendable_connections(SearchKind::Deepseek).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), [deepseek.as_str()]);
        assert!(store.lendable_connections(SearchKind::Brave).is_empty());

        let mut draft = SearchDraft::new(SearchKind::Deepseek);
        draft.key_source = KeySource::Connection { connection: other };
        assert!(store.save_search_provider(&draft, None).unwrap_err().contains("isn't on https://api.deepseek.com"));
        draft.key_source = KeySource::Connection { connection: deepseek.clone() };
        store.save_search_provider(&draft, None).unwrap();
        store.set_web_search(true).unwrap();
        assert!(store.web_setup().search.is_some());

        // The lender can't be removed while it lends, and a switched-off lender lends nothing.
        assert!(store.remove_connection(&deepseek).unwrap_err().contains("web search"));
        store.set_connection_enabled(&deepseek, false).unwrap();
        assert!(store.web_setup().search.is_none());
        assert!(store.search_providers()[0].problem.as_deref().unwrap().contains("switched off"));

        let mut brave = SearchDraft::new(SearchKind::Brave);
        brave.key_source = KeySource::Connection { connection: deepseek };
        assert!(store.save_search_provider(&brave, None).unwrap_err().contains("its own key"));
    }

    #[test]
    fn validates_what_is_saved() {
        let (_folder, mut store) = store();
        let mut searxng = SearchDraft::new(SearchKind::Searxng);
        assert!(store.save_search_provider(&searxng, None).unwrap_err().contains("SearXNG instance URL"));
        searxng.base_url = Some("http://127.0.0.1:8888/".into());
        store.save_search_provider(&searxng, None).unwrap();
        let saved = &store.web_config().providers[0];
        assert_eq!((saved.base_url.as_deref(), &saved.key_source), (Some("http://127.0.0.1:8888"), &KeySource::None));

        let mut custom = SearchDraft::new(SearchKind::Custom);
        custom.base_url = Some("http://search.example.com/api".into());
        custom.custom = Some(CustomSpec {
            auth_header: Some("X-API-Key".into()),
            query_param: Some("q".into()),
            results_path: "data.items".into(),
            title_path: "name".into(),
            url_path: "link".into(),
            ..CustomSpec::default()
        });
        assert!(store.save_search_provider(&custom, Some("ck")).unwrap_err().contains("https://"));
        custom.base_url = Some("https://search.example.com/api".into());
        store.save_search_provider(&custom, Some("ck")).unwrap();
        custom.custom.as_mut().unwrap().title_path = "a..b".into();
        assert!(store.save_search_provider(&custom, Some("ck")).is_err());

        let mut post = SearchDraft::new(SearchKind::Custom);
        post.base_url = Some("https://search.example.com/api".into());
        post.custom = Some(CustomSpec { method: HttpMethod::Post, body_template: Some(r#"{"q": 1}"#.into()), title_path: "t".into(), url_path: "u".into(), ..CustomSpec::default() });
        assert!(store.save_search_provider(&post, None).unwrap_err().contains("{{query}}"));

        let mut price = SearchDraft::new(SearchKind::Deepseek);
        price.price_search_per_1k = Some(f64::NAN);
        assert!(store.save_search_provider(&price, Some("k")).is_err());
    }

    #[test]
    fn reading_through_a_provider_is_offered_with_search() {
        let (_folder, mut store) = store();
        store.save_search_provider(&SearchDraft::new(SearchKind::Deepseek), Some("sk-d")).unwrap();
        store.set_web_search(true).unwrap();
        store.set_route_fetch(true).unwrap();
        let setup = store.web_setup();
        assert!(setup.route_fetch && setup.search.is_some() && setup.fetch);
        let names: Vec<String> = wack_web::tools(&setup).unwrap().iter().map(|tool| tool.spec().name).collect();
        assert_eq!(names, [wack_web::WEB_SEARCH, wack_web::WEB_FETCH]);
    }
}
