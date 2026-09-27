//! Per-chat browser preview. The browser is a child `WKWebView`, separate from the renderer:
//! visited pages get an ephemeral data store and no capability entry, while the selected chat's
//! view is positioned over the Browser panel. Hidden views stay attached so background agents
//! can keep rendering, inspect the page, and take snapshots without activating another window.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use block2::RcBlock;
use objc2::{runtime::AnyObject, AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventModifierFlags, NSEventType, NSView,
};
use objc2_foundation::{NSDictionary, NSError, NSPoint, NSString};
use objc2_web_kit::{WKContentWorld, WKSnapshotConfiguration, WKWebView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    webview::{DownloadEvent, PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, Webview,
    WebviewUrl,
};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

const DEFAULT_WIDTH: f64 = 1024.0;
const DEFAULT_HEIGHT: f64 = 768.0;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CONSOLE_ENTRIES: usize = 200;

const INIT_SCRIPT: &str = r#"
(() => {
  // Remote pages have no capability entry. Remove the injected convenience globals as well,
  // so a page cannot mistake the preview for WackCode's trusted renderer.
  try { delete window.__TAURI__; delete window.__TAURI_INTERNALS__; } catch (_) {}
  const state = { console: [], dialogs: [] };
  Object.defineProperty(window, '__wackcodeBrowser', { value: state, configurable: false });
  const keep = (entry) => {
    state.console.push(entry);
    if (state.console.length > 200) state.console.shift();
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level]?.bind(console);
    if (!original) continue;
    console[level] = (...args) => {
      try { keep({ level, text: args.map((value) => typeof value === 'string' ? value : JSON.stringify(value)).join(' ').slice(0, 4000) }); } catch (_) {}
      return original(...args);
    };
  }
  addEventListener('error', (event) => keep({ level: 'error', text: String(event.message || event.error || 'Page error').slice(0, 4000) }));
  addEventListener('unhandledrejection', (event) => keep({ level: 'error', text: `Unhandled promise rejection: ${String(event.reason)}`.slice(0, 4000) }));
  // A modal JavaScript dialog would block a background chat indefinitely. Record it and choose
  // the non-destructive answer; browser_snapshot reports it to the agent and user.
  window.alert = (message) => { state.dialogs.push({ kind: 'alert', message: String(message).slice(0, 4000) }); };
  window.confirm = (message) => { state.dialogs.push({ kind: 'confirm', message: String(message).slice(0, 4000) }); return false; };
  window.prompt = (message, value = '') => { state.dialogs.push({ kind: 'prompt', message: String(message).slice(0, 4000), defaultValue: String(value).slice(0, 1000) }); return null; };
})();
"#;

const SNAPSHOT_SCRIPT: &str = r#"
(() => {
  try {
    const visible = (element) => {
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect();
      return style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) !== 0 && box.width > 0 && box.height > 0 && box.bottom >= 0 && box.right >= 0 && box.top <= innerHeight && box.left <= innerWidth;
    };
    const selector = (element) => {
      if (element.id) return `#${CSS.escape(element.id)}`;
      const parts = [];
      for (let node = element; node && node.nodeType === 1 && node !== document.documentElement; node = node.parentElement) {
        let part = node.localName;
        const siblings = node.parentElement ? [...node.parentElement.children].filter((other) => other.localName === node.localName) : [];
        if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
        parts.unshift(part);
        if (parts.length >= 8) break;
      }
      return parts.join(' > ');
    };
    const candidates = [...document.querySelectorAll('a[href],button,input:not([type=hidden]),textarea,select,[role=button],[role=link],[role=checkbox],[role=menuitem],[tabindex],canvas')]
      .filter(visible).slice(0, 200);
    const elements = candidates.map((element) => {
      const box = element.getBoundingClientRect();
      const type = element.getAttribute('type') || undefined;
      const value = type === 'password' ? undefined : ('value' in element ? String(element.value).slice(0, 500) : undefined);
      return {
        selector: selector(element), tag: element.localName, role: element.getAttribute('role') || undefined,
        name: (element.getAttribute('aria-label') || element.getAttribute('title') || element.innerText || element.getAttribute('placeholder') || '').trim().slice(0, 500),
        type, value, disabled: Boolean(element.disabled || element.getAttribute('aria-disabled') === 'true'),
        checked: 'checked' in element ? Boolean(element.checked) : undefined,
        options: element instanceof HTMLSelectElement ? [...element.options].slice(0, 200).map((option) => ({ label: option.label.slice(0, 500), value: option.value.slice(0, 500) })) : undefined,
        bounds: { x: box.x, y: box.y, width: box.width, height: box.height }
      };
    });
    return { ok: true, title: document.title, url: location.href, viewport: { width: innerWidth, height: innerHeight, scale: devicePixelRatio },
      text: (document.body?.innerText || '').slice(0, 30000), elements, dialogs: window.__wackcodeBrowser?.dialogs || [] };
  } catch (error) { return { ok: false, error: String(error) }; }
})()
"#;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub task_id: String,
    pub exists: bool,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub error: Option<String>,
    pub agent_active: bool,
    pub user_control: bool,
    pub popup: bool,
}

impl BrowserState {
    fn empty(task_id: &str) -> Self {
        Self {
            task_id: task_id.into(),
            exists: false,
            url: String::new(),
            title: String::new(),
            loading: false,
            can_go_back: false,
            can_go_forward: false,
            error: None,
            agent_active: false,
            user_control: false,
            popup: false,
        }
    }
}

#[derive(Debug, Clone)]
struct ElementTarget {
    epoch: u64,
    selector: String,
    x: f64,
    y: f64,
}

struct BrowserSession {
    webview: Webview,
    state: BrowserState,
    visible: bool,
    width: f64,
    height: f64,
    epoch: u64,
    targets: HashMap<String, ElementTarget>,
    history: Vec<String>,
    history_index: usize,
    moving_history: bool,
    return_url: Option<String>,
    control_epoch: u64,
    revealed_to_user: bool,
}

#[derive(Default)]
pub struct BrowserManager {
    sessions: Mutex<HashMap<String, BrowserSession>>,
    operations: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    cancelled: Mutex<HashSet<String>>,
    create_lock: Mutex<()>,
}

impl BrowserManager {
    fn operation(&self, task_id: &str) -> Arc<AsyncMutex<()>> {
        let mut operations = self.operations.lock().expect("browser operation lock");
        operations.entry(task_id.into()).or_default().clone()
    }

    fn state(&self, task_id: &str) -> BrowserState {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(task_id).map(|session| session.state.clone()))
            .unwrap_or_else(|| BrowserState::empty(task_id))
    }

    fn webview(&self, task_id: &str) -> Result<Webview, String> {
        self.sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?
            .get(task_id)
            .map(|session| session.webview.clone())
            .ok_or_else(|| "Open the browser first.".to_string())
    }

    pub fn cancel(&self, request_id: &str) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            cancelled.insert(request_id.into());
        }
    }

    fn is_cancelled(&self, request_id: &str) -> bool {
        self.cancelled
            .lock()
            .map(|set| set.contains(request_id))
            .unwrap_or(true)
    }

    pub fn dispose(&self, task_id: &str) {
        let session = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(task_id));
        if let Some(session) = session {
            let _ = session.webview.close();
        }
        if let Ok(mut operations) = self.operations.lock() {
            operations.remove(task_id);
        }
    }

    pub fn dispose_all(&self) {
        let sessions = self
            .sessions
            .lock()
            .ok()
            .map(|mut sessions| {
                sessions
                    .drain()
                    .map(|(_, session)| session)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session in sessions {
            let _ = session.webview.close();
        }
    }
}

fn emit_state(app: &AppHandle, state: BrowserState, reveal: bool) {
    let task_id = state.task_id.clone();
    let _ = app.emit(
        "worker-event",
        json!({ "type": "browser_state", "taskId": task_id, "browser": state, "reveal": reveal }),
    );
}

fn park_webview(webview: &Webview, width: f64, height: f64) -> Result<(), String> {
    // Keep inactive pages attached and renderable at their last viewport size. The parent view
    // clips this far-offscreen child, so it cannot cover React chrome or application dialogs.
    webview
        .set_bounds(Rect {
            position: Position::Logical(LogicalPosition::new(-10_000.0, -10_000.0)),
            size: Size::Logical(LogicalSize::new(width, height)),
        })
        .map_err(|error| error.to_string())?;
    webview.show().map_err(|error| error.to_string())
}

fn mutate_session(app: &AppHandle, task_id: &str, mutate: impl FnOnce(&mut BrowserSession)) {
    let manager = app.state::<BrowserManager>();
    let state = manager.sessions.lock().ok().and_then(|mut sessions| {
        let session = sessions.get_mut(task_id)?;
        mutate(session);
        Some(session.state.clone())
    });
    if let Some(state) = state {
        emit_state(app, state, false);
    }
}

fn on_page_load(app: &AppHandle, task_id: &str, url: &str, event: PageLoadEvent) {
    mutate_session(app, task_id, |session| {
        session.state.loading = event == PageLoadEvent::Started;
        if url != "about:blank" {
            session.state.url = url.into();
            if event == PageLoadEvent::Started {
                if session.moving_history {
                    session.moving_history = false;
                } else if session
                    .history
                    .get(session.history_index)
                    .map(String::as_str)
                    != Some(url)
                {
                    session
                        .history
                        .truncate(session.history_index.saturating_add(1));
                    session.history.push(url.into());
                    session.history_index = session.history.len().saturating_sub(1);
                }
            }
        }
        if event == PageLoadEvent::Finished {
            session.epoch = session.epoch.wrapping_add(1);
            session.targets.clear();
            session.state.error = None;
        }
        session.state.can_go_back = session.history_index > 0;
        session.state.can_go_forward = session.history_index + 1 < session.history.len();
    });
}

fn ensure_session(app: &AppHandle, task_id: &str) -> Result<Webview, String> {
    let manager = app.state::<BrowserManager>();
    if let Ok(sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get(task_id) {
            return Ok(session.webview.clone());
        }
    }
    let _creating = manager
        .create_lock
        .lock()
        .map_err(|_| "Browser creation lock was poisoned".to_string())?;
    if let Ok(sessions) = manager.sessions.lock() {
        if let Some(session) = sessions.get(task_id) {
            return Ok(session.webview.clone());
        }
    }

    let window = app
        .get_window("main")
        .ok_or_else(|| "The main window is unavailable".to_string())?;
    let label = format!(
        "browser-{}",
        task_id.replace(|character: char| !character.is_ascii_alphanumeric(), "-")
    );
    let load_app = app.clone();
    let load_task = task_id.to_string();
    let title_app = app.clone();
    let title_task = task_id.to_string();
    let popup_app = app.clone();
    let popup_task = task_id.to_string();
    let builder = WebviewBuilder::new(label, WebviewUrl::External("about:blank".parse().unwrap()))
        .incognito(true)
        .initialization_script(INIT_SCRIPT)
        .on_navigation(|url| {
            matches!(url.scheme(), "http" | "https") || url.as_str() == "about:blank"
        })
        .on_download(|_, event| {
            // Downloads and uploads are intentionally outside the first browser-preview release.
            !matches!(event, DownloadEvent::Requested { .. })
        })
        .on_page_load(move |_, payload| {
            on_page_load(
                &load_app,
                &load_task,
                payload.url().as_str(),
                payload.event(),
            )
        })
        .on_document_title_changed(move |_, title| {
            mutate_session(&title_app, &title_task, |session| {
                session.state.title = title.chars().take(500).collect()
            })
        })
        .on_new_window(move |url, _| {
            if matches!(url.scheme(), "http" | "https") {
                let manager = popup_app.state::<BrowserManager>();
                let webview = manager.sessions.lock().ok().and_then(|mut sessions| {
                    let session = sessions.get_mut(&popup_task)?;
                    if session.return_url.is_none() && !session.state.url.is_empty() {
                        session.return_url = Some(session.state.url.clone());
                    }
                    session.state.popup = true;
                    Some(session.webview.clone())
                });
                if let Some(webview) = webview {
                    let _ = webview.navigate(url);
                }
            }
            tauri::webview::NewWindowResponse::Deny
        });
    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(-10_000.0, -10_000.0),
            LogicalSize::new(DEFAULT_WIDTH, DEFAULT_HEIGHT),
        )
        .map_err(|error| format!("Could not create the browser: {error}"))?;
    park_webview(&webview, DEFAULT_WIDTH, DEFAULT_HEIGHT)?;
    let state = BrowserState {
        exists: true,
        ..BrowserState::empty(task_id)
    };
    manager
        .sessions
        .lock()
        .map_err(|_| "Browser lock was poisoned".to_string())?
        .insert(
            task_id.into(),
            BrowserSession {
                webview: webview.clone(),
                state: state.clone(),
                visible: false,
                width: DEFAULT_WIDTH,
                height: DEFAULT_HEIGHT,
                epoch: 0,
                targets: HashMap::new(),
                history: Vec::new(),
                history_index: 0,
                moving_history: false,
                return_url: None,
                control_epoch: 0,
                revealed_to_user: false,
            },
        );
    emit_state(app, state, false);
    Ok(webview)
}

fn normalize_url(raw: &str) -> Result<tauri::Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter a browser URL.".into());
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let url: tauri::Url = candidate
        .parse()
        .map_err(|_| "Enter a valid HTTP or HTTPS URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The browser opens only HTTP and HTTPS URLs.".into());
    }
    Ok(url)
}

fn navigate(
    app: &AppHandle,
    task_id: &str,
    raw: &str,
    by_agent: bool,
) -> Result<BrowserState, String> {
    let url = normalize_url(raw)?;
    let webview = ensure_session(app, task_id)?;
    let manager = app.state::<BrowserManager>();
    let (state, reveal) = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(task_id)
            .ok_or_else(|| "Browser session disappeared".to_string())?;
        session.state.loading = true;
        session.state.error = None;
        // A background chat may open a page without stealing the visible panel. The session is
        // only considered revealed when React actually presents it (`browser_present`).
        let reveal = by_agent && !session.revealed_to_user;
        (session.state.clone(), reveal)
    };
    emit_state(app, state, reveal);
    webview
        .navigate(url)
        .map_err(|error| format!("Could not open that page: {error}"))?;
    Ok(manager.state(task_id))
}

async fn eval_value(webview: &Webview, script: impl Into<String>) -> Result<Value, String> {
    // Run inspection helpers in WebKit's client world. It shares the document but not the
    // page's JavaScript namespace, so page code cannot replace the built-ins these helpers use.
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let script = format!("JSON.stringify({})", script.into());
    webview
        .with_webview(move |platform| unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let world = WKContentWorld::defaultClientWorld(MainThreadMarker::new_unchecked());
            let handler = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                let result = if !error.is_null() {
                    Err(format!("Could not inspect the page: {}", &*error))
                } else if value.is_null() {
                    Err("The page did not return a result.".to_string())
                } else {
                    Ok((&*(value.cast::<NSString>())).to_string())
                };
                if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                    let _ = sender.send(result);
                }
            });
            view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(&script),
                None,
                &world,
                Some(&handler),
            );
        })
        .map_err(|error| format!("Could not inspect the page: {error}"))?;
    let raw = tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The page did not answer in time.".to_string())?
        .map_err(|_| "The page inspection was cancelled.".to_string())??;
    if raw.is_empty() {
        return Err("The page did not return a result.".into());
    }
    serde_json::from_str(&raw).map_err(|_| "The page returned an unreadable result.".to_string())
}

async fn eval_page_value(webview: &Webview, script: impl Into<String>) -> Result<Value, String> {
    // Console and dialog records live in the page world because those hooks must observe page
    // JavaScript. Keep every DOM inspection and interaction helper in `eval_value` above.
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    webview
        .eval_with_callback(script, move |result| {
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(result);
            }
        })
        .map_err(|error| format!("Could not inspect the page: {error}"))?;
    let raw = tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The page did not answer in time.".to_string())?
        .map_err(|_| "The page inspection was cancelled.".to_string())?;
    if raw.is_empty() {
        return Err("The page did not return a result.".into());
    }
    serde_json::from_str(&raw).map_err(|_| "The page returned an unreadable result.".to_string())
}

async fn page_snapshot(app: &AppHandle, task_id: &str) -> Result<Value, String> {
    let webview = app.state::<BrowserManager>().webview(task_id)?;
    let mut value = eval_value(&webview, SNAPSHOT_SCRIPT).await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Could not inspect this page.")
            .to_string());
    }
    let mut targets = HashMap::new();
    let epoch = app
        .state::<BrowserManager>()
        .sessions
        .lock()
        .map_err(|_| "Browser lock was poisoned".to_string())?
        .get(task_id)
        .map(|session| session.epoch)
        .ok_or_else(|| "Browser session disappeared".to_string())?;
    if let Some(elements) = value.get_mut("elements").and_then(Value::as_array_mut) {
        for (index, element) in elements.iter_mut().enumerate() {
            let reference = format!("e{epoch}-{index}");
            if let (Some(selector), Some(bounds)) = (
                element.get("selector").and_then(Value::as_str),
                element.get("bounds"),
            ) {
                let x = bounds.get("x").and_then(Value::as_f64).unwrap_or_default()
                    + bounds
                        .get("width")
                        .and_then(Value::as_f64)
                        .unwrap_or_default()
                        / 2.0;
                let y = bounds.get("y").and_then(Value::as_f64).unwrap_or_default()
                    + bounds
                        .get("height")
                        .and_then(Value::as_f64)
                        .unwrap_or_default()
                        / 2.0;
                targets.insert(
                    reference.clone(),
                    ElementTarget {
                        epoch,
                        selector: selector.into(),
                        x,
                        y,
                    },
                );
            }
            if let Some(object) = element.as_object_mut() {
                object.insert("ref".into(), json!(reference));
                object.remove("selector");
            }
        }
    }
    if let Ok(mut sessions) = app.state::<BrowserManager>().sessions.lock() {
        if let Some(session) = sessions.get_mut(task_id) {
            session.targets = targets;
        }
    }
    Ok(value)
}

async fn native_click(webview: &Webview, x: f64, y: f64) -> Result<(), String> {
    let (sender, receiver) = oneshot::channel();
    webview.with_webview(move |platform| {
        let result = unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let local = NSPoint::new(x, if view.isFlipped() { y } else { view.bounds().size.height - y });
            let window_point = view.convertPoint_toView(local, None);
            let target = view.hitTest(local).unwrap_or_else(|| {
                let view: &NSView = view;
                view.retain()
            });
            let window_number = view.window().map(|window| window.windowNumber()).unwrap_or_default();
            for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind, window_point, NSEventModifierFlags::empty(), 0.0, window_number, None, 0, 1,
                    if kind == NSEventType::LeftMouseDown { 1.0 } else { 0.0 },
                );
                if let Some(event) = event {
                    if kind == NSEventType::LeftMouseDown { target.mouseDown(&event); } else { target.mouseUp(&event); }
                }
            }
            Ok::<(), String>(())
        };
        let _ = sender.send(result);
    }).map_err(|error| error.to_string())?;
    tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The click timed out.".to_string())?
        .map_err(|_| "The click was cancelled.".to_string())?
}

async fn native_hover(webview: &Webview, x: f64, y: f64) -> Result<(), String> {
    let (sender, receiver) = oneshot::channel();
    webview.with_webview(move |platform| {
        let result = unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let local = NSPoint::new(x, if view.isFlipped() { y } else { view.bounds().size.height - y });
            let window_point = view.convertPoint_toView(local, None);
            let target = view.hitTest(local).unwrap_or_else(|| {
                let view: &NSView = view;
                view.retain()
            });
            let window_number = view.window().map(|window| window.windowNumber()).unwrap_or_default();
            if let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::MouseMoved, window_point, NSEventModifierFlags::empty(), 0.0, window_number, None, 0, 0, 0.0,
            ) { target.mouseMoved(&event); }
            Ok::<(), String>(())
        };
        let _ = sender.send(result);
    }).map_err(|error| error.to_string())?;
    tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The hover timed out.".to_string())?
        .map_err(|_| "The hover was cancelled.".to_string())?
}

async fn native_key(
    webview: &Webview,
    characters: String,
    modifiers: NSEventModifierFlags,
    key_code: u16,
) -> Result<(), String> {
    let (sender, receiver) = oneshot::channel();
    webview.with_webview(move |platform| {
        let result = unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let window_number = view.window().map(|window| window.windowNumber()).unwrap_or_default();
            let characters = NSString::from_str(&characters);
            for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
                let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind, NSPoint::new(0.0, 0.0), modifiers, 0.0, window_number, None,
                    &characters, &characters, false, key_code,
                );
                if let Some(event) = event {
                    if kind == NSEventType::KeyDown { view.keyDown(&event); } else { view.keyUp(&event); }
                }
            }
            Ok::<(), String>(())
        };
        let _ = sender.send(result);
    }).map_err(|error| error.to_string())?;
    tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The key press timed out.".to_string())?
        .map_err(|_| "The key press was cancelled.".to_string())?
}

fn key_description(key: &str) -> (String, u16) {
    match key {
        "Enter" => ("\r".into(), 36),
        "Tab" => ("\t".into(), 48),
        "Escape" => ("\u{1b}".into(), 53),
        "Backspace" => ("\u{8}".into(), 51),
        "ArrowLeft" => ("\u{f702}".into(), 123),
        "ArrowRight" => ("\u{f703}".into(), 124),
        "ArrowDown" => ("\u{f701}".into(), 125),
        "ArrowUp" => ("\u{f700}".into(), 126),
        value => (value.into(), 0),
    }
}

fn target_for(app: &AppHandle, task_id: &str, reference: &str) -> Result<ElementTarget, String> {
    let manager = app.state::<BrowserManager>();
    let sessions = manager
        .sessions
        .lock()
        .map_err(|_| "Browser lock was poisoned".to_string())?;
    let session = sessions
        .get(task_id)
        .ok_or_else(|| "Open the browser first.".to_string())?;
    let target = session.targets.get(reference).cloned().ok_or_else(|| {
        "That element reference is stale. Take a new browser snapshot.".to_string()
    })?;
    if target.epoch != session.epoch {
        return Err("That element reference is stale. Take a new browser snapshot.".into());
    }
    Ok(target)
}

async fn resolve_target(webview: &Webview, target: &ElementTarget) -> Result<(f64, f64), String> {
    let selector = serde_json::to_string(&target.selector).map_err(|error| error.to_string())?;
    let script = format!(
        r#"(() => {{ const element = document.querySelector({selector}); if (!element) return {{ ok:false }}; const box = element.getBoundingClientRect(); return {{ ok:true, x:box.x+box.width/2, y:box.y+box.height/2 }}; }})()"#
    );
    let value = eval_value(webview, script).await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err("That element is no longer on the page. Take a new browser snapshot.".into());
    }
    Ok((
        value.get("x").and_then(Value::as_f64).unwrap_or(target.x),
        value.get("y").and_then(Value::as_f64).unwrap_or(target.y),
    ))
}

async fn browser_action(app: &AppHandle, task_id: &str, action: &Value) -> Result<Value, String> {
    let kind = action
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "browser_act needs an action kind.".to_string())?;
    let webview = app.state::<BrowserManager>().webview(task_id)?;
    match kind {
        "click" | "hover" => {
            let (x, y) = if let Some(reference) = action.get("ref").and_then(Value::as_str) {
                let target = target_for(app, task_id, reference)?;
                resolve_target(&webview, &target).await?
            } else {
                (action.get("x").and_then(Value::as_f64).ok_or_else(|| "click needs ref or x/y.".to_string())?,
                 action.get("y").and_then(Value::as_f64).ok_or_else(|| "click needs ref or x/y.".to_string())?)
            };
            if kind == "click" { native_click(&webview, x, y).await?; } else { native_hover(&webview, x, y).await?; }
            Ok(json!({ "ok": true, kind: { "x": x, "y": y } }))
        }
        "fill" => {
            let reference = action.get("ref").and_then(Value::as_str).ok_or_else(|| format!("{kind} needs an element ref."))?;
            let target = target_for(app, task_id, reference)?;
            let (x, y) = resolve_target(&webview, &target).await?;
            native_click(&webview, x, y).await?;
            native_key(&webview, "a".into(), NSEventModifierFlags::Command, 0).await?;
            let value = action.get("value").and_then(Value::as_str).unwrap_or_default();
            if value.is_empty() { native_key(&webview, "\u{8}".into(), NSEventModifierFlags::empty(), 51).await?; }
            else { native_key(&webview, value.into(), NSEventModifierFlags::empty(), 0).await?; }
            Ok(json!({ "ok": true, "filled": reference }))
        }
        "select" => {
            let reference = action.get("ref").and_then(Value::as_str).ok_or_else(|| "select needs an element ref.".to_string())?;
            let target = target_for(app, task_id, reference)?;
            let selector = serde_json::to_string(&target.selector).map_err(|error| error.to_string())?;
            let value = serde_json::to_string(action.get("value").and_then(Value::as_str).unwrap_or_default()).map_err(|error| error.to_string())?;
            let result = eval_value(&webview, format!(r#"(() => {{ const e=document.querySelector({selector}); if(!(e instanceof HTMLSelectElement)) return {{ok:false,error:'The select is missing.'}}; const wanted={value}; const option=[...e.options].find((item) => item.value === wanted) || [...e.options].find((item) => item.label.toLocaleLowerCase() === wanted.toLocaleLowerCase()); if(!option) return {{ok:false,error:'That option is missing.',options:[...e.options].slice(0,200).map((item) => ({{label:item.label,value:item.value}}))}}; e.value=option.value; e.dispatchEvent(new Event('input',{{bubbles:true}})); e.dispatchEvent(new Event('change',{{bubbles:true}})); return {{ok:true,value:e.value,label:option.label}}; }})()"#)).await?;
            if result.get("ok").and_then(Value::as_bool) != Some(true) { return Err(result.get("error").and_then(Value::as_str).unwrap_or("Could not update that select.").into()); }
            Ok(result)
        }
        "scroll" => {
            let x = action.get("x").and_then(Value::as_f64).unwrap_or_default();
            let y = action.get("y").and_then(Value::as_f64).unwrap_or(500.0);
            eval_value(&webview, format!("(() => {{ scrollBy({{left:{x},top:{y},behavior:'instant'}}); return {{ok:true,x:scrollX,y:scrollY}}; }})()" )).await
        }
        "press" => {
            let key = action.get("key").and_then(Value::as_str).ok_or_else(|| "press needs a key.".to_string())?;
            let (characters, key_code) = key_description(key);
            native_key(&webview, characters, NSEventModifierFlags::empty(), key_code).await?;
            Ok(json!({ "ok": true, "key": key }))
        }
        "back" | "forward" | "reload" | "stop" => browser_navigation_inner(app, task_id, kind),
        "handle_dialog" => eval_page_value(&webview, "(() => { const dialogs=window.__wackcodeBrowser?.dialogs || []; const dialog=dialogs.shift(); return {ok:Boolean(dialog),dialog}; })()").await,
        _ => Err(format!("Unknown browser action '{kind}'.")),
    }
}

fn browser_navigation_inner(app: &AppHandle, task_id: &str, action: &str) -> Result<Value, String> {
    let manager = app.state::<BrowserManager>();
    let webview = manager.webview(task_id)?;
    match action {
        "back" => {
            let mut sessions = manager
                .sessions
                .lock()
                .map_err(|_| "Browser lock was poisoned".to_string())?;
            let session = sessions
                .get_mut(task_id)
                .ok_or_else(|| "Open the browser first.".to_string())?;
            if session.history_index == 0 {
                return Ok(json!({ "ok": true }));
            }
            session.history_index -= 1;
            session.moving_history = true;
            webview.eval("history.back()")
        }
        "forward" => {
            let mut sessions = manager
                .sessions
                .lock()
                .map_err(|_| "Browser lock was poisoned".to_string())?;
            let session = sessions
                .get_mut(task_id)
                .ok_or_else(|| "Open the browser first.".to_string())?;
            if session.history_index + 1 >= session.history.len() {
                return Ok(json!({ "ok": true }));
            }
            session.history_index += 1;
            session.moving_history = true;
            webview.eval("history.forward()")
        }
        "reload" => webview.reload(),
        "stop" => webview.with_webview(|platform| unsafe {
            (&*platform.inner().cast::<WKWebView>()).stopLoading();
        }),
        _ => return Err("Unknown browser navigation action.".into()),
    }
    .map_err(|error| error.to_string())?;
    let state = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(task_id)
            .ok_or_else(|| "Open the browser first.".to_string())?;
        session.state.can_go_back = session.history_index > 0;
        session.state.can_go_forward = session.history_index + 1 < session.history.len();
        session.state.clone()
    };
    emit_state(app, state, false);
    Ok(json!({ "ok": true }))
}

async fn screenshot(webview: &Webview) -> Result<(Vec<u8>, u32, u32), String> {
    let size = webview.size().map_err(|error| error.to_string())?;
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    webview
        .with_webview(move |platform| unsafe {
            let view: &WKWebView = &*platform.inner().cast();
            let configuration = WKSnapshotConfiguration::new(MainThreadMarker::new_unchecked());
            configuration.setAfterScreenUpdates(false);
            let handler = RcBlock::new(
                move |image: *mut objc2_app_kit::NSImage, error: *mut objc2_foundation::NSError| {
                    let result = if image.is_null() {
                        let message = if error.is_null() {
                            "WebKit did not return an image.".into()
                        } else {
                            format!("WebKit could not capture the page: {}", &*error)
                        };
                        Err(message)
                    } else {
                        let image = &*image;
                        image
                            .TIFFRepresentation()
                            .and_then(|tiff| {
                                NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                            })
                            .and_then(|bitmap| {
                                bitmap.representationUsingType_properties(
                                    NSBitmapImageFileType::PNG,
                                    &NSDictionary::dictionary(),
                                )
                            })
                            .map(|data| data.as_bytes_unchecked().to_vec())
                            .ok_or_else(|| "WebKit could not encode the page snapshot.".to_string())
                    };
                    if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                        let _ = sender.send(result);
                    }
                },
            );
            view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler);
        })
        .map_err(|error| error.to_string())?;
    let bytes = tokio::time::timeout(OPERATION_TIMEOUT, receiver)
        .await
        .map_err(|_| "The screenshot timed out.".to_string())?
        .map_err(|_| "The screenshot was cancelled.".to_string())??;
    Ok((bytes, size.width, size.height))
}

async fn console_entries(app: &AppHandle, task_id: &str) -> Result<Value, String> {
    let webview = app.state::<BrowserManager>().webview(task_id)?;
    let value = eval_page_value(
        &webview,
        "(() => ({ ok:true, entries:(window.__wackcodeBrowser?.console || []).slice(-200) }))()",
    )
    .await?;
    let mut entries = value
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.len() > MAX_CONSOLE_ENTRIES {
        entries.drain(..entries.len() - MAX_CONSOLE_ENTRIES);
    }
    Ok(json!({ "ok": true, "entries": entries }))
}

pub async fn execute_agent_request(
    app: AppHandle,
    task_id: String,
    request_id: String,
    request: Value,
) -> Result<Value, String> {
    let manager = app.state::<BrowserManager>();
    let operation = manager.operation(&task_id);
    let _guard = operation.lock().await;
    if manager.is_cancelled(&request_id) {
        if let Ok(mut cancelled) = manager.cancelled.lock() {
            cancelled.remove(&request_id);
        }
        return Err("Browser action cancelled.".into());
    }
    let op = request.get("op").and_then(Value::as_str).unwrap_or("");
    if op == "open" {
        ensure_session(&app, &task_id)?;
    } else {
        manager.webview(&task_id)?;
    }
    let control_epoch = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(&task_id)
            .ok_or_else(|| "Browser session disappeared".to_string())?;
        if session.state.user_control {
            return Err(
                "The user has taken control of this browser. Wait until they resume agent control."
                    .into(),
            );
        }
        session.state.agent_active = true;
        let epoch = session.control_epoch;
        emit_state(&app, session.state.clone(), false);
        epoch
    };
    let outcome = match op {
        "open" => request
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| "browser_open needs a URL.".to_string())
            .and_then(|url| {
                navigate(&app, &task_id, url, true).map(|state| json!({ "state": state }))
            }),
        "snapshot" => page_snapshot(&app, &task_id).await,
        "act" => {
            browser_action(
                &app,
                &task_id,
                request.get("action").unwrap_or(&Value::Null),
            )
            .await
        }
        "screenshot" => {
            let webview = manager.webview(&task_id)?;
            let scale = webview
                .window()
                .scale_factor()
                .map_err(|error| error.to_string())?;
            screenshot(&webview).await.map(|(bytes, width, height)| json!({
                "ok": true,
                "image": { "mimeType": "image/png", "data": BASE64.encode(bytes), "width": width, "height": height },
                "viewport": { "width": f64::from(width) / scale, "height": f64::from(height) / scale },
                "coordinateScale": scale,
                "url": manager.state(&task_id).url
            }))
        }
        "console" => console_entries(&app, &task_id).await,
        _ => Err("Unknown browser request.".into()),
    };
    let interrupted = manager.is_cancelled(&request_id)
        || manager
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(&task_id)
                    .map(|session| session.control_epoch != control_epoch)
            })
            .unwrap_or(true);
    let state = manager.sessions.lock().ok().and_then(|mut sessions| {
        let session = sessions.get_mut(&task_id)?;
        session.state.agent_active = false;
        Some(session.state.clone())
    });
    if let Some(state) = state {
        emit_state(&app, state, false);
    }
    if let Ok(mut cancelled) = manager.cancelled.lock() {
        cancelled.remove(&request_id);
    }
    if interrupted {
        Err("Browser action cancelled because the user took control.".into())
    } else {
        outcome
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPresentInput {
    task_id: String,
    visible: bool,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[tauri::command]
pub fn browser_state(app: AppHandle, task_id: String) -> BrowserState {
    app.state::<BrowserManager>().state(&task_id)
}

#[tauri::command]
pub fn browser_present(app: AppHandle, input: BrowserPresentInput) -> Result<BrowserState, String> {
    let manager = app.state::<BrowserManager>();
    if !input.visible {
        let parked = manager.sessions.lock().ok().and_then(|mut sessions| {
            let session = sessions.get_mut(&input.task_id)?;
            session.visible = false;
            Some((session.webview.clone(), session.width, session.height))
        });
        if let Some((webview, width, height)) = parked {
            park_webview(&webview, width, height)?;
        }
        return Ok(manager.state(&input.task_id));
    }
    for value in [input.x, input.y, input.width, input.height] {
        if !value.is_finite() {
            return Err("Browser bounds are invalid.".into());
        }
    }
    if input.width < 1.0 || input.height < 1.0 {
        return Err("Browser bounds are empty.".into());
    }
    let webview = ensure_session(&app, &input.task_id)?;
    webview
        .set_bounds(Rect {
            position: Position::Logical(LogicalPosition::new(input.x, input.y)),
            size: Size::Logical(LogicalSize::new(input.width, input.height)),
        })
        .map_err(|error| error.to_string())?;
    let has_page = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(&input.task_id)
            .ok_or_else(|| "Browser session disappeared".to_string())?;
        session.visible = true;
        session.width = input.width;
        session.height = input.height;
        session.revealed_to_user = true;
        !session.state.url.is_empty()
    };
    if has_page {
        webview.show().map_err(|error| error.to_string())?;
    } else {
        park_webview(&webview, input.width, input.height)?;
    }
    Ok(manager.state(&input.task_id))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOpenInput {
    task_id: String,
    url: String,
}

#[tauri::command]
pub fn browser_open(app: AppHandle, input: BrowserOpenInput) -> Result<BrowserState, String> {
    navigate(&app, &input.task_id, &input.url, false)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNavigationInput {
    task_id: String,
    action: String,
}

#[tauri::command]
pub fn browser_navigation(app: AppHandle, input: BrowserNavigationInput) -> Result<Value, String> {
    browser_navigation_inner(&app, &input.task_id, &input.action)
}

#[tauri::command]
pub fn browser_set_control(
    app: AppHandle,
    task_id: String,
    user_control: bool,
) -> Result<BrowserState, String> {
    let manager = app.state::<BrowserManager>();
    let state = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(&task_id)
            .ok_or_else(|| "Open the browser first.".to_string())?;
        session.state.user_control = user_control;
        session.control_epoch = session.control_epoch.wrapping_add(1);
        session.state.agent_active = false;
        session.state.clone()
    };
    emit_state(&app, state.clone(), false);
    Ok(state)
}

#[tauri::command]
pub fn browser_reset(app: AppHandle, task_id: String) -> Result<BrowserState, String> {
    app.state::<BrowserManager>().dispose(&task_id);
    let state = BrowserState::empty(&task_id);
    emit_state(&app, state.clone(), false);
    Ok(state)
}

#[tauri::command]
pub fn browser_return_from_popup(app: AppHandle, task_id: String) -> Result<BrowserState, String> {
    let manager = app.state::<BrowserManager>();
    let url = {
        let mut sessions = manager
            .sessions
            .lock()
            .map_err(|_| "Browser lock was poisoned".to_string())?;
        let session = sessions
            .get_mut(&task_id)
            .ok_or_else(|| "Open the browser first.".to_string())?;
        session.state.popup = false;
        session
            .return_url
            .take()
            .ok_or_else(|| "There is no parent page to return to.".to_string())?
    };
    navigate(&app, &task_id, &url, false)
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn browser_urls_are_http_only_and_localhost_gets_a_scheme() {
        assert_eq!(
            normalize_url("localhost:5173/settings").unwrap().as_str(),
            "http://localhost:5173/settings"
        );
        assert_eq!(
            normalize_url("https://example.com/").unwrap().scheme(),
            "https"
        );
        assert!(normalize_url("file:///tmp/page.html").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
    }
}
