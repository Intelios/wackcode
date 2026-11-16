//! Search providers: what Settings › Web stores for each, the requests they take and the answers
//! they give, ported from version 2's `web.rs` with its tests.
//!
//! Invariants:
//! - Only the provider's own endpoint is contacted, and redirects are refused: a key is never
//!   forwarded to another host. Keys travel only over https, except to this Mac.
//! - A key lives inside a [`SearchClient`] and nowhere else: nothing here returns it, and every
//!   error is scrubbed of it.
//! - No silent fallback: an error from the active provider is the answer.
//!
//! Building requests and reading answers are pure functions, so every preset is tested without a
//! network.

use std::{sync::Arc, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use url::Url;

pub const MAX_COUNT: u8 = 10;
pub const DEFAULT_COUNT: u8 = 5;
const MAX_QUERY_CHARS: usize = 400;
const TIMEOUT: Duration = Duration::from_secs(30);
const SEARCH_BODY_LIMIT: usize = 2 * 1024 * 1024;
const FETCH_BODY_LIMIT: usize = 6 * 1024 * 1024;
/// Matches web_fetch's own body cap.
const FETCH_CONTENT_LIMIT: usize = 5 * 1024 * 1024;
/// Parallel requests per provider, under DeepSeek's 5 per second.
const PROVIDER_CONCURRENCY: usize = 4;
const USER_AGENT: &str = "WackCode-web/1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    Deepseek,
    Brave,
    Tavily,
    Searxng,
    Custom,
}

impl SearchKind {
    pub const ALL: [SearchKind; 5] = [SearchKind::Deepseek, SearchKind::Brave, SearchKind::Tavily, SearchKind::Searxng, SearchKind::Custom];
}

/// Where a provider's key comes from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum KeySource {
    /// No key: a SearXNG instance, or a Custom endpoint without auth.
    #[default]
    None,
    /// Its own key, kept in `secrets.json` as `search:<id>`.
    Own,
    /// The key of a Settings › Providers connection on the same origin.
    Connection { connection: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
}

/// How to call a search API that isn't a preset, and where its answer keeps the results.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CustomSpec {
    #[serde(default)]
    pub method: HttpMethod,
    /// The header the key goes in, such as `Authorization` or `X-API-Key`. None sends no key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    /// Text before the key in that header, such as `Bearer `.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_prefix: Option<String>,
    /// GET: the query-string parameter that carries the query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_param: Option<String>,
    /// POST: a JSON body with `{{query}}` and `{{count}}` placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_template: Option<String>,
    /// Dotted paths into the answer; an empty results path is the answer itself.
    #[serde(default)]
    pub results_path: String,
    #[serde(default)]
    pub title_path: String,
    #[serde(default)]
    pub url_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_path: Option<String>,
}

/// One search provider as Settings › Web saves it. Never holds a key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchProvider {
    pub id: String,
    pub name: String,
    pub kind: SearchKind,
    #[serde(default)]
    pub key_source: KeySource,
    /// The endpoint for SearXNG (the instance) and Custom (the search URL). Presets ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomSpec>,
    /// The user's price per 1,000 billed requests, for the usage meter's estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_search_per_1k: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_fetch_per_1k: Option<f64>,
}

pub struct Preset {
    pub label: &'static str,
    /// Fixed search endpoint; `None` means the provider's own `base_url` (SearXNG, Custom).
    pub search_url: Option<&'static str>,
    pub fetch_url: Option<&'static str>,
    /// A Settings › Providers connection on this origin may lend its key.
    pub link_origin: Option<&'static str>,
    pub max_count: u8,
    /// The provider's own query limit, counted after operators are added.
    pub max_query_chars: usize,
    /// The price per 1,000 billed requests a new provider of this kind starts with.
    pub default_price: Option<f64>,
}

pub fn preset(kind: SearchKind) -> &'static Preset {
    match kind {
        SearchKind::Deepseek => &Preset {
            label: "DeepSeek",
            search_url: Some("https://api.deepseek.com/v1/tools/search"),
            fetch_url: Some("https://api.deepseek.com/v1/tools/fetch"),
            link_origin: Some("https://api.deepseek.com"),
            max_count: 50,
            max_query_chars: 128,
            default_price: Some(0.75),
        },
        SearchKind::Brave => &Preset {
            label: "Brave Search",
            search_url: Some("https://api.search.brave.com/res/v1/web/search"),
            fetch_url: None,
            link_origin: None,
            max_count: 20,
            max_query_chars: 400,
            default_price: None,
        },
        SearchKind::Tavily => &Preset {
            label: "Tavily",
            search_url: Some("https://api.tavily.com/search"),
            fetch_url: Some("https://api.tavily.com/extract"),
            link_origin: None,
            max_count: 20,
            max_query_chars: 400,
            default_price: None,
        },
        SearchKind::Searxng => &Preset {
            label: "SearXNG",
            search_url: None,
            fetch_url: None,
            link_origin: None,
            max_count: MAX_COUNT,
            max_query_chars: 400,
            default_price: None,
        },
        SearchKind::Custom => &Preset {
            label: "Custom",
            search_url: None,
            fetch_url: None,
            link_origin: None,
            max_count: MAX_COUNT,
            max_query_chars: 400,
            default_price: None,
        },
    }
}

/// Whether this provider can't search without a key.
pub fn needs_key(provider: &SearchProvider) -> bool {
    match provider.kind {
        SearchKind::Deepseek | SearchKind::Brave | SearchKind::Tavily => true,
        SearchKind::Searxng => false,
        SearchKind::Custom => provider.custom.as_ref().is_some_and(|spec| spec.auth_header.is_some()),
    }
}

pub fn supports_fetch(kind: SearchKind) -> bool {
    preset(kind).fetch_url.is_some()
}

/// Where a provider's own key is kept in the secret store.
pub fn secret_id(id: &str) -> String {
    format!("search:{id}")
}

fn search_endpoint(provider: &SearchProvider) -> Result<Url, String> {
    let raw = match preset(provider.kind).search_url {
        Some(url) => url.to_string(),
        None => {
            let base = provider.base_url.as_deref().unwrap_or("").trim().trim_end_matches('/');
            if provider.kind == SearchKind::Searxng && !base.ends_with("/search") { format!("{base}/search") } else { base.to_string() }
        }
    };
    Url::parse(&raw).map_err(|_| format!("{} needs a valid URL in Settings › Web.", provider.name))
}

// ---------------------------------------------------------------------------------------------
// Requests and answers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recency {
    Day,
    Week,
    Month,
    Year,
}

impl Recency {
    fn days(self) -> i64 {
        match self {
            Recency::Day => 1,
            Recency::Week => 7,
            Recency::Month => 31,
            Recency::Year => 365,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Recency::Day => "day",
            Recency::Week => "week",
            Recency::Month => "month",
            Recency::Year => "year",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    pub query: String,
    pub count: u8,
    pub site: Option<String>,
    pub recency: Option<Recency>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    /// Short, for the results card.
    pub snippet: String,
    /// Longer, for the model.
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedPage {
    pub title: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

/// Checks and tidies what the model asked for.
pub fn normalize_query(query: &str, count: Option<u64>, site: Option<&str>, recency: Option<Recency>) -> Result<SearchQuery, String> {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Err("Give web_search a query.".into());
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("Keep search queries under {MAX_QUERY_CHARS} characters."));
    }
    let count = count.unwrap_or(DEFAULT_COUNT.into()).clamp(1, MAX_COUNT.into()) as u8;
    let site = match site.map(str::trim).filter(|site| !site.is_empty()) {
        None => None,
        Some(site) => {
            let site = site.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_ascii_lowercase();
            let valid = site.len() <= 253 && site.contains('.') && site.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            if !valid {
                return Err("`site` should be a bare domain such as docs.rs.".into());
            }
            Some(site)
        }
    };
    Ok(SearchQuery { query, count, site, recency })
}

/// The query text with `site:` and, where the provider reads it from the query, `after:` added.
fn query_with_operators(query: &SearchQuery, after: Option<&str>) -> String {
    let mut text = query.query.clone();
    if let Some(site) = &query.site {
        text.push_str(&format!(" site:{site}"));
    }
    if let Some(after) = after {
        text.push_str(&format!(" after:{after}"));
    }
    text
}

/// `today` is days since 1970-01-01, so tests can fix the date.
pub fn build_search(provider: &SearchProvider, key: Option<&str>, query: &SearchQuery, today: i64) -> Result<Prepared, String> {
    let preset = preset(provider.kind);
    let count = query.count.min(preset.max_count).max(1);
    let mut url = search_endpoint(provider)?;
    let mut headers = Vec::new();
    let mut body = None;
    let mut method = HttpMethod::Get;
    let check_length = |text: &str| -> Result<(), String> {
        if text.chars().count() > preset.max_query_chars {
            Err(format!(
                "{} accepts search queries up to {} characters, including site and date filters. Shorten the query.",
                provider.name, preset.max_query_chars
            ))
        } else {
            Ok(())
        }
    };
    match provider.kind {
        SearchKind::Deepseek => {
            let after = query.recency.map(|recency| date(today - recency.days()));
            let text = query_with_operators(query, after.as_deref());
            check_length(&text)?;
            method = HttpMethod::Post;
            body = Some(json!({ "query": text, "count": count }));
            headers.push(("Authorization".into(), format!("Bearer {}", key.unwrap_or(""))));
        }
        SearchKind::Brave => {
            let text = query_with_operators(query, None);
            check_length(&text)?;
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("q", &text).append_pair("count", &count.to_string());
                if let Some(recency) = query.recency {
                    let freshness = match recency {
                        Recency::Day => "pd",
                        Recency::Week => "pw",
                        Recency::Month => "pm",
                        Recency::Year => "py",
                    };
                    pairs.append_pair("freshness", freshness);
                }
            }
            headers.push(("X-Subscription-Token".into(), key.unwrap_or("").to_string()));
            headers.push(("Accept".into(), "application/json".into()));
        }
        SearchKind::Tavily => {
            check_length(&query.query)?;
            method = HttpMethod::Post;
            let mut payload = json!({ "query": query.query, "max_results": count });
            if let Some(site) = &query.site {
                payload["include_domains"] = json!([site]);
            }
            if let Some(recency) = query.recency {
                payload["time_range"] = json!(recency.word());
            }
            body = Some(payload);
            headers.push(("Authorization".into(), format!("Bearer {}", key.unwrap_or(""))));
        }
        SearchKind::Searxng => {
            let text = query_with_operators(query, None);
            check_length(&text)?;
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &text).append_pair("format", "json");
            if let Some(recency) = query.recency {
                pairs.append_pair("time_range", recency.word());
            }
        }
        SearchKind::Custom => {
            let spec = provider.custom.as_ref().ok_or_else(|| format!("{} is missing its request settings.", provider.name))?;
            let text = query_with_operators(query, None);
            check_length(&text)?;
            match spec.method {
                HttpMethod::Get => {
                    let param = spec.query_param.as_deref().unwrap_or("q");
                    let mut pairs = url.query_pairs_mut();
                    pairs.append_pair(param, &text);
                    if let Some(count_param) = &spec.count_param {
                        pairs.append_pair(count_param, &count.to_string());
                    }
                }
                HttpMethod::Post => {
                    method = HttpMethod::Post;
                    body = Some(render_body_template(spec.body_template.as_deref().unwrap_or(""), &text, count)?);
                }
            }
            if let Some(header) = &spec.auth_header {
                headers.push((header.clone(), format!("{}{}", spec.auth_prefix.as_deref().unwrap_or(""), key.unwrap_or(""))));
            }
            headers.push(("Accept".into(), "application/json".into()));
        }
    }
    Ok(Prepared { method, url, headers, body })
}

/// Fills `{{query}}` (as JSON string content) and `{{count}}` into a Custom POST body.
pub fn render_body_template(template: &str, query: &str, count: u8) -> Result<Value, String> {
    let escaped = serde_json::to_string(query).map_err(|error| error.to_string())?;
    let escaped = &escaped[1..escaped.len() - 1];
    let filled = template.replace("{{query}}", escaped).replace("{{count}}", &count.to_string());
    serde_json::from_str(&filled).map_err(|_| "The request body template isn't valid JSON once the query is filled in.".to_string())
}

struct RawHit {
    title: String,
    url: String,
    snippet: String,
    summary: String,
    date: Option<String>,
    site: Option<String>,
}

pub fn parse_search(provider: &SearchProvider, body: &Value, count: u8) -> Result<Vec<SearchHit>, String> {
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let optional = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).filter(|value| !value.trim().is_empty()).map(str::to_string);
    let mut raw = Vec::new();
    match provider.kind {
        SearchKind::Deepseek => {
            for item in array_at(body, "results")? {
                raw.push(RawHit {
                    title: text(item, "title"),
                    url: text(item, "url"),
                    snippet: text(item, "snippet"),
                    summary: text(item, "summary"),
                    date: optional(item, "date"),
                    site: optional(item, "site_name"),
                });
            }
        }
        SearchKind::Brave => {
            // No web results at all (only news, say) comes back without `web`.
            for item in body.pointer("/web/results").and_then(Value::as_array).into_iter().flatten() {
                let description = text(item, "description");
                raw.push(RawHit {
                    title: text(item, "title"),
                    url: text(item, "url"),
                    snippet: description.clone(),
                    summary: description,
                    date: optional(item, "page_age").or_else(|| optional(item, "age")),
                    site: item.pointer("/profile/name").and_then(Value::as_str).map(str::to_string),
                });
            }
        }
        SearchKind::Tavily | SearchKind::Searxng => {
            let date_key = if provider.kind == SearchKind::Tavily { "published_date" } else { "publishedDate" };
            for item in array_at(body, "results")? {
                let content = text(item, "content");
                raw.push(RawHit {
                    title: text(item, "title"),
                    url: text(item, "url"),
                    snippet: content.clone(),
                    summary: content,
                    date: optional(item, date_key),
                    site: None,
                });
            }
        }
        SearchKind::Custom => {
            let spec = provider.custom.as_ref().ok_or_else(|| format!("{} is missing its response settings.", provider.name))?;
            let results = json_path(body, &spec.results_path)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{}'s response has no results list at “{}”.", provider.name, spec.results_path))?;
            for item in results {
                let at = |path: &Option<String>| path.as_deref().and_then(|path| json_path(item, path)).and_then(value_text);
                let snippet = at(&spec.snippet_path).unwrap_or_default();
                raw.push(RawHit {
                    title: json_path(item, &spec.title_path).and_then(value_text).unwrap_or_default(),
                    url: json_path(item, &spec.url_path).and_then(value_text).unwrap_or_default(),
                    snippet: snippet.clone(),
                    summary: snippet,
                    date: at(&spec.date_path),
                    site: None,
                });
            }
        }
    }
    let mut hits: Vec<SearchHit> = Vec::new();
    for hit in raw {
        let Ok(parsed) = Url::parse(hit.url.trim()) else { continue };
        if !matches!(parsed.scheme(), "http" | "https") || hits.iter().any(|seen| seen.url == parsed.as_str()) {
            continue;
        }
        let host = parsed.host_str().unwrap_or("").to_string();
        let title = clean_text(&hit.title, 200);
        hits.push(SearchHit {
            title: if title.is_empty() { host } else { title },
            url: parsed.to_string(),
            snippet: clean_text(&hit.snippet, 300),
            summary: clean_text(if hit.summary.trim().is_empty() { &hit.snippet } else { &hit.summary }, 800),
            date: hit.date.map(|date| clean_text(&date, 40)).filter(|date| !date.is_empty()),
            site: hit.site.map(|site| clean_text(&site, 80)).filter(|site| !site.is_empty()),
        });
        if hits.len() >= count as usize {
            break;
        }
    }
    Ok(hits)
}

fn array_at<'a>(body: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    body.get(key).and_then(Value::as_array).ok_or_else(|| "The search provider returned a response without results.".to_string())
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// A dotted path with numeric indexes, such as `data.0.items`. An empty path is the value.
pub fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim();
    if path.is_empty() {
        return Some(value);
    }
    path.split('.').try_fold(value, |current, segment| match current {
        Value::Array(items) => segment.parse::<usize>().ok().and_then(|index| items.get(index)),
        Value::Object(map) => map.get(segment),
        _ => None,
    })
}

pub fn validate_json_path(path: &str, label: &str, allow_empty: bool) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return if allow_empty { Ok(String::new()) } else { Err(format!("Enter the JSON path for {label}.")) };
    }
    if path.chars().count() > 200 || path.split('.').any(|segment| segment.is_empty() || segment.chars().any(char::is_whitespace)) {
        return Err(format!("The JSON path for {label} should look like data.results or items.0.link."));
    }
    Ok(path.to_string())
}

/// Plain one-line text: tags dropped, common entities decoded, control characters removed,
/// whitespace collapsed, and cut to `max` characters.
pub fn clean_text(value: &str, max: usize) -> String {
    let mut stripped = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            _ => stripped.push(character),
        }
    }
    let decoded = stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ");
    let compact = decoded
        .chars()
        .map(|character| if character.is_control() { ' ' } else { character })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= max {
        compact
    } else {
        let mut cut: String = compact.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

pub fn build_fetch(provider: &SearchProvider, key: Option<&str>, url: &str) -> Result<Prepared, String> {
    let endpoint = preset(provider.kind)
        .fetch_url
        .ok_or_else(|| format!("{} can't read pages. Switch off “Read pages through {}” in Settings › Web.", provider.name, provider.name))?;
    let endpoint = Url::parse(endpoint).map_err(|error| error.to_string())?;
    let body = match provider.kind {
        SearchKind::Tavily => json!({ "urls": [url] }),
        _ => json!({ "url": url }),
    };
    Ok(Prepared {
        method: HttpMethod::Post,
        url: endpoint,
        headers: vec![("Authorization".into(), format!("Bearer {}", key.unwrap_or("")))],
        body: Some(body),
    })
}

pub fn parse_fetch(provider: &SearchProvider, body: &Value) -> Result<FetchedPage, String> {
    let (title, content) = match provider.kind {
        SearchKind::Tavily => match body.get("results").and_then(Value::as_array).and_then(|results| results.first()) {
            Some(item) => (String::new(), item.get("raw_content").and_then(Value::as_str).unwrap_or("").to_string()),
            None => {
                let reason = body
                    .pointer("/failed_results/0/error")
                    .and_then(Value::as_str)
                    .map(|reason| format!(": {}", clean_text(reason, 200)))
                    .unwrap_or_default();
                return Err(format!("{} couldn't read that page{reason}.", provider.name));
            }
        },
        _ => (
            body.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            body.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
        ),
    };
    let truncated = content.len() > FETCH_CONTENT_LIMIT;
    let content = if truncated {
        let mut end = FETCH_CONTENT_LIMIT;
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        content[..end].to_string()
    } else {
        content
    };
    Ok(FetchedPage { title: clean_text(&title, 300), content, truncated })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Search,
    Fetch,
}

/// The sentence the agent (and Settings' Test search) sees for an error status.
pub fn status_sentence(provider: &SearchProvider, op: Op, status: u16, message: Option<&str>) -> String {
    let name = &provider.name;
    let base = match (status, op) {
        (401, _) => format!("{name} rejected the API key. Check it in Settings › Web."),
        (402, _) if provider.kind == SearchKind::Deepseek => "Your DeepSeek balance is empty. Top up at platform.deepseek.com, then try again.".to_string(),
        (402, _) => format!("{name} says the account is out of credit."),
        (403, Op::Fetch) => format!("{name} refused to read that page."),
        (403, Op::Search) => format!("{name} refused the request (HTTP 403)."),
        (404, Op::Fetch) => format!("{name} couldn't get readable content from that page."),
        (408 | 504, _) => format!("{name} timed out. Try again shortly."),
        (429, _) => format!("{name} is rate-limiting requests. Wait a moment, then try again."),
        (500..=599, _) => format!("{name} had a server error (HTTP {status}). Try again shortly."),
        _ => format!("{name} returned HTTP {status}."),
    };
    match message.map(|message| clean_text(message, 300)).filter(|message| !message.is_empty()) {
        Some(message) => format!("{base} It said: {message}"),
        None => base,
    }
}

fn error_message(body: &Value) -> Option<&str> {
    body.pointer("/error/message")
        .or_else(|| body.pointer("/detail/error"))
        .or_else(|| body.get("detail"))
        .or_else(|| body.get("message"))
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
}

// ---------------------------------------------------------------------------------------------
// The client

/// How a request ended, for the usage meter. A stopped request is not recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// HTTP 200 with results or page content: what a provider charges for.
    Billed,
    /// HTTP 200 with nothing in it.
    Empty,
    /// An error status or no answer at all.
    Failed,
}

/// Counts what a provider answered (Settings › Web's usage meter).
pub trait Meter: Send + Sync {
    fn record(&self, provider: &str, op: Op, outcome: Outcome);
}

/// One provider, ready to search: its settings and, privately, its key. It is built by the
/// engine (`Store::search_client`), the one place a search key leaves the secret store.
pub struct SearchClient {
    provider: SearchProvider,
    key: Option<String>,
    client: reqwest::Client,
    slots: Arc<Semaphore>,
    meter: Option<Arc<dyn Meter>>,
}

impl std::fmt::Debug for SearchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchClient").field("provider", &self.provider.name).finish_non_exhaustive()
    }
}

impl SearchClient {
    pub fn new(provider: SearchProvider, key: Option<String>, meter: Option<Arc<dyn Meter>>) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| format!("Could not set up web search: {error}."))?;
        Ok(Self { provider, key, client, slots: Arc::new(Semaphore::new(PROVIDER_CONCURRENCY)), meter })
    }

    pub fn name(&self) -> &str {
        &self.provider.name
    }

    pub fn kind(&self) -> SearchKind {
        self.provider.kind
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, String> {
        let prepared = build_search(&self.provider, self.key.as_deref(), query, today())?;
        let body = self.call(Op::Search, prepared).await?;
        let hits = parse_search(&self.provider, &body, query.count);
        self.record(Op::Search, if hits.as_ref().is_ok_and(|hits| !hits.is_empty()) { Outcome::Billed } else { Outcome::Empty });
        hits
    }

    /// Reads a page through the provider's own fetch endpoint. The URL must already have passed
    /// the guard's public-address check.
    pub async fn fetch(&self, url: &str) -> Result<FetchedPage, String> {
        let prepared = build_fetch(&self.provider, self.key.as_deref(), url)?;
        let body = self.call(Op::Fetch, prepared).await?;
        let page = parse_fetch(&self.provider, &body);
        self.record(Op::Fetch, if page.as_ref().is_ok_and(|page| !page.content.is_empty()) { Outcome::Billed } else { Outcome::Empty });
        page
    }

    fn record(&self, op: Op, outcome: Outcome) {
        if let Some(meter) = &self.meter {
            meter.record(&self.provider.id, op, outcome);
        }
    }

    fn scrub(&self, message: String) -> String {
        match self.key.as_deref().filter(|key| key.len() >= 4) {
            Some(key) => message.replace(key, "[credential redacted]"),
            None => message,
        }
    }

    /// Sends one request and returns the JSON body of a 2xx answer, or the sentence for anything
    /// else, scrubbed of the key. Every finished request is counted.
    async fn call(&self, op: Op, prepared: Prepared) -> Result<Value, String> {
        let _slot = self.slots.acquire().await.map_err(|_| "Web search is shutting down.".to_string())?;
        let result = self.send(op, prepared).await.map_err(|message| self.scrub(message));
        if result.is_err() {
            self.record(op, Outcome::Failed);
        }
        result
    }

    async fn send(&self, op: Op, prepared: Prepared) -> Result<Value, String> {
        let name = &self.provider.name;
        let limit = match op {
            Op::Search => SEARCH_BODY_LIMIT,
            Op::Fetch => FETCH_BODY_LIMIT,
        };
        let mut request = match prepared.method {
            HttpMethod::Get => self.client.get(prepared.url),
            HttpMethod::Post => self.client.post(prepared.url),
        };
        for (header, value) in &prepared.headers {
            request = request.header(header, value);
        }
        if let Some(body) = &prepared.body {
            request = request.json(body);
        }
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                format!("{name} didn't answer within {} seconds.", TIMEOUT.as_secs())
            } else {
                format!("Could not reach {name}: {}", error.without_url())
            }
        })?;
        let status = response.status();
        if status.is_redirection() {
            return Err(format!("{name} redirected the request. Set its URL to the final endpoint in Settings › Web."));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| format!("{name} stopped answering: {}", error.without_url()))? {
            if bytes.len() + chunk.len() > limit {
                return Err(format!("{name}'s answer was larger than {} MB.", limit / (1024 * 1024)));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Option<Value> = serde_json::from_slice(&bytes).ok();
        if !status.is_success() {
            let message = body.as_ref().and_then(error_message);
            return Err(status_sentence(&self.provider, op, status.as_u16(), message));
        }
        body.ok_or_else(|| {
            let hint = if self.provider.kind == SearchKind::Searxng { " Turn on the JSON format in the instance's settings.yml (search.formats)." } else { "" };
            format!("{name} didn't return JSON.{hint}")
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Dates, without a date crate

/// Days since 1970-01-01, today, in UTC.
fn today() -> i64 {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0);
    (seconds / 86_400) as i64
}

/// `YYYY-MM-DD` for a count of days since 1970-01-01 (Howard Hinnant's civil-from-days).
fn date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 2026-10-17.
    const TODAY: i64 = 20_743;

    pub fn provider(kind: SearchKind) -> SearchProvider {
        SearchProvider {
            id: "s1".into(),
            name: preset(kind).label.into(),
            kind,
            key_source: KeySource::Own,
            base_url: match kind {
                SearchKind::Searxng => Some("http://127.0.0.1:8888".into()),
                SearchKind::Custom => Some("https://search.example.com/api".into()),
                _ => None,
            },
            custom: (kind == SearchKind::Custom).then(|| CustomSpec {
                method: HttpMethod::Get,
                auth_header: Some("X-API-Key".into()),
                auth_prefix: None,
                query_param: Some("term".into()),
                count_param: Some("n".into()),
                body_template: None,
                results_path: "data.items".into(),
                title_path: "name".into(),
                url_path: "link.href".into(),
                snippet_path: Some("blurb".into()),
                date_path: None,
            }),
            price_search_per_1k: None,
            price_fetch_per_1k: None,
        }
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery { query: text.into(), count: 5, site: None, recency: None }
    }

    #[test]
    fn dates_count_from_the_epoch() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(TODAY), "2026-10-17");
        assert_eq!(date(TODAY - 7), "2026-10-10");
        assert_eq!(date(11_016), "2000-02-29");
    }

    #[test]
    fn deepseek_puts_filters_in_the_query_and_the_key_in_a_bearer_header() {
        let mut search = query("rust async");
        search.site = Some("docs.rs".into());
        search.recency = Some(Recency::Week);
        let prepared = build_search(&provider(SearchKind::Deepseek), Some("sk-test"), &search, TODAY).unwrap();
        assert_eq!(prepared.method, HttpMethod::Post);
        assert_eq!(prepared.url.as_str(), "https://api.deepseek.com/v1/tools/search");
        assert_eq!(prepared.body.unwrap(), json!({ "query": "rust async site:docs.rs after:2026-10-10", "count": 5 }));
        assert!(prepared.headers.contains(&("Authorization".into(), "Bearer sk-test".into())));
    }

    #[test]
    fn deepseek_refuses_queries_past_its_limit_before_sending() {
        let error = build_search(&provider(SearchKind::Deepseek), Some("k"), &query(&"a".repeat(129)), TODAY).unwrap_err();
        assert!(error.contains("128 characters"), "{error}");
        // The filters count too.
        let mut filtered = query(&"a".repeat(120));
        filtered.site = Some("docs.rs".into());
        assert!(build_search(&provider(SearchKind::Deepseek), Some("k"), &filtered, TODAY).is_err());
    }

    #[test]
    fn brave_tavily_and_searxng_map_filters_their_own_way() {
        let mut search = query("tauri");
        search.recency = Some(Recency::Day);
        search.site = Some("github.com".into());
        let brave = build_search(&provider(SearchKind::Brave), Some("bk"), &search, TODAY).unwrap();
        assert_eq!(brave.url.query(), Some("q=tauri+site%3Agithub.com&count=5&freshness=pd"));
        assert!(brave.headers.contains(&("X-Subscription-Token".into(), "bk".into())));
        let tavily = build_search(&provider(SearchKind::Tavily), Some("tk"), &search, TODAY).unwrap();
        assert_eq!(tavily.body.unwrap(), json!({ "query": "tauri", "max_results": 5, "include_domains": ["github.com"], "time_range": "day" }));
        let searxng = build_search(&provider(SearchKind::Searxng), None, &search, TODAY).unwrap();
        assert_eq!(searxng.url.as_str(), "http://127.0.0.1:8888/search?q=tauri+site%3Agithub.com&format=json&time_range=day");
        assert!(searxng.headers.is_empty());
    }

    #[test]
    fn custom_get_and_post_follow_the_spec() {
        let custom = provider(SearchKind::Custom);
        let get = build_search(&custom, Some("ck"), &query("hi"), TODAY).unwrap();
        assert_eq!(get.url.as_str(), "https://search.example.com/api?term=hi&n=5");
        assert!(get.headers.contains(&("X-API-Key".into(), "ck".into())));
        let mut post = custom.clone();
        let spec = post.custom.as_mut().unwrap();
        spec.method = HttpMethod::Post;
        spec.body_template = Some(r#"{"q": "{{query}}", "size": {{count}}}"#.into());
        let prepared = build_search(&post, Some("ck"), &query("say \"hi\""), TODAY).unwrap();
        assert_eq!(prepared.body.unwrap(), json!({ "q": "say \"hi\"", "size": 5 }));
    }

    #[test]
    fn parses_each_preset_and_drops_bad_urls_and_duplicates() {
        let deepseek = json!({ "results": [
            { "title": "A <b>page</b>", "url": "https://a.example/x", "snippet": "short", "summary": "long summary", "site_name": "A", "site_icon": "https://a.example/i.png", "date": "2026-06-01T00:00:00+00:00" },
            { "title": "dup", "url": "https://a.example/x", "snippet": "", "summary": "" },
            { "title": "bad", "url": "javascript:alert(1)" },
        ] });
        let hits = parse_search(&provider(SearchKind::Deepseek), &deepseek, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "A page");
        assert_eq!(hits[0].summary, "long summary");
        assert_eq!(hits[0].site.as_deref(), Some("A"));
        assert!(!serde_json::to_string(&hits).unwrap().contains("i.png"));

        let brave = json!({ "web": { "results": [{ "title": "B", "url": "https://b.example/", "description": "with <strong>bold</strong> &amp; more", "page_age": "2026-01-01" }] } });
        assert_eq!(parse_search(&provider(SearchKind::Brave), &brave, 5).unwrap()[0].snippet, "with bold & more");
        assert!(parse_search(&provider(SearchKind::Brave), &json!({ "query": {} }), 5).unwrap().is_empty());

        let tavily = json!({ "results": [{ "title": "T", "url": "https://t.example/", "content": "c" }] });
        assert_eq!(parse_search(&provider(SearchKind::Tavily), &tavily, 5).unwrap()[0].summary, "c");
        let searxng = json!({ "results": [{ "title": "S", "url": "https://s.example/", "content": "x", "publishedDate": "2025-02-02" }] });
        assert_eq!(parse_search(&provider(SearchKind::Searxng), &searxng, 5).unwrap()[0].date.as_deref(), Some("2025-02-02"));

        let custom = json!({ "data": { "items": [{ "name": "C", "link": { "href": "https://c.example/" }, "blurb": "b" }] } });
        let hits = parse_search(&provider(SearchKind::Custom), &custom, 5).unwrap();
        assert_eq!((hits[0].title.as_str(), hits[0].url.as_str(), hits[0].snippet.as_str()), ("C", "https://c.example/", "b"));
        assert!(parse_search(&provider(SearchKind::Custom), &json!({}), 5).unwrap_err().contains("data.items"));
    }

    #[test]
    fn caps_results_at_the_count() {
        let results: Vec<Value> = (0..8).map(|index| json!({ "title": "x", "url": format!("https://x.example/{index}") })).collect();
        assert_eq!(parse_search(&provider(SearchKind::Tavily), &json!({ "results": results }), 3).unwrap().len(), 3);
    }

    #[test]
    fn json_paths_walk_objects_and_arrays() {
        let value = json!({ "a": [{ "b": "c" }] });
        assert_eq!(json_path(&value, "a.0.b"), Some(&json!("c")));
        assert_eq!(json_path(&value, "a.1.b"), None);
        assert_eq!(json_path(&value, ""), Some(&value));
        assert!(validate_json_path("a..b", "x", false).is_err());
        assert!(validate_json_path("", "x", false).is_err());
        assert_eq!(validate_json_path(" data.results ", "x", false).unwrap(), "data.results");
    }

    #[test]
    fn fetch_requests_and_answers() {
        let deepseek = provider(SearchKind::Deepseek);
        let prepared = build_fetch(&deepseek, Some("k"), "https://a.example/").unwrap();
        assert_eq!(prepared.url.as_str(), "https://api.deepseek.com/v1/tools/fetch");
        assert_eq!(prepared.body.unwrap(), json!({ "url": "https://a.example/" }));
        let page = parse_fetch(&deepseek, &json!({ "title": "T", "content": "# T\n\nBody" })).unwrap();
        assert_eq!((page.title.as_str(), page.content.as_str(), page.truncated), ("T", "# T\n\nBody", false));
        let tavily = provider(SearchKind::Tavily);
        assert_eq!(build_fetch(&tavily, Some("k"), "https://a.example/").unwrap().body.unwrap(), json!({ "urls": ["https://a.example/"] }));
        assert_eq!(parse_fetch(&tavily, &json!({ "results": [{ "raw_content": "body" }] })).unwrap().content, "body");
        assert!(parse_fetch(&tavily, &json!({ "results": [], "failed_results": [{ "error": "blocked" }] })).unwrap_err().contains("blocked"));
        assert!(build_fetch(&provider(SearchKind::Brave), Some("k"), "https://a.example/").is_err());
    }

    #[test]
    fn status_sentences_say_what_to_do() {
        let deepseek = provider(SearchKind::Deepseek);
        assert!(status_sentence(&deepseek, Op::Search, 402, None).contains("DeepSeek balance is empty"));
        assert!(status_sentence(&provider(SearchKind::Brave), Op::Search, 402, None).contains("out of credit"));
        assert!(status_sentence(&deepseek, Op::Search, 401, None).contains("Settings › Web"));
        assert!(status_sentence(&deepseek, Op::Fetch, 404, None).contains("readable content"));
        assert!(status_sentence(&deepseek, Op::Search, 429, Some("slow down")).ends_with("It said: slow down"));
    }

    #[test]
    fn normalizes_agent_queries() {
        let query = normalize_query("  rust   tauri ", Some(99), Some("https://Docs.rs/"), None).unwrap();
        assert_eq!((query.query.as_str(), query.count, query.site.as_deref()), ("rust tauri", MAX_COUNT, Some("docs.rs")));
        assert_eq!(normalize_query("x", None, None, None).unwrap().count, DEFAULT_COUNT);
        assert!(normalize_query("   ", None, None, None).is_err());
        assert!(normalize_query("x", None, Some("not a site"), None).is_err());
    }

    /// Serves one canned HTTP response on a local port.
    pub async fn serve_once(head: &'static str, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0u8; 8192];
                let _ = stream.read(&mut buffer).await;
                let response = format!("HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    fn searxng_at(base: String) -> SearchProvider {
        let mut provider = provider(SearchKind::Searxng);
        provider.base_url = Some(base);
        provider.key_source = KeySource::None;
        provider
    }

    #[derive(Default)]
    struct Counts(std::sync::Mutex<Vec<(Op, Outcome)>>);

    impl Meter for Counts {
        fn record(&self, _: &str, op: Op, outcome: Outcome) {
            self.0.lock().unwrap().push((op, outcome));
        }
    }

    #[tokio::test]
    async fn calls_refuse_redirects_scrub_the_key_and_are_counted() {
        let base = serve_once("302 Found\r\nLocation: https://elsewhere.example/", "").await;
        let error = SearchClient::new(searxng_at(base), None, None).unwrap().search(&query("x")).await.unwrap_err();
        assert!(error.contains("redirected"), "{error}");

        // A keyed Custom provider on loopback, whose server echoes the key back.
        let base = serve_once("401 Unauthorized\r\nContent-Type: application/json", r#"{"error":{"message":"bad key sk-abcdefg"}}"#).await;
        let mut custom = provider(SearchKind::Custom);
        custom.base_url = Some(base);
        let counts = Arc::new(Counts::default());
        let client = SearchClient::new(custom, Some("sk-abcdefg".into()), Some(counts.clone())).unwrap();
        let error = client.search(&query("x")).await.unwrap_err();
        assert!(error.contains("rejected the API key") && !error.contains("sk-abcdefg"), "{error}");
        assert_eq!(counts.0.lock().unwrap().as_slice(), [(Op::Search, Outcome::Failed)]);

        let base = serve_once("200 OK\r\nContent-Type: application/json", r#"{"results":[{"title":"T","url":"https://t.example/"}]}"#).await;
        let counts = Arc::new(Counts::default());
        let hits = SearchClient::new(searxng_at(base), None, Some(counts.clone())).unwrap().search(&query("x")).await.unwrap();
        assert_eq!(hits[0].title, "T");
        assert_eq!(counts.0.lock().unwrap().as_slice(), [(Op::Search, Outcome::Billed)]);

        let base = serve_once("200 OK\r\nContent-Type: text/html", "<html>").await;
        let error = SearchClient::new(searxng_at(base), None, None).unwrap().search(&query("x")).await.unwrap_err();
        assert!(error.contains("JSON format"), "{error}");
    }
}
