//! Settings › Web: the search providers behind the agent's `web_search` tool, and web_fetch
//! routed through a provider's own fetch endpoint.
//!
//! Rust makes every one of these requests. A worker asks over the `web` native channel
//! (`web_request` / `web_cancel` / `web_response`, like the browser's) and gets results back,
//! so a search key never reaches a worker, Chat mode included. Workers learn only what is
//! available (`runtime_payload`, sent in `init` and live as `set_web`, never in the
//! fingerprint).
//!
//! Invariants:
//! - Only the provider's own endpoint is contacted, and redirects are refused: a key is never
//!   forwarded to another host. Keys travel only over https, except to this Mac.
//! - A linked key (`SearchKeySource::Connection`) is used only when the provider's endpoint has
//!   the same origin as the connection it comes from. Custom and SearXNG can never link.
//! - Before a page URL goes to a provider's fetch, it is checked here as well as in the worker:
//!   http(s), no credentials, and a public address. A private URL never leaves the Mac.
//! - No silent fallback: an error from the active provider is the answer.
use crate::models::{
    CustomSearchSpec, HttpMethod, ProviderKind, ProviderRecord, SaveSearchProviderInput, SearchKeySource,
    SearchProviderKind, SearchProviderRecord, WebConfig,
};
use crate::secrets::SecretStore;
use crate::storage::MetadataState;
use crate::web_usage::{self, Op, Outcome};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs},
    pin::pin,
    sync::{Arc, Mutex},
    task::Poll,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{Notify, Semaphore};

pub const MAX_PROVIDERS: usize = 16;
const TIMEOUT: Duration = Duration::from_secs(30);
const SEARCH_BODY_LIMIT: usize = 2 * 1024 * 1024;
const FETCH_BODY_LIMIT: usize = 6 * 1024 * 1024;
/// Matches web_fetch's own `MAX_BODY_BYTES`.
const FETCH_CONTENT_LIMIT: usize = 5 * 1024 * 1024;
pub const MAX_COUNT: u8 = 10;
const DEFAULT_COUNT: u8 = 5;
const MAX_QUERY_CHARS: usize = 400;
const MAX_URL_CHARS: usize = 8192;
/// Parallel requests per provider, under DeepSeek's 5 per second when sub-agents search at once.
const PROVIDER_CONCURRENCY: usize = 4;
const USER_AGENT: &str = "WackCode-web/1.0";

// ---------------------------------------------------------------------------------------------
// Presets

pub struct Preset {
    pub label: &'static str,
    /// Fixed search endpoint; `None` means the record's own `base_url` (SearXNG, Custom).
    pub search_url: Option<&'static str>,
    pub fetch_url: Option<&'static str>,
    /// A Settings › Providers connection on this origin may lend its key.
    pub link_origin: Option<&'static str>,
    pub max_count: u8,
    /// The provider's own query limit, counted after operators are added.
    pub max_query_chars: usize,
}

pub fn preset(kind: SearchProviderKind) -> &'static Preset {
    match kind {
        SearchProviderKind::Deepseek => &Preset {
            label: "DeepSeek",
            search_url: Some("https://api.deepseek.com/v1/tools/search"),
            fetch_url: Some("https://api.deepseek.com/v1/tools/fetch"),
            link_origin: Some("https://api.deepseek.com"),
            max_count: 50,
            max_query_chars: 128,
        },
        SearchProviderKind::Brave => &Preset {
            label: "Brave Search",
            search_url: Some("https://api.search.brave.com/res/v1/web/search"),
            fetch_url: None,
            link_origin: None,
            max_count: 20,
            max_query_chars: 400,
        },
        SearchProviderKind::Tavily => &Preset {
            label: "Tavily",
            search_url: Some("https://api.tavily.com/search"),
            fetch_url: Some("https://api.tavily.com/extract"),
            link_origin: None,
            max_count: 20,
            max_query_chars: 400,
        },
        SearchProviderKind::Searxng => &Preset {
            label: "SearXNG",
            search_url: None,
            fetch_url: None,
            link_origin: None,
            max_count: MAX_COUNT,
            max_query_chars: 400,
        },
        SearchProviderKind::Custom => &Preset {
            label: "Custom",
            search_url: None,
            fetch_url: None,
            link_origin: None,
            max_count: MAX_COUNT,
            max_query_chars: 400,
        },
    }
}

/// Whether this provider can't search without a key.
pub fn needs_key(record: &SearchProviderRecord) -> bool {
    match record.kind {
        SearchProviderKind::Deepseek | SearchProviderKind::Brave | SearchProviderKind::Tavily => true,
        SearchProviderKind::Searxng => false,
        SearchProviderKind::Custom => record.custom.as_ref().is_some_and(|spec| spec.auth_header.is_some()),
    }
}

pub fn supports_fetch(kind: SearchProviderKind) -> bool {
    preset(kind).fetch_url.is_some()
}

pub fn secret_id(id: &str) -> String {
    format!("search:{id}")
}

fn search_endpoint(record: &SearchProviderRecord) -> Result<reqwest::Url, String> {
    let raw = match preset(record.kind).search_url {
        Some(url) => url.to_string(),
        None => {
            let base = record.base_url.as_deref().unwrap_or("").trim().trim_end_matches('/');
            if record.kind == SearchProviderKind::Searxng && !base.ends_with("/search") {
                format!("{base}/search")
            } else {
                base.to_string()
            }
        }
    };
    reqwest::Url::parse(&raw).map_err(|_| format!("{} needs a valid URL in Settings › Web.", record.name))
}

// ---------------------------------------------------------------------------------------------
// Requests and results (pure, so each preset is testable without a network)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub query: String,
    pub count: u8,
    pub site: Option<String>,
    pub recency: Option<Recency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    pub title: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRequest {
    pub method: HttpMethod,
    pub url: reqwest::Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
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

pub fn build_search(
    record: &SearchProviderRecord,
    key: Option<&str>,
    query: &SearchQuery,
    today: chrono::NaiveDate,
) -> Result<PreparedRequest, String> {
    let preset = preset(record.kind);
    let count = query.count.min(preset.max_count).max(1);
    let mut url = search_endpoint(record)?;
    let mut headers = Vec::new();
    let mut body = None;
    let mut method = HttpMethod::Get;
    let check_length = |text: &str| -> Result<(), String> {
        if text.chars().count() > preset.max_query_chars {
            Err(format!(
                "{} accepts search queries up to {} characters, including site and date filters. Shorten the query.",
                record.name, preset.max_query_chars
            ))
        } else {
            Ok(())
        }
    };
    match record.kind {
        SearchProviderKind::Deepseek => {
            let after = query.recency.map(|recency| (today - chrono::Duration::days(recency.days())).format("%Y-%m-%d").to_string());
            let text = query_with_operators(query, after.as_deref());
            check_length(&text)?;
            method = HttpMethod::Post;
            body = Some(json!({ "query": text, "count": count }));
            headers.push(("Authorization".into(), format!("Bearer {}", key.unwrap_or(""))));
        }
        SearchProviderKind::Brave => {
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
        SearchProviderKind::Tavily => {
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
        SearchProviderKind::Searxng => {
            let text = query_with_operators(query, None);
            check_length(&text)?;
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &text).append_pair("format", "json");
            if let Some(recency) = query.recency {
                pairs.append_pair("time_range", recency.word());
            }
        }
        SearchProviderKind::Custom => {
            let spec = record.custom.as_ref().ok_or_else(|| format!("{} is missing its request settings.", record.name))?;
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
    Ok(PreparedRequest { method, url, headers, body })
}

/// Fills `{{query}}` (as JSON string content) and `{{count}}` into a Custom POST body.
pub fn render_body_template(template: &str, query: &str, count: u8) -> Result<Value, String> {
    let escaped = serde_json::to_string(query).map_err(|error| error.to_string())?;
    let escaped = &escaped[1..escaped.len() - 1];
    let filled = template.replace("{{query}}", escaped).replace("{{count}}", &count.to_string());
    serde_json::from_str(&filled).map_err(|_| "The request body template isn't valid JSON once the query is filled in.".to_string())
}

pub fn parse_search(record: &SearchProviderRecord, body: &Value, count: u8) -> Result<Vec<SearchHit>, String> {
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let optional = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).filter(|value| !value.trim().is_empty()).map(str::to_string);
    let mut raw: Vec<(String, String, String, String, Option<String>, Option<String>)> = Vec::new();
    match record.kind {
        SearchProviderKind::Deepseek => {
            for item in array_at(body, "results")? {
                raw.push((text(item, "title"), text(item, "url"), text(item, "snippet"), text(item, "summary"), optional(item, "date"), optional(item, "site_name")));
            }
        }
        SearchProviderKind::Brave => {
            // No web results at all (only news, say) comes back without `web`.
            let results = body.pointer("/web/results").and_then(Value::as_array).cloned().unwrap_or_default();
            for item in &results {
                let description = text(item, "description");
                let site = item.pointer("/profile/name").and_then(Value::as_str).map(str::to_string);
                raw.push((text(item, "title"), text(item, "url"), description.clone(), description, optional(item, "page_age").or_else(|| optional(item, "age")), site));
            }
        }
        SearchProviderKind::Tavily => {
            for item in array_at(body, "results")? {
                let content = text(item, "content");
                raw.push((text(item, "title"), text(item, "url"), content.clone(), content, optional(item, "published_date"), None));
            }
        }
        SearchProviderKind::Searxng => {
            for item in array_at(body, "results")? {
                let content = text(item, "content");
                raw.push((text(item, "title"), text(item, "url"), content.clone(), content, optional(item, "publishedDate"), None));
            }
        }
        SearchProviderKind::Custom => {
            let spec = record.custom.as_ref().ok_or_else(|| format!("{} is missing its response settings.", record.name))?;
            let results = json_path(body, &spec.results_path)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{}'s response has no results list at “{}”.", record.name, spec.results_path))?;
            for item in results {
                let at = |path: &Option<String>| path.as_deref().and_then(|path| json_path(item, path)).and_then(value_text);
                let snippet = at(&spec.snippet_path).unwrap_or_default();
                raw.push((
                    json_path(item, &spec.title_path).and_then(value_text).unwrap_or_default(),
                    json_path(item, &spec.url_path).and_then(value_text).unwrap_or_default(),
                    snippet.clone(),
                    snippet,
                    at(&spec.date_path),
                    None,
                ));
            }
        }
    }
    let mut hits: Vec<SearchHit> = Vec::new();
    for (title, url, snippet, summary, date, site) in raw {
        let Ok(parsed) = reqwest::Url::parse(url.trim()) else { continue };
        if !matches!(parsed.scheme(), "http" | "https") || hits.iter().any(|hit| hit.url == parsed.as_str()) {
            continue;
        }
        let host = parsed.host_str().unwrap_or("").to_string();
        let title = clean_text(&title, 200);
        hits.push(SearchHit {
            title: if title.is_empty() { host } else { title },
            url: parsed.to_string(),
            snippet: clean_text(&snippet, 300),
            summary: clean_text(if summary.trim().is_empty() { &snippet } else { &summary }, 800),
            date: date.map(|date| clean_text(&date, 40)).filter(|date| !date.is_empty()),
            site: site.map(|site| clean_text(&site, 80)).filter(|site| !site.is_empty()),
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

pub fn build_fetch(record: &SearchProviderRecord, key: Option<&str>, url: &str) -> Result<PreparedRequest, String> {
    let endpoint = preset(record.kind)
        .fetch_url
        .ok_or_else(|| format!("{} can't read pages. Switch off “Read pages through {}” in Settings › Web.", record.name, record.name))?;
    let endpoint = reqwest::Url::parse(endpoint).map_err(|error| error.to_string())?;
    let body = match record.kind {
        SearchProviderKind::Tavily => json!({ "urls": [url] }),
        _ => json!({ "url": url }),
    };
    Ok(PreparedRequest {
        method: HttpMethod::Post,
        url: endpoint,
        headers: vec![("Authorization".into(), format!("Bearer {}", key.unwrap_or("")))],
        body: Some(body),
    })
}

pub fn parse_fetch(record: &SearchProviderRecord, body: &Value) -> Result<FetchedPage, String> {
    let (title, content) = match record.kind {
        SearchProviderKind::Tavily => {
            let first = body.get("results").and_then(Value::as_array).and_then(|results| results.first());
            match first {
                Some(item) => (String::new(), item.get("raw_content").and_then(Value::as_str).unwrap_or("").to_string()),
                None => {
                    let reason = body
                        .pointer("/failed_results/0/error")
                        .and_then(Value::as_str)
                        .map(|reason| format!(": {}", clean_text(reason, 200)))
                        .unwrap_or_default();
                    return Err(format!("{} couldn't read that page{reason}.", record.name));
                }
            }
        }
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

/// The sentence the agent (and Settings' Test search) sees for an error status.
pub fn status_sentence(record: &SearchProviderRecord, op: Op, status: u16, message: Option<&str>) -> String {
    let name = &record.name;
    let base = match (status, op) {
        (401, _) => format!("{name} rejected the API key. Check it in Settings › Web."),
        (402, _) if record.kind == SearchProviderKind::Deepseek => {
            "Your DeepSeek balance is empty. Top up at platform.deepseek.com, then try again.".to_string()
        }
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
// HTTP

/// Sends one prepared request and returns the JSON body of a 2xx answer, or the sentence for
/// anything else. The key is scrubbed from every error.
async fn call(
    record: &SearchProviderRecord,
    op: Op,
    prepared: PreparedRequest,
    key: Option<&str>,
) -> Result<Value, CallError> {
    let limit = match op {
        Op::Search => SEARCH_BODY_LIMIT,
        Op::Fetch => FETCH_BODY_LIMIT,
    };
    let scrub = |message: String| match key.filter(|key| key.len() >= 4) {
        Some(key) => message.replace(key, "[credential redacted]"),
        None => message,
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| CallError::Unanswered(format!("Could not search: {error}")))?;
    let mut request = match prepared.method {
        HttpMethod::Get => client.get(prepared.url),
        HttpMethod::Post => client.post(prepared.url),
    };
    for (name, value) in &prepared.headers {
        request = request.header(name, value);
    }
    if let Some(body) = &prepared.body {
        request = request.json(body);
    }
    let mut response = request.send().await.map_err(|error| {
        CallError::Unanswered(scrub(if error.is_timeout() {
            format!("{} didn't answer within {} seconds.", record.name, TIMEOUT.as_secs())
        } else {
            format!("Could not reach {}: {}", record.name, error.without_url())
        }))
    })?;
    let status = response.status();
    if status.is_redirection() {
        return Err(CallError::Answered(format!(
            "{} redirected the request. Set its URL to the final endpoint in Settings › Web.",
            record.name
        )));
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > limit {
                    return Err(CallError::Answered(format!("{}'s answer was larger than {} MB.", record.name, limit / (1024 * 1024))));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => return Err(CallError::Unanswered(scrub(format!("{} stopped answering: {}", record.name, error.without_url())))),
        }
    }
    let body: Option<Value> = serde_json::from_slice(&bytes).ok();
    if !status.is_success() {
        let message = body.as_ref().and_then(error_message);
        return Err(CallError::Answered(scrub(status_sentence(record, op, status.as_u16(), message))));
    }
    body.ok_or_else(|| {
        let hint = if record.kind == SearchProviderKind::Searxng { " Turn on the JSON format in the instance's settings.yml (search.formats)." } else { "" };
        CallError::Answered(format!("{} didn't return JSON.{hint}", record.name))
    })
}

#[derive(Debug)]
enum CallError {
    /// The provider answered with an error: counted as a failed request.
    Answered(String),
    /// No answer: still counted as failed, since the request may have reached the provider.
    Unanswered(String),
}

impl CallError {
    fn message(self) -> String {
        match self {
            CallError::Answered(message) | CallError::Unanswered(message) => message,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Keys

/// The key this provider sends, `None` when it needs none. The error is the user's sentence.
pub fn resolve_key(record: &SearchProviderRecord, providers: &[ProviderRecord], secrets: &SecretStore) -> Result<Option<String>, String> {
    let missing = || format!("Add an API key for {} in Settings › Web.", record.name);
    match &record.key_source {
        SearchKeySource::None => {
            if needs_key(record) { Err(missing()) } else { Ok(None) }
        }
        SearchKeySource::Own => secrets.get_optional(&secret_id(&record.id))?.map(Some).ok_or_else(missing),
        SearchKeySource::Connection { provider_id } => {
            let connection = linkable_connection(record.kind, provider_id, providers)?;
            secrets
                .get_optional(&connection.id)?
                .map(Some)
                .ok_or_else(|| format!("The connection ‘{}’ has no API key. Add one in Settings › Providers.", connection.name))
        }
    }
}

/// The connection a provider may borrow a key from: an enabled API-key connection on the
/// preset's own origin. Anything else is refused, so a key never reaches another host.
pub fn linkable_connection<'a>(kind: SearchProviderKind, provider_id: &str, providers: &'a [ProviderRecord]) -> Result<&'a ProviderRecord, String> {
    let preset = preset(kind);
    let origin = preset
        .link_origin
        .ok_or_else(|| format!("{} can't use a connection's key. Give it its own key.", preset.label))?;
    let connection = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| "The connection this provider borrows its key from no longer exists. Pick another key in Settings › Web.".to_string())?;
    if connection.kind == ProviderKind::Subscription {
        return Err("A subscription sign-in has no API key to lend. Give this provider its own key.".into());
    }
    if !connection.enabled {
        return Err(format!("The connection ‘{}’ is switched off. Switch it on in Settings › Providers, or give this provider its own key.", connection.name));
    }
    if !same_origin(&connection.base_url, origin) {
        return Err(format!("The connection ‘{}’ isn't on {origin}, so its key can't be used here.", connection.name));
    }
    Ok(connection)
}

pub fn same_origin(left: &str, right: &str) -> bool {
    match (reqwest::Url::parse(left.trim()), reqwest::Url::parse(right.trim())) {
        (Ok(left), Ok(right)) => left.origin() == right.origin(),
        _ => false,
    }
}

/// Recomputes each provider's `has_key` from the secret store and Providers.
pub fn refresh_key_state(config: &mut WebConfig, providers: &[ProviderRecord], secrets: &SecretStore) {
    for record in &mut config.providers {
        record.has_key = resolve_key(record, providers, secrets).is_ok();
    }
}

/// The `web` value for `init` and `set_web`: what a worker may offer, never a key. Null while
/// search is off or the active provider can't search (no key, a switched-off connection).
pub fn runtime_payload(config: &WebConfig, providers: &[ProviderRecord], secrets: &SecretStore) -> Value {
    if !config.enabled {
        return Value::Null;
    }
    let Some(active) = config.active_provider_id.as_deref().and_then(|id| config.providers.iter().find(|record| record.id == id)) else {
        return Value::Null;
    };
    if resolve_key(active, providers, secrets).is_err() {
        return Value::Null;
    }
    let fetch = if config.route_fetch && supports_fetch(active.kind) { json!({ "label": active.name }) } else { Value::Null };
    json!({ "search": { "label": active.name, "kind": active.kind }, "fetch": fetch })
}

// ---------------------------------------------------------------------------------------------
// Page URLs sent to a provider's fetch

const BLOCKED_SUFFIXES: &[&str] = &[".localhost", ".local", ".internal", ".home.arpa"];

/// The URL checks web_fetch makes before reading a page, repeated here before handing one to a
/// third party: http(s), no credentials, a real public host.
pub fn validate_page_url(raw: &str) -> Result<reqwest::Url, String> {
    if raw.chars().count() > MAX_URL_CHARS {
        return Err("That URL is too long.".into());
    }
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "That isn't a valid URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https pages can be read.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with a username or password aren't read.".into());
    }
    let host = url.host_str().ok_or_else(|| "That URL has no host.".to_string())?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" || BLOCKED_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
        return Err("Pages on this Mac or the local network can't be read.".into());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err("Pages on this Mac or the local network can't be read.".into());
        }
    }
    Ok(url)
}

/// Resolves the host and refuses it if any address is private, so an internal name never
/// reaches a provider either.
async fn assert_public_host(url: &reqwest::Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']').to_string();
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let lookup = host.clone();
    let addresses = tauri::async_runtime::spawn_blocking(move || (lookup.as_str(), port).to_socket_addrs().map(|found| found.collect::<Vec<_>>()))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|_| format!("Could not find {host}."))?;
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err("Pages on this Mac or the local network can't be read.".into());
    }
    Ok(())
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let segments = ip.segments();
            // Only global unicast, minus documentation, Teredo and 6to4 (judged by its IPv4).
            if segments[0] & 0xe000 != 0x2000 {
                return false;
            }
            if segments[0] == 0x2001 && (segments[1] == 0x0db8 || segments[1] == 0) {
                return false;
            }
            if segments[0] == 0x2002 {
                let embedded = Ipv4Addr::new((segments[1] >> 8) as u8, segments[1] as u8, (segments[2] >> 8) as u8, segments[2] as u8);
                return is_public_v4(embedded);
            }
            ip != Ipv6Addr::UNSPECIFIED
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

// ---------------------------------------------------------------------------------------------
// Agent requests (the `web` native channel)

#[derive(Default)]
pub struct WebState {
    limits: Mutex<HashMap<String, Arc<Semaphore>>>,
    cancels: Mutex<HashMap<String, Arc<Notify>>>,
}

impl WebState {
    fn limit(&self, provider_id: &str) -> Arc<Semaphore> {
        let mut limits = self.limits.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        limits.entry(provider_id.to_string()).or_insert_with(|| Arc::new(Semaphore::new(PROVIDER_CONCURRENCY))).clone()
    }

    /// The cancel signal for a request. A cancel that arrives before the request starts leaves a
    /// stored permit, so the request stops as soon as it begins.
    fn signal(&self, request_id: &str) -> Arc<Notify> {
        let mut cancels = self.cancels.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        cancels.entry(request_id.to_string()).or_default().clone()
    }

    pub fn cancel(&self, request_id: &str) {
        self.signal(request_id).notify_one();
    }

    fn finish(&self, request_id: &str) {
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.remove(request_id);
        }
    }
}

/// Runs `future` unless `cancel` fires first.
async fn unless_cancelled<F: Future>(future: F, cancel: &Notify) -> Option<F::Output> {
    let mut future = pin!(future);
    let mut notified = pin!(cancel.notified());
    std::future::poll_fn(|context| {
        if notified.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(context).map(Some)
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
enum AgentRequest {
    #[serde(rename = "search")]
    Search {
        query: String,
        #[serde(default)]
        count: Option<u32>,
        #[serde(default)]
        site: Option<String>,
        #[serde(default)]
        recency: Option<Recency>,
    },
    #[serde(rename = "fetch")]
    Fetch { url: String },
}

pub fn normalize_query(query: &str, count: Option<u32>, site: Option<&str>, recency: Option<Recency>) -> Result<SearchQuery, String> {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Err("Give web_search a query.".into());
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!("Keep search queries under {MAX_QUERY_CHARS} characters."));
    }
    let count = count.unwrap_or(DEFAULT_COUNT as u32).clamp(1, MAX_COUNT as u32) as u8;
    let site = match site.map(str::trim).filter(|site| !site.is_empty()) {
        None => None,
        Some(site) => {
            let site = site.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_ascii_lowercase();
            let valid = site.len() <= 253
                && site.contains('.')
                && site.chars().all(|character| character.is_ascii_alphanumeric() || character == '.' || character == '-');
            if !valid {
                return Err("`site` should be a bare domain such as docs.rs.".into());
            }
            Some(site)
        }
    };
    Ok(SearchQuery { query, count, site, recency })
}

struct Snapshot {
    record: SearchProviderRecord,
    route_fetch: bool,
    key: Option<String>,
}

/// Re-reads Settings on every request: the worker's view may be a moment old.
fn active_snapshot(app: &AppHandle) -> Result<Snapshot, String> {
    let state = app.state::<MetadataState>();
    let (config, providers) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        (data.web.clone(), data.providers.clone())
    };
    if !config.enabled {
        return Err("Web search is off. Switch it on in Settings › Web.".into());
    }
    let record = config
        .active_provider_id
        .as_deref()
        .and_then(|id| config.providers.iter().find(|record| record.id == id))
        .cloned()
        .ok_or_else(|| "No web search provider is active. Pick one in Settings › Web.".to_string())?;
    let key = resolve_key(&record, &providers, &state.secrets)?;
    Ok(Snapshot { record, route_fetch: config.route_fetch, key })
}

pub async fn execute_agent_request(app: AppHandle, _task_id: String, request_id: String, request: Value) -> Result<Value, String> {
    let state = app.state::<WebState>();
    let cancel = state.signal(&request_id);
    let result = run_agent_request(&app, request, &cancel).await;
    state.finish(&request_id);
    result
}

async fn run_agent_request(app: &AppHandle, request: Value, cancel: &Notify) -> Result<Value, String> {
    let request: AgentRequest = serde_json::from_value(request).map_err(|_| "That web request isn't one WackCode understands.".to_string())?;
    let snapshot = active_snapshot(app)?;
    let record = snapshot.record;
    let provider = json!({ "id": record.id, "name": record.name, "kind": record.kind });
    match request {
        AgentRequest::Search { query, count, site, recency } => {
            let query = normalize_query(&query, count, site.as_deref(), recency)?;
            let hits = run(app, &record, Op::Search, cancel, || async {
                let prepared = build_search(&record, snapshot.key.as_deref(), &query, chrono::Local::now().date_naive())?;
                Ok(prepared)
            }, |body| parse_search(&record, body, query.count), |hits: &Vec<SearchHit>| !hits.is_empty(), snapshot.key.as_deref()).await?;
            Ok(json!({ "provider": provider, "results": hits }))
        }
        AgentRequest::Fetch { url } => {
            if !snapshot.route_fetch || !supports_fetch(record.kind) {
                return Err(format!("Reading pages through {} is off. Switch it on in Settings › Web.", record.name));
            }
            let url = validate_page_url(&url)?;
            assert_public_host(&url).await?;
            let page = run(app, &record, Op::Fetch, cancel, || async { build_fetch(&record, snapshot.key.as_deref(), url.as_str()) },
                |body| parse_fetch(&record, body), |page: &FetchedPage| !page.content.trim().is_empty(), snapshot.key.as_deref()).await?;
            Ok(json!({ "provider": provider, "url": url.as_str(), "title": page.title, "content": page.content, "truncated": page.truncated }))
        }
    }
}

/// One provider request: limited per provider, cancellable, and counted once it is answered.
async fn run<T, B, BF, P, N>(
    app: &AppHandle,
    record: &SearchProviderRecord,
    op: Op,
    cancel: &Notify,
    build: B,
    parse: P,
    non_empty: N,
    key: Option<&str>,
) -> Result<T, String>
where
    B: FnOnce() -> BF,
    BF: Future<Output = Result<PreparedRequest, String>>,
    P: FnOnce(&Value) -> Result<T, String>,
    N: FnOnce(&T) -> bool,
{
    let prepared = build().await?;
    let limit = app.state::<WebState>().limit(&record.id);
    let Some(permit) = unless_cancelled(limit.acquire_owned(), cancel).await else {
        return Err("Cancelled.".into());
    };
    let _permit = permit.map_err(|_| "Web search is shutting down.".to_string())?;
    let Some(answer) = unless_cancelled(call(record, op, prepared, key), cancel).await else {
        return Err("Cancelled.".into());
    };
    let outcome = answer.and_then(|body| parse(&body).map_err(CallError::Answered));
    match outcome {
        Ok(value) => {
            web_usage::record(app, &record.id, op, if non_empty(&value) { Outcome::Billed } else { Outcome::Empty });
            Ok(value)
        }
        Err(error) => {
            web_usage::record(app, &record.id, op, Outcome::Failed);
            Err(error.message())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Settings › Web commands

fn validate_endpoint(raw: Option<&str>, label: &str) -> Result<String, String> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty()).ok_or_else(|| format!("Enter the {label} URL."))?;
    let url = reqwest::Url::parse(raw).map_err(|_| format!("Enter a valid {label} URL, starting with https://."))?;
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
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn validate_header_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| character.is_ascii_alphanumeric() || "-_".contains(character));
    if valid { Ok(name.to_string()) } else { Err("The auth header should be a plain header name, such as Authorization or X-API-Key.".into()) }
}

fn validate_price(price: Option<f64>) -> Result<Option<f64>, String> {
    match price {
        Some(price) if !price.is_finite() || !(0.0..=10_000.0).contains(&price) => Err("Enter a price between 0 and 10,000 per 1,000 requests.".into()),
        other => Ok(other),
    }
}

fn validate_custom(spec: &CustomSearchSpec) -> Result<CustomSearchSpec, String> {
    let trim = |value: &Option<String>| value.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    let auth_header = trim(&spec.auth_header).map(|name| validate_header_name(&name)).transpose()?;
    let auth_prefix = spec.auth_prefix.as_deref().filter(|prefix| !prefix.is_empty()).map(str::to_string);
    if auth_prefix.as_deref().is_some_and(|prefix| prefix.len() > 40 || prefix.chars().any(char::is_control)) {
        return Err("The key prefix should be short text such as “Bearer ”.".into());
    }
    let query_param = trim(&spec.query_param);
    let body_template = trim(&spec.body_template);
    match spec.method {
        HttpMethod::Get => {
            if query_param.is_none() {
                return Err("Enter the query parameter name, such as q.".into());
            }
        }
        HttpMethod::Post => {
            let template = body_template.as_deref().ok_or_else(|| "Enter a JSON body with {{query}} where the query goes.".to_string())?;
            if !template.contains("{{query}}") {
                return Err("The body template needs {{query}} where the query goes.".into());
            }
            render_body_template(template, "test", 5)?;
        }
    }
    Ok(CustomSearchSpec {
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

/// Validates a save (or a Test search draft) into the record it would store. Keys are handled
/// by the caller; `has_key` is filled in later.
pub fn prepare_record(
    input: &SaveSearchProviderInput,
    existing: Option<&SearchProviderRecord>,
    providers: &[ProviderRecord],
) -> Result<SearchProviderRecord, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Give the provider a name.".into());
    }
    if name.chars().count() > 60 {
        return Err("Keep the provider's name under 60 characters.".into());
    }
    if existing.is_some_and(|existing| existing.kind != input.kind) {
        return Err("A provider's type can't change. Add a new provider instead.".into());
    }
    let (base_url, custom) = match input.kind {
        SearchProviderKind::Searxng => (Some(validate_endpoint(input.base_url.as_deref(), "SearXNG instance")?), None),
        SearchProviderKind::Custom => {
            let spec = input.custom.as_ref().ok_or_else(|| "Fill in how to call this search API.".to_string())?;
            (Some(validate_endpoint(input.base_url.as_deref(), "search API")?), Some(validate_custom(spec)?))
        }
        _ => (None, None),
    };
    let now = Utc::now().to_rfc3339();
    let mut record = SearchProviderRecord {
        id: existing.map(|existing| existing.id.clone()).or_else(|| input.id.clone()).unwrap_or_else(|| format!("search-{}", uuid::Uuid::new_v4().simple())),
        name: name.to_string(),
        kind: input.kind,
        key_source: input.key_source.clone(),
        has_key: false,
        base_url,
        custom,
        price_search_per_1k: validate_price(input.price_search_per_1k)?,
        price_fetch_per_1k: validate_price(input.price_fetch_per_1k)?,
        created_at: existing.map(|existing| existing.created_at.clone()).unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    if !needs_key(&record) {
        record.key_source = SearchKeySource::None;
    }
    match &record.key_source {
        SearchKeySource::None if needs_key(&record) => record.key_source = SearchKeySource::Own,
        SearchKeySource::Connection { provider_id } => {
            linkable_connection(record.kind, provider_id, providers)?;
        }
        _ => {}
    }
    // A key goes only over https, except to this Mac.
    if needs_key(&record) {
        if let Some(url) = &record.base_url {
            if url.starts_with("http://") && !is_loopback(url) {
                return Err("This provider sends a key, so its URL must use https:// (plain http:// is allowed only to this Mac).".into());
            }
        }
    }
    Ok(record)
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

/// The config the renderer shows, with each `has_key` freshly worked out.
pub fn current_config(state: &MetadataState) -> Result<WebConfig, String> {
    let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
    let mut config = data.web.clone();
    refresh_key_state(&mut config, &data.providers, &state.secrets);
    Ok(config)
}

#[tauri::command]
pub async fn save_search_provider(app: AppHandle, state: State<'_, MetadataState>, input: SaveSearchProviderInput) -> Result<WebConfig, String> {
    let api_key = clean_key(input.api_key.as_deref())?;
    let (existing, providers, count) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let existing = input.id.as_deref().and_then(|id| data.web.providers.iter().find(|record| record.id == id)).cloned();
        (existing, data.providers.clone(), data.web.providers.len())
    };
    if input.id.is_some() && existing.is_none() {
        return Err("That search provider no longer exists.".into());
    }
    if existing.is_none() && count >= MAX_PROVIDERS {
        return Err(format!("WackCode keeps up to {MAX_PROVIDERS} search providers. Remove one first."));
    }
    let mut record = prepare_record(&input, existing.as_ref(), &providers)?;
    let secret = secret_id(&record.id);
    // The key is stored before the record that needs it, and dropped when nothing uses it.
    if record.key_source == SearchKeySource::Own {
        match api_key {
            Some(key) => state.secrets.set(&secret, &key)?,
            None if state.secrets.get_optional(&secret)?.is_none() => {
                return Err(format!("Paste an API key for {}.", record.name));
            }
            None => {}
        }
    } else {
        state.secrets.remove(&secret)?;
    }
    // Stored for readers of the file; `current_config` recomputes it on every read anyway.
    record.has_key = resolve_key(&record, &providers, &state.secrets).is_ok();
    state.mutate(|data| {
        match data.web.providers.iter_mut().find(|existing| existing.id == record.id) {
            Some(existing) => *existing = record.clone(),
            None => data.web.providers.push(record.clone()),
        }
        if data.web.active_provider_id.is_none() {
            data.web.active_provider_id = Some(record.id.clone());
        }
        Ok(())
    })?;
    crate::worker::broadcast_web(&app).await?;
    current_config(&state)
}

#[tauri::command]
pub async fn delete_search_provider(app: AppHandle, state: State<'_, MetadataState>, provider_id: String) -> Result<WebConfig, String> {
    state.secrets.remove(&secret_id(&provider_id))?;
    state.mutate(|data| {
        data.web.providers.retain(|record| record.id != provider_id);
        if data.web.active_provider_id.as_deref() == Some(provider_id.as_str()) {
            data.web.active_provider_id = None;
        }
        Ok(())
    })?;
    let usage = app.state::<crate::web_usage::WebUsageState>();
    usage.reset(Some(&provider_id));
    web_usage::schedule_flush(&app);
    crate::worker::broadcast_web(&app).await?;
    current_config(&state)
}

#[tauri::command]
pub async fn set_active_search_provider(app: AppHandle, state: State<'_, MetadataState>, provider_id: Option<String>) -> Result<WebConfig, String> {
    state.mutate(|data| {
        if let Some(id) = &provider_id {
            if !data.web.providers.iter().any(|record| &record.id == id) {
                return Err("That search provider no longer exists.".to_string());
            }
        }
        data.web.active_provider_id = provider_id.clone();
        Ok(())
    })?;
    crate::worker::broadcast_web(&app).await?;
    current_config(&state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebConfigInput {
    pub enabled: bool,
    pub route_fetch: bool,
}

#[tauri::command]
pub async fn set_web_config(app: AppHandle, state: State<'_, MetadataState>, input: WebConfigInput) -> Result<WebConfig, String> {
    state.mutate(|data| {
        data.web.enabled = input.enabled;
        data.web.route_fetch = input.route_fetch;
        Ok(())
    })?;
    crate::worker::broadcast_web(&app).await?;
    current_config(&state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSearchInput {
    /// The saved provider to test, or the one the draft edits.
    #[serde(default)]
    pub id: Option<String>,
    /// Unsaved edits: tested as they stand. A blank key falls back to the saved one.
    #[serde(default)]
    pub draft: Option<SaveSearchProviderInput>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub results: Vec<SearchHit>,
    pub elapsed_ms: u64,
}

/// Settings' Test search: three results through the provider (saved, or as drafted). A saved
/// provider's test counts in the usage meter like any other search; an unsaved one has no
/// counter to land in.
#[tauri::command]
pub async fn test_search_provider(app: AppHandle, state: State<'_, MetadataState>, input: TestSearchInput) -> Result<WebTestResult, String> {
    let started = Instant::now();
    let outcome = test_search(&app, &state, input).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(match outcome {
        Ok(results) => WebTestResult { ok: true, error: None, results, elapsed_ms },
        Err(error) => WebTestResult { ok: false, error: Some(crate::worker::redact_and_limit(&error)), results: Vec::new(), elapsed_ms },
    })
}

async fn test_search(app: &AppHandle, state: &MetadataState, input: TestSearchInput) -> Result<Vec<SearchHit>, String> {
    let (existing, providers) = {
        let data = state.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        let id = input.id.as_deref().or_else(|| input.draft.as_ref().and_then(|draft| draft.id.as_deref()));
        (id.and_then(|id| data.web.providers.iter().find(|record| record.id == id)).cloned(), data.providers.clone())
    };
    let (record, key) = match &input.draft {
        Some(draft) => {
            let record = prepare_record(draft, existing.as_ref(), &providers)?;
            let key = match (&record.key_source, clean_key(draft.api_key.as_deref())?) {
                (SearchKeySource::Own, Some(key)) => Some(key),
                _ => resolve_key(&record, &providers, &state.secrets)?,
            };
            (record, key)
        }
        None => {
            let record = existing.clone().ok_or_else(|| "That search provider no longer exists.".to_string())?;
            let key = resolve_key(&record, &providers, &state.secrets)?;
            (record, key)
        }
    };
    let query = normalize_query(&input.query, Some(3), None, None)?;
    let prepared = build_search(&record, key.as_deref(), &query, chrono::Local::now().date_naive())?;
    let answer = call(&record, Op::Search, prepared, key.as_deref()).await;
    let parsed = answer.and_then(|body| parse_search(&record, &body, 3).map_err(CallError::Answered));
    if existing.is_some() {
        let outcome = match &parsed {
            Ok(hits) if !hits.is_empty() => Outcome::Billed,
            Ok(_) => Outcome::Empty,
            Err(_) => Outcome::Failed,
        };
        web_usage::record(app, &record.id, Op::Search, outcome);
    }
    parsed.map_err(CallError::message)
}

#[tauri::command]
pub fn web_usage(app: AppHandle) -> web_usage::WebUsageSummary {
    app.state::<crate::web_usage::WebUsageState>().summary(&web_usage::current_month())
}

#[tauri::command]
pub fn reset_web_usage(app: AppHandle, provider_id: Option<String>) -> web_usage::WebUsageSummary {
    let usage = app.state::<crate::web_usage::WebUsageState>();
    usage.reset(provider_id.as_deref());
    web_usage::schedule_flush(&app);
    usage.summary(&web_usage::current_month())
}

/// Search providers that borrow this connection's key, by name: deleting it is refused.
pub fn providers_linking(config: &WebConfig, connection_id: &str) -> Vec<String> {
    config
        .providers
        .iter()
        .filter(|record| matches!(&record.key_source, SearchKeySource::Connection { provider_id } if provider_id == connection_id))
        .map(|record| record.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn record(kind: SearchProviderKind) -> SearchProviderRecord {
        SearchProviderRecord {
            id: "s1".into(),
            name: preset(kind).label.into(),
            kind,
            key_source: SearchKeySource::Own,
            has_key: true,
            base_url: match kind {
                SearchProviderKind::Searxng => Some("http://127.0.0.1:8888".into()),
                SearchProviderKind::Custom => Some("https://search.example.com/api".into()),
                _ => None,
            },
            custom: (kind == SearchProviderKind::Custom).then(|| CustomSearchSpec {
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
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery { query: text.into(), count: 5, site: None, recency: None }
    }

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()
    }

    fn connection(id: &str, base_url: &str) -> ProviderRecord {
        ProviderRecord {
            id: id.into(),
            name: "DeepSeek chat".into(),
            kind: ProviderKind::Custom,
            base_url: base_url.into(),
            api_format: "openai-completions".into(),
            models: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
            has_api_key: true,
            connected: true,
            enabled: true,
        }
    }

    #[test]
    fn deepseek_puts_filters_in_the_query_and_the_key_in_a_bearer_header() {
        let mut search = query("rust async");
        search.site = Some("docs.rs".into());
        search.recency = Some(Recency::Week);
        let prepared = build_search(&record(SearchProviderKind::Deepseek), Some("sk-test"), &search, today()).unwrap();
        assert_eq!(prepared.method, HttpMethod::Post);
        assert_eq!(prepared.url.as_str(), "https://api.deepseek.com/v1/tools/search");
        assert_eq!(prepared.body.unwrap(), json!({ "query": "rust async site:docs.rs after:2026-10-10", "count": 5 }));
        assert!(prepared.headers.contains(&("Authorization".into(), "Bearer sk-test".into())));
    }

    #[test]
    fn deepseek_refuses_queries_past_its_limit() {
        let error = build_search(&record(SearchProviderKind::Deepseek), Some("k"), &query(&"a".repeat(129)), today()).unwrap_err();
        assert!(error.contains("128 characters"), "{error}");
    }

    #[test]
    fn brave_tavily_and_searxng_map_filters_their_own_way() {
        let mut search = query("tauri");
        search.recency = Some(Recency::Day);
        search.site = Some("github.com".into());
        let brave = build_search(&record(SearchProviderKind::Brave), Some("bk"), &search, today()).unwrap();
        assert_eq!(brave.url.query(), Some("q=tauri+site%3Agithub.com&count=5&freshness=pd"));
        assert!(brave.headers.contains(&("X-Subscription-Token".into(), "bk".into())));
        let tavily = build_search(&record(SearchProviderKind::Tavily), Some("tk"), &search, today()).unwrap();
        assert_eq!(tavily.body.unwrap(), json!({ "query": "tauri", "max_results": 5, "include_domains": ["github.com"], "time_range": "day" }));
        let searxng = build_search(&record(SearchProviderKind::Searxng), None, &search, today()).unwrap();
        assert_eq!(searxng.url.as_str(), "http://127.0.0.1:8888/search?q=tauri+site%3Agithub.com&format=json&time_range=day");
        assert!(searxng.headers.is_empty());
    }

    #[test]
    fn custom_get_and_post_follow_the_spec() {
        let custom = record(SearchProviderKind::Custom);
        let get = build_search(&custom, Some("ck"), &query("hi"), today()).unwrap();
        assert_eq!(get.url.as_str(), "https://search.example.com/api?term=hi&n=5");
        assert!(get.headers.contains(&("X-API-Key".into(), "ck".into())));
        let mut post = custom.clone();
        let spec = post.custom.as_mut().unwrap();
        spec.method = HttpMethod::Post;
        spec.body_template = Some(r#"{"q": "{{query}}", "size": {{count}}}"#.into());
        let prepared = build_search(&post, Some("ck"), &query("say \"hi\""), today()).unwrap();
        assert_eq!(prepared.body.unwrap(), json!({ "q": "say \"hi\"", "size": 5 }));
    }

    #[test]
    fn parses_each_preset_and_drops_icons_bad_urls_and_duplicates() {
        let deepseek = json!({ "results": [
            { "title": "A <b>page</b>", "url": "https://a.example/x", "snippet": "short", "summary": "long summary", "site_name": "A", "site_icon": "https://a.example/i.png", "date": "2026-06-01T00:00:00+00:00" },
            { "title": "dup", "url": "https://a.example/x", "snippet": "", "summary": "" },
            { "title": "bad", "url": "javascript:alert(1)" },
        ] });
        let hits = parse_search(&record(SearchProviderKind::Deepseek), &deepseek, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "A page");
        assert_eq!(hits[0].summary, "long summary");
        assert_eq!(hits[0].site.as_deref(), Some("A"));
        assert!(!serde_json::to_string(&hits).unwrap().contains("i.png"));

        let brave = json!({ "web": { "results": [{ "title": "B", "url": "https://b.example/", "description": "with <strong>bold</strong> &amp; more", "page_age": "2026-01-01" }] } });
        let hits = parse_search(&record(SearchProviderKind::Brave), &brave, 5).unwrap();
        assert_eq!(hits[0].snippet, "with bold & more");
        assert!(parse_search(&record(SearchProviderKind::Brave), &json!({ "query": {} }), 5).unwrap().is_empty());

        let tavily = json!({ "results": [{ "title": "T", "url": "https://t.example/", "content": "c" }] });
        assert_eq!(parse_search(&record(SearchProviderKind::Tavily), &tavily, 5).unwrap()[0].summary, "c");

        let searxng = json!({ "results": [{ "title": "S", "url": "https://s.example/", "content": "x", "publishedDate": "2025-02-02" }] });
        assert_eq!(parse_search(&record(SearchProviderKind::Searxng), &searxng, 5).unwrap()[0].date.as_deref(), Some("2025-02-02"));

        let custom = json!({ "data": { "items": [{ "name": "C", "link": { "href": "https://c.example/" }, "blurb": "b" }] } });
        let hits = parse_search(&record(SearchProviderKind::Custom), &custom, 5).unwrap();
        assert_eq!((hits[0].title.as_str(), hits[0].url.as_str(), hits[0].snippet.as_str()), ("C", "https://c.example/", "b"));
        assert!(parse_search(&record(SearchProviderKind::Custom), &json!({}), 5).unwrap_err().contains("data.items"));
    }

    #[test]
    fn caps_results_at_the_count() {
        let results: Vec<Value> = (0..8).map(|index| json!({ "title": "x", "url": format!("https://x.example/{index}") })).collect();
        assert_eq!(parse_search(&record(SearchProviderKind::Tavily), &json!({ "results": results }), 3).unwrap().len(), 3);
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
        let deepseek = record(SearchProviderKind::Deepseek);
        let prepared = build_fetch(&deepseek, Some("k"), "https://a.example/").unwrap();
        assert_eq!(prepared.url.as_str(), "https://api.deepseek.com/v1/tools/fetch");
        assert_eq!(prepared.body.unwrap(), json!({ "url": "https://a.example/" }));
        let page = parse_fetch(&deepseek, &json!({ "title": "T", "content": "# T\n\nBody" })).unwrap();
        assert_eq!((page.title.as_str(), page.content.as_str(), page.truncated), ("T", "# T\n\nBody", false));
        let tavily = record(SearchProviderKind::Tavily);
        assert_eq!(build_fetch(&tavily, Some("k"), "https://a.example/").unwrap().body.unwrap(), json!({ "urls": ["https://a.example/"] }));
        assert_eq!(parse_fetch(&tavily, &json!({ "results": [{ "raw_content": "body" }] })).unwrap().content, "body");
        assert!(parse_fetch(&tavily, &json!({ "results": [], "failed_results": [{ "error": "blocked" }] })).unwrap_err().contains("blocked"));
        assert!(build_fetch(&record(SearchProviderKind::Brave), Some("k"), "https://a.example/").is_err());
    }

    #[test]
    fn status_sentences_say_what_to_do() {
        let deepseek = record(SearchProviderKind::Deepseek);
        assert!(status_sentence(&deepseek, Op::Search, 402, None).contains("DeepSeek balance is empty"));
        assert!(status_sentence(&record(SearchProviderKind::Brave), Op::Search, 402, None).contains("out of credit"));
        assert!(status_sentence(&deepseek, Op::Search, 401, None).contains("Settings › Web"));
        assert!(status_sentence(&deepseek, Op::Fetch, 404, None).contains("readable content"));
        assert!(status_sentence(&deepseek, Op::Search, 429, Some("slow down")).ends_with("It said: slow down"));
    }

    #[test]
    fn page_urls_must_be_public() {
        assert!(validate_page_url("https://example.com/a").is_ok());
        for url in [
            "http://localhost:3000/",
            "http://printer.local/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "https://user:pw@example.com/",
            "file:///etc/passwd",
            "ftp://example.com/",
        ] {
            assert!(validate_page_url(url).is_err(), "{url}");
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("2606:4700::1111".parse().unwrap()));
        assert!(!is_public_ip("100.64.0.1".parse().unwrap()));
        assert!(!is_public_ip("2002:0a00:0001::1".parse().unwrap()));
        assert!(!is_public_ip("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn normalizes_agent_queries() {
        let query = normalize_query("  rust   tauri ", Some(99), Some("https://Docs.rs/"), None).unwrap();
        assert_eq!((query.query.as_str(), query.count, query.site.as_deref()), ("rust tauri", MAX_COUNT, Some("docs.rs")));
        assert_eq!(normalize_query("x", None, None, None).unwrap().count, DEFAULT_COUNT);
        assert!(normalize_query("   ", None, None, None).is_err());
        assert!(normalize_query("x", None, Some("not a site"), None).is_err());
    }

    #[test]
    fn linked_keys_only_reach_their_own_origin() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = SecretStore::load(directory.path()).unwrap();
        secrets.set("custom-ds", "sk-linked").unwrap();
        secrets.set("custom-other", "sk-other").unwrap();
        let providers = vec![connection("custom-ds", "https://api.deepseek.com/v1"), connection("custom-other", "https://api.openai.com/v1")];
        let mut linked = record(SearchProviderKind::Deepseek);
        linked.key_source = SearchKeySource::Connection { provider_id: "custom-ds".into() };
        assert_eq!(resolve_key(&linked, &providers, &secrets).unwrap().as_deref(), Some("sk-linked"));

        linked.key_source = SearchKeySource::Connection { provider_id: "custom-other".into() };
        assert!(resolve_key(&linked, &providers, &secrets).unwrap_err().contains("isn't on https://api.deepseek.com"));

        let mut disabled = providers.clone();
        disabled[0].enabled = false;
        linked.key_source = SearchKeySource::Connection { provider_id: "custom-ds".into() };
        assert!(resolve_key(&linked, &disabled, &secrets).unwrap_err().contains("switched off"));

        let mut subscription = providers.clone();
        subscription[0].kind = ProviderKind::Subscription;
        assert!(resolve_key(&linked, &subscription, &secrets).is_err());

        let mut brave = record(SearchProviderKind::Brave);
        brave.key_source = SearchKeySource::Connection { provider_id: "custom-ds".into() };
        assert!(resolve_key(&brave, &providers, &secrets).unwrap_err().contains("its own key"));
    }

    #[test]
    fn own_keys_and_keyless_providers() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = SecretStore::load(directory.path()).unwrap();
        let own = record(SearchProviderKind::Brave);
        assert!(resolve_key(&own, &[], &secrets).unwrap_err().contains("Add an API key"));
        secrets.set(&secret_id("s1"), "bk").unwrap();
        assert_eq!(resolve_key(&own, &[], &secrets).unwrap().as_deref(), Some("bk"));
        let mut searxng = record(SearchProviderKind::Searxng);
        searxng.key_source = SearchKeySource::None;
        assert_eq!(resolve_key(&searxng, &[], &secrets).unwrap(), None);
    }

    #[test]
    fn runtime_payload_never_carries_a_key() {
        let directory = tempfile::tempdir().unwrap();
        let secrets = SecretStore::load(directory.path()).unwrap();
        secrets.set(&secret_id("s1"), "sk-secret-value").unwrap();
        let mut config = WebConfig { enabled: true, providers: vec![record(SearchProviderKind::Deepseek)], active_provider_id: Some("s1".into()), route_fetch: true };
        let payload = runtime_payload(&config, &[], &secrets);
        assert_eq!(payload, json!({ "search": { "label": "DeepSeek", "kind": "deepseek" }, "fetch": { "label": "DeepSeek" } }));
        assert!(!payload.to_string().contains("sk-secret"));
        config.providers[0].kind = SearchProviderKind::Brave;
        assert_eq!(runtime_payload(&config, &[], &secrets)["fetch"], Value::Null);
        config.enabled = false;
        assert_eq!(runtime_payload(&config, &[], &secrets), Value::Null);
        config.enabled = true;
        secrets.remove(&secret_id("s1")).unwrap();
        assert_eq!(runtime_payload(&config, &[], &secrets), Value::Null);
    }

    fn input(kind: SearchProviderKind) -> SaveSearchProviderInput {
        SaveSearchProviderInput {
            id: None,
            name: "Mine".into(),
            kind,
            key_source: SearchKeySource::Own,
            api_key: None,
            base_url: None,
            custom: None,
            price_search_per_1k: Some(0.75),
            price_fetch_per_1k: None,
        }
    }

    #[test]
    fn prepare_record_validates_what_is_saved() {
        assert!(prepare_record(&input(SearchProviderKind::Deepseek), None, &[]).is_ok());
        let mut searxng = input(SearchProviderKind::Searxng);
        assert!(prepare_record(&searxng, None, &[]).unwrap_err().contains("SearXNG instance URL"));
        searxng.base_url = Some("http://127.0.0.1:8888/".into());
        let saved = prepare_record(&searxng, None, &[]).unwrap();
        assert_eq!((saved.base_url.as_deref(), &saved.key_source), (Some("http://127.0.0.1:8888"), &SearchKeySource::None));

        let mut custom = input(SearchProviderKind::Custom);
        custom.base_url = Some("http://search.example.com/api".into());
        custom.custom = record(SearchProviderKind::Custom).custom;
        assert!(prepare_record(&custom, None, &[]).unwrap_err().contains("https://"));
        custom.base_url = Some("https://search.example.com/api".into());
        assert!(prepare_record(&custom, None, &[]).is_ok());
        custom.custom.as_mut().unwrap().title_path = "a..b".into();
        assert!(prepare_record(&custom, None, &[]).is_err());

        let mut linked = input(SearchProviderKind::Deepseek);
        linked.key_source = SearchKeySource::Connection { provider_id: "custom-other".into() };
        assert!(prepare_record(&linked, None, &[connection("custom-other", "https://api.openai.com/v1")]).is_err());

        let mut price = input(SearchProviderKind::Deepseek);
        price.price_search_per_1k = Some(f64::NAN);
        assert!(prepare_record(&price, None, &[]).is_err());
        assert!(clean_key(Some("https://api.deepseek.com")).is_err());
    }

    #[test]
    fn deletes_are_refused_while_a_search_provider_borrows_the_key() {
        let mut linked = record(SearchProviderKind::Deepseek);
        linked.key_source = SearchKeySource::Connection { provider_id: "custom-ds".into() };
        let config = WebConfig { providers: vec![linked], ..WebConfig::default() };
        assert_eq!(providers_linking(&config, "custom-ds"), vec!["DeepSeek".to_string()]);
        assert!(providers_linking(&config, "custom-other").is_empty());
    }

    /// Serves one canned HTTP response on a local port and returns its address.
    fn serve_once(head: &str, body: &str) -> String {
        let response = format!("HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    fn searxng_at(base: String) -> SearchProviderRecord {
        let mut record = record(SearchProviderKind::Searxng);
        record.base_url = Some(base);
        record.key_source = SearchKeySource::None;
        record
    }

    #[test]
    fn calls_refuse_redirects_and_scrub_the_key() {
        tauri::async_runtime::block_on(async {
            let base = serve_once("302 Found\r\nLocation: https://elsewhere.example/", "");
            let record = searxng_at(base);
            let prepared = build_search(&record, None, &query("x"), today()).unwrap();
            let error = call(&record, Op::Search, prepared, None).await.unwrap_err().message();
            assert!(error.contains("redirected"), "{error}");

            let base = serve_once("401 Unauthorized\r\nContent-Type: application/json", r#"{"error":{"message":"bad key sk-abcdefg"}}"#);
            let record = searxng_at(base);
            let prepared = build_search(&record, Some("sk-abcdefg"), &query("x"), today()).unwrap();
            let error = call(&record, Op::Search, prepared, Some("sk-abcdefg")).await.unwrap_err().message();
            assert!(error.contains("rejected the API key") && !error.contains("sk-abcdefg"), "{error}");

            let base = serve_once("200 OK\r\nContent-Type: application/json", r#"{"results":[{"title":"T","url":"https://t.example/"}]}"#);
            let record = searxng_at(base);
            let prepared = build_search(&record, None, &query("x"), today()).unwrap();
            let body = call(&record, Op::Search, prepared, None).await.unwrap();
            assert_eq!(parse_search(&record, &body, 5).unwrap()[0].title, "T");

            let base = serve_once("200 OK\r\nContent-Type: text/html", "<html>");
            let record = searxng_at(base);
            let prepared = build_search(&record, None, &query("x"), today()).unwrap();
            let error = call(&record, Op::Search, prepared, None).await.unwrap_err().message();
            assert!(error.contains("JSON format"), "{error}");
        });
    }

    #[test]
    fn a_cancel_that_arrives_first_still_stops_the_request() {
        tauri::async_runtime::block_on(async {
            let state = WebState::default();
            state.cancel("r1");
            let signal = state.signal("r1");
            let result = unless_cancelled(std::future::pending::<()>(), &signal).await;
            assert!(result.is_none());
            state.finish("r1");
            let signal = state.signal("r2");
            assert_eq!(unless_cancelled(async { 7 }, &signal).await, Some(7));
        });
    }
}
