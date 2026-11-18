//! A tab's state as plain data, folded from the engine's events.
//!
//! The engine (CEF) reports what happens in a tab: address, title and loading changes, dialogs, a
//! crash, and DevTools events. This module turns them into what the pane draws and the agent's
//! tools read. It knows nothing of CEF, so every rule here is tested with recorded events.
//!
//! Invariants:
//! - Console and network history live here, in the engine, never in the page: a navigation does
//!   not wipe them. Each entry carries the `page` it happened on (a counter bumped on every
//!   main-frame navigation), so a reader can ask for the current page only.
//! - Both are ring buffers with a fixed capacity; an entry pushed out is counted in `dropped`,
//!   so a reader can say how much it missed.
//! - Every entry has a sequence number that only grows, so a reader can ask for what is new since
//!   it last looked.
//! - Text is capped (`MAX_TEXT`, `MAX_URL`) so one runaway log line cannot fill memory.

use std::collections::VecDeque;

use serde_json::Value;

pub const CONSOLE_CAPACITY: usize = 500;
pub const NETWORK_CAPACITY: usize = 500;
pub const MAX_TEXT: usize = 8_000;
pub const MAX_URL: usize = 2_000;

fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// A bounded history with sequence numbers.
#[derive(Clone, Debug)]
pub struct Ring<T> {
    items: VecDeque<(u64, T)>,
    capacity: usize,
    next: u64,
    dropped: u64,
}

impl<T> Ring<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity,
            next: 1,
            dropped: 0,
        }
    }

    pub fn push(&mut self, item: T) -> u64 {
        let seq = self.next;
        self.next += 1;
        if self.items.len() == self.capacity {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.items.push_back((seq, item));
        seq
    }

    /// Entries with a sequence number above `after`, oldest first.
    pub fn since(&self, after: u64) -> impl Iterator<Item = (u64, &T)> {
        self.items
            .iter()
            .filter(move |(seq, _)| *seq > after)
            .map(|(seq, item)| (*seq, item))
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.since(0)
    }

    /// The newest entry matching `find`, to update in place.
    pub fn find_last_mut(&mut self, mut find: impl FnMut(&T) -> bool) -> Option<&mut T> {
        self.items
            .iter_mut()
            .rev()
            .map(|(_, item)| item)
            .find(|item| find(item))
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn last_seq(&self) -> u64 {
        self.next - 1
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Log,
    Info,
    Warning,
    Error,
}

impl Level {
    fn from_cdp(name: &str) -> Self {
        match name {
            "debug" | "verbose" => Level::Debug,
            "info" => Level::Info,
            "warning" | "warn" => Level::Warning,
            "error" | "assert" => Level::Error,
            _ => Level::Log,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Log => "log",
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleEntry {
    pub page: u64,
    pub level: Level,
    pub text: String,
    /// `url:line` where it came from, when known.
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkEntry {
    pub page: u64,
    pub request_id: String,
    pub method: String,
    pub url: String,
    /// `Document`, `XHR`, `Fetch`, `Script`…
    pub kind: String,
    pub status: Option<u16>,
    pub mime: Option<String>,
    /// Bytes received, once finished.
    pub size: Option<u64>,
    pub finished: bool,
    /// Why it failed (`net::ERR_…`, or "canceled").
    pub failed: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

/// A JavaScript dialog the page opened, waiting for the user or the agent to answer it.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub kind: DialogKind,
    pub message: String,
    pub default_prompt: String,
}

#[derive(Clone, Debug)]
pub struct TabState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Why the page's process ended, until the next navigation.
    pub crashed: Option<String>,
    /// The page failed to load: `(url, error)`, until the next navigation.
    pub load_error: Option<(String, String)>,
    pub dialog: Option<Dialog>,
    /// Main-frame navigations so far; entries are labelled with it.
    pub page: u64,
    /// How many times the page has taken keyboard focus (a click in it). The pane moves the app's
    /// own focus to itself when this changes, so the app's keys act on the pane, not the composer.
    pub focused: u64,
    pub console: Ring<ConsoleEntry>,
    pub network: Ring<NetworkEntry>,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            crashed: None,
            load_error: None,
            dialog: None,
            page: 0,
            focused: 0,
            console: Ring::new(CONSOLE_CAPACITY),
            network: Ring::new(NETWORK_CAPACITY),
        }
    }
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(key))?
        .as_str()
}

/// One console argument as a person would read it.
fn render_arg(arg: &Value) -> String {
    match arg.get("value") {
        Some(Value::String(text)) => text.clone(),
        Some(value) if !value.is_null() => value.to_string(),
        _ => arg
            .get("description")
            .and_then(Value::as_str)
            .or_else(|| arg.get("unserializableValue").and_then(Value::as_str))
            .or_else(|| arg.get("type").and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
    }
}

fn source(url: Option<&str>, line: Option<i64>) -> Option<String> {
    let url = url.filter(|u| !u.is_empty())?;
    Some(match line {
        Some(line) => format!("{}:{}", cap(url, MAX_URL), line + 1),
        None => cap(url, MAX_URL),
    })
}

impl TabState {
    pub fn set_address(&mut self, url: &str) {
        self.url = cap(url, MAX_URL);
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = cap(title, 300);
    }

    pub fn set_loading(&mut self, loading: bool, can_go_back: bool, can_go_forward: bool) {
        self.loading = loading;
        self.can_go_back = can_go_back;
        self.can_go_forward = can_go_forward;
        if loading {
            self.load_error = None;
        }
    }

    pub fn push_console(&mut self, level: Level, text: &str, source: Option<String>) {
        let entry = ConsoleEntry {
            page: self.page,
            level,
            text: cap(text, MAX_TEXT),
            source,
        };
        self.console.push(entry);
    }

    /// Folds one DevTools event. Unknown events are ignored.
    pub fn apply_cdp(&mut self, method: &str, params: &Value) {
        match method {
            "Page.frameNavigated" => {
                let frame = &params["frame"];
                if frame.get("parentId").is_none() {
                    self.page += 1;
                    self.crashed = None;
                    self.dialog = None;
                    if let Some(url) = frame["url"].as_str() {
                        self.set_address(url);
                    }
                }
            }
            "Runtime.consoleAPICalled" => {
                let level = Level::from_cdp(params["type"].as_str().unwrap_or("log"));
                let text = params["args"]
                    .as_array()
                    .map(|args| args.iter().map(render_arg).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                let frame = params["stackTrace"]["callFrames"].get(0);
                let src = frame.and_then(|f| source(f["url"].as_str(), f["lineNumber"].as_i64()));
                self.push_console(level, &text, src);
            }
            "Runtime.exceptionThrown" => {
                let details = &params["exceptionDetails"];
                let text = details["exception"]["description"]
                    .as_str()
                    .or_else(|| details["text"].as_str())
                    .unwrap_or("Uncaught exception");
                let src = source(details["url"].as_str(), details["lineNumber"].as_i64());
                self.push_console(Level::Error, text, src);
            }
            "Log.entryAdded" => {
                let entry = &params["entry"];
                let level = Level::from_cdp(entry["level"].as_str().unwrap_or("info"));
                let src = source(entry["url"].as_str(), entry["lineNumber"].as_i64());
                self.push_console(level, entry["text"].as_str().unwrap_or(""), src);
            }
            "Network.requestWillBeSent" => {
                let Some(id) = params["requestId"].as_str() else {
                    return;
                };
                let url = str_at(params, &["request", "url"]).unwrap_or("");
                // A redirect reuses the request id: the earlier hop is finished, the new one starts.
                if let Some(earlier) = self
                    .network
                    .find_last_mut(|e| e.request_id == id && !e.finished)
                {
                    earlier.finished = true;
                    earlier.status = params["redirectResponse"]["status"]
                        .as_u64()
                        .map(|s| s as u16)
                        .or(earlier.status);
                }
                let entry = NetworkEntry {
                    page: self.page,
                    request_id: id.to_string(),
                    method: str_at(params, &["request", "method"])
                        .unwrap_or("GET")
                        .to_string(),
                    url: cap(url, MAX_URL),
                    kind: params["type"].as_str().unwrap_or("Other").to_string(),
                    status: None,
                    mime: None,
                    size: None,
                    finished: false,
                    failed: None,
                };
                self.network.push(entry);
            }
            "Network.responseReceived" => {
                let Some(id) = params["requestId"].as_str() else {
                    return;
                };
                if let Some(entry) = self.network.find_last_mut(|e| e.request_id == id) {
                    entry.status = params["response"]["status"].as_u64().map(|s| s as u16);
                    entry.mime = params["response"]["mimeType"].as_str().map(str::to_string);
                }
            }
            "Network.loadingFinished" => {
                let Some(id) = params["requestId"].as_str() else {
                    return;
                };
                if let Some(entry) = self.network.find_last_mut(|e| e.request_id == id) {
                    entry.finished = true;
                    entry.size = params["encodedDataLength"].as_f64().map(|n| n as u64);
                }
            }
            "Network.loadingFailed" => {
                let Some(id) = params["requestId"].as_str() else {
                    return;
                };
                if let Some(entry) = self.network.find_last_mut(|e| e.request_id == id) {
                    entry.finished = true;
                    entry.failed = Some(if params["canceled"].as_bool() == Some(true) {
                        "canceled".to_string()
                    } else {
                        params["errorText"].as_str().unwrap_or("failed").to_string()
                    });
                }
            }
            _ => {}
        }
    }

    /// Console errors since `after`: what an action's result reports as new.
    pub fn errors_since(&self, after: u64) -> usize {
        self.console
            .since(after)
            .filter(|(_, e)| e.level == Level::Error)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rings_drop_the_oldest_and_count_it() {
        let mut ring = Ring::new(3);
        for n in 0..5 {
            ring.push(n);
        }
        assert_eq!(
            ring.iter().map(|(_, n)| *n).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(ring.dropped(), 2);
        assert_eq!(ring.last_seq(), 5);
        assert_eq!(ring.since(4).map(|(_, n)| *n).collect::<Vec<_>>(), vec![4]);
    }

    #[test]
    fn console_events_become_readable_lines() {
        let mut tab = TabState::default();
        tab.apply_cdp(
            "Runtime.consoleAPICalled",
            &json!({"type": "warning", "args": [{"type": "string", "value": "slow"}, {"type": "number", "value": 3}, {"type": "object", "description": "Object"}],
                    "stackTrace": {"callFrames": [{"url": "http://localhost:5173/app.js", "lineNumber": 9}]}}),
        );
        tab.apply_cdp(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails": {"text": "Uncaught", "exception": {"description": "TypeError: x is undefined"}, "url": "http://localhost:5173/app.js", "lineNumber": 41}}),
        );
        tab.apply_cdp("Log.entryAdded", &json!({"entry": {"level": "error", "text": "Failed to load resource: 404", "url": "http://localhost:5173/missing.png"}}));
        let lines: Vec<_> = tab
            .console
            .iter()
            .map(|(_, e)| (e.level, e.text.clone(), e.source.clone()))
            .collect();
        assert_eq!(
            lines[0],
            (
                Level::Warning,
                "slow 3 Object".into(),
                Some("http://localhost:5173/app.js:10".into())
            )
        );
        assert_eq!(
            lines[1],
            (
                Level::Error,
                "TypeError: x is undefined".into(),
                Some("http://localhost:5173/app.js:42".into())
            )
        );
        assert_eq!(lines[2].0, Level::Error);
        assert_eq!(tab.errors_since(1), 2);
    }

    #[test]
    fn history_survives_navigation_labelled_by_page() {
        let mut tab = TabState::default();
        tab.apply_cdp(
            "Page.frameNavigated",
            &json!({"frame": {"id": "1", "url": "http://localhost:3000/"}}),
        );
        tab.push_console(Level::Log, "first page", None);
        tab.apply_cdp(
            "Page.frameNavigated",
            &json!({"frame": {"id": "2", "parentId": "1", "url": "http://ads.example/"}}),
        );
        assert_eq!(tab.page, 1, "subframes are not navigations of the page");
        tab.apply_cdp(
            "Page.frameNavigated",
            &json!({"frame": {"id": "1", "url": "http://localhost:3000/next"}}),
        );
        tab.push_console(Level::Log, "second page", None);
        assert_eq!(tab.page, 2);
        assert_eq!(tab.url, "http://localhost:3000/next");
        let pages: Vec<_> = tab.console.iter().map(|(_, e)| e.page).collect();
        assert_eq!(pages, vec![1, 2]);
    }

    #[test]
    fn network_requests_follow_their_lifecycle_and_redirects() {
        let mut tab = TabState::default();
        tab.apply_cdp("Network.requestWillBeSent", &json!({"requestId": "7", "type": "Fetch", "request": {"url": "http://localhost:3000/api", "method": "POST"}}));
        tab.apply_cdp(
            "Network.responseReceived",
            &json!({"requestId": "7", "response": {"status": 500, "mimeType": "application/json"}}),
        );
        tab.apply_cdp(
            "Network.loadingFinished",
            &json!({"requestId": "7", "encodedDataLength": 120.0}),
        );
        tab.apply_cdp("Network.requestWillBeSent", &json!({"requestId": "8", "type": "Document", "request": {"url": "http://a.example/", "method": "GET"}}));
        tab.apply_cdp("Network.requestWillBeSent", &json!({"requestId": "8", "type": "Document", "request": {"url": "https://a.example/", "method": "GET"}, "redirectResponse": {"status": 301}}));
        tab.apply_cdp(
            "Network.loadingFailed",
            &json!({"requestId": "8", "errorText": "net::ERR_CONNECTION_REFUSED"}),
        );
        let entries: Vec<_> = tab.network.iter().map(|(_, e)| e.clone()).collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            (
                entries[0].method.as_str(),
                entries[0].status,
                entries[0].size
            ),
            ("POST", Some(500), Some(120))
        );
        assert_eq!((entries[1].status, entries[1].finished), (Some(301), true));
        assert_eq!(
            entries[2].failed.as_deref(),
            Some("net::ERR_CONNECTION_REFUSED")
        );
    }

    #[test]
    fn text_is_capped_on_a_char_boundary() {
        let mut tab = TabState::default();
        tab.push_console(Level::Log, &"é".repeat(MAX_TEXT), None);
        let (_, entry) = tab.console.iter().next().unwrap();
        assert!(entry.text.len() <= MAX_TEXT + '…'.len_utf8());
        assert!(entry.text.ends_with('…'));
    }
}
