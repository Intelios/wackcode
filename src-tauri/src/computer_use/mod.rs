//! Computer use: the agent observes and operates native app windows, mainly to verify apps the
//! user is building. This module is the security boundary; the worker's tools
//! (`worker/src/builtin/computer-use/`) only describe requests. Every request re-checks, in
//! order: the setting and macOS support, the permission it needs, the target's block status
//! (`policy`), and this chat's grant — raising the access card itself when there is none. The
//! card is answered renderer → `computer_use_respond_access`, never through the worker.
//!
//! Background first: element actions go through Accessibility and keys go only to the target
//! process. Only pointer actions Accessibility can't do use the real cursor (`input::Foreground`).
//!
//! A chat has a computer-use *session* from its first request in a run until that run ends.
//! While any session exists the ⌃⌥⌘. stop shortcut is registered and the menu bar offers
//! "Stop Computer Use".

mod apps;
mod ax;
mod capture;
mod engine;
mod geometry;
mod input;
mod keys;
mod outline;
mod permissions;
pub mod policy;
mod request;

use crate::models::ComputerUseConfig;
use crate::storage::MetadataState;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use engine::{CaptureGeometry, Engine, EngineState, Observation, StopToken};
use policy::{AppIdentity, AppKey, Grant, GrantStore, OwnIdentity};
use request::{Action, Button, ComputerRequest, Expect, Target};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

const STOPPED: &str = "Computer use was stopped.";
const OPERATION_TIMEOUT: Duration = Duration::from_secs(45);
const LAUNCH_WINDOW_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccessDecision {
    Allow,
    Deny,
    Never,
    Cancelled,
}

struct PendingAccess {
    task_id: String,
    identity: AppIdentity,
    reply: oneshot::Sender<AccessDecision>,
}

#[derive(Default)]
struct Inner {
    grants: GrantStore,
    /// Chats with a computer-use session, and the app each last used.
    sessions: HashMap<String, String>,
    /// In-flight requests: request id → (chat, stop token).
    requests: HashMap<String, (String, StopToken)>,
    /// Requests cancelled before they started.
    cancelled: std::collections::HashSet<String>,
    /// Open access cards by their request id.
    access: HashMap<String, PendingAccess>,
    hotkey_registered: bool,
    hotkey_available: bool,
}

pub struct ComputerUseManager {
    inner: Mutex<Inner>,
    operations: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    engine: OnceLock<Engine>,
    own: OwnIdentity,
}

impl Default for ComputerUseManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner { hotkey_available: true, ..Inner::default() }),
            operations: Mutex::new(HashMap::new()),
            engine: OnceLock::new(),
            own: OwnIdentity::current(),
        }
    }
}

impl ComputerUseManager {
    fn engine(&self) -> &Engine {
        self.engine.get_or_init(Engine::start)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
        self.inner.lock().map_err(|_| "The computer-use lock was poisoned.".to_string())
    }

    fn operation(&self, task_id: &str) -> Arc<AsyncMutex<()>> {
        let mut operations = self.operations.lock().expect("computer-use operation lock");
        operations.entry(task_id.into()).or_default().clone()
    }

    /// Whether any chat has a session (the menu bar shows "Stop Computer Use").
    pub fn any_session(&self) -> bool {
        self.inner.lock().map(|inner| !inner.sessions.is_empty()).unwrap_or(false)
    }

    /// The worker cancelled a request (Stop, or the tool's abort signal).
    pub fn cancel(&self, request_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            match inner.requests.get(request_id) {
                Some((_, token)) => token.stop(),
                None => {
                    inner.cancelled.insert(request_id.to_string());
                }
            }
        }
    }

    /// Stops everything a chat has in flight; returns the replies of its open cards to cancel
    /// outside the lock.
    fn stop_chat(inner: &mut Inner, task_id: &str) -> Vec<oneshot::Sender<AccessDecision>> {
        for (task, token) in inner.requests.values() {
            if task == task_id {
                token.stop();
            }
        }
        let ids = inner.access.iter().filter(|(_, pending)| pending.task_id == task_id).map(|(id, _)| id.clone()).collect::<Vec<_>>();
        ids.into_iter().filter_map(|id| inner.access.remove(&id)).map(|pending| pending.reply).collect()
    }
}

fn cancel_replies(replies: Vec<oneshot::Sender<AccessDecision>>) {
    for reply in replies {
        let _ = reply.send(AccessDecision::Cancelled);
    }
}

/// Events for the renderer ride `worker-event`, like the worker's own; the menu bar sees them
/// first so a pending card shows the chat as waiting.
fn emit(app: &AppHandle, task_id: &str, value: Value) {
    crate::menu_bar::handle_worker_event(app, task_id, &value);
    let _ = app.emit("worker-event", value);
}

fn config(app: &AppHandle) -> ComputerUseConfig {
    app.state::<MetadataState>().data.lock().map(|data| data.computer_use.clone()).unwrap_or_default()
}

fn app_json(identity: &AppIdentity) -> Value {
    json!({ "name": identity.name, "bundleId": identity.bundle_id, "pid": identity.pid })
}

fn window_json(window: &ax::AxWindow, number: Option<u32>) -> Value {
    let frame = window.frame.unwrap_or(geometry::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
    json!({
        "id": number, "title": window.title, "x": frame.x, "y": frame.y, "width": frame.width, "height": frame.height,
        "main": window.main, "onScreen": !window.minimized
    })
}

// ---------------------------------------------------------------------------------------------
// Requests

pub async fn execute_agent_request(app: AppHandle, task_id: String, request_id: String, request: Value) -> Result<Value, String> {
    let manager = app.state::<ComputerUseManager>();
    let parsed = request::parse(&request)?;
    let operation = manager.operation(&task_id);
    let _guard = operation.lock().await;
    let token = StopToken::default();
    {
        let mut inner = manager.lock()?;
        if inner.cancelled.remove(&request_id) {
            return Err(STOPPED.into());
        }
        inner.requests.insert(request_id.clone(), (task_id.clone(), token.clone()));
    }
    let result = match tokio::time::timeout(OPERATION_TIMEOUT, run(&app, &manager, &task_id, parsed, &token)).await {
        Ok(result) => result,
        Err(_) => {
            token.stop();
            Err("The computer-use action took too long and was stopped.".into())
        }
    };
    if let Ok(mut inner) = manager.lock() {
        inner.requests.remove(&request_id);
    }
    if token.is_stopped() && result.is_ok() {
        return Err(STOPPED.into());
    }
    result
}

async fn run(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, request: ComputerRequest, token: &StopToken) -> Result<Value, String> {
    let settings = config(app);
    if !settings.enabled {
        return Err("Computer use is switched off in Settings › Packages.".into());
    }
    if !permissions::is_supported() {
        return Err("Computer use needs macOS 14 or later.".into());
    }
    if !permissions::accessibility() {
        return Err("WackCode doesn't have Accessibility permission, which computer use needs. Ask the user to allow it in Settings › Computer use.".into());
    }
    match request {
        ComputerRequest::Apps => list_apps(manager, task_id, settings).await,
        ComputerRequest::Open { app: query } => open(app, manager, task_id, &query, settings, token).await,
        ComputerRequest::Snapshot { app: query, window } => {
            let target = running_target(app, manager, task_id, &query, settings, token).await?;
            begin_session(app, manager, task_id, &target.name);
            snapshot(manager, task_id, target, window).await
        }
        ComputerRequest::Screenshot { app: query, window } => {
            if !permissions::screen_recording_preflight() {
                return Err("WackCode doesn't have Screen Recording permission, so it can't capture windows. Use computer_snapshot, or ask the user to allow it in Settings › Computer use.".into());
            }
            let target = running_target(app, manager, task_id, &query, settings, token).await?;
            begin_session(app, manager, task_id, &target.name);
            screenshot(manager, task_id, target, window).await
        }
        ComputerRequest::Act { app: query, state_id, actions, expect } => {
            let never_allow = settings.never_allow.clone();
            let target = running_target(app, manager, task_id, &query, settings, token).await?;
            begin_session(app, manager, task_id, &target.name);
            act(manager, task_id, target, state_id, actions, expect, never_allow, token.clone()).await
        }
    }
}

/// Resolves `query`, refuses blocked apps, and makes sure this chat may use it.
async fn resolve_target(
    app: &AppHandle,
    manager: &ComputerUseManager,
    task_id: &str,
    query: &str,
    settings: &ComputerUseConfig,
    token: &StopToken,
    launch: bool,
) -> Result<apps::Resolved, String> {
    let home = app.path().home_dir().ok();
    let query = query.to_string();
    let resolved = manager.engine().run(move |_| apps::resolve(&query, home.as_deref())).await??;
    let identity = match &resolved {
        apps::Resolved::Running(running) => running.identity(),
        apps::Resolved::Installed(installed) => installed.identity(),
    };
    if let Some(reason) = policy::block_reason(&identity, &manager.own, &settings.never_allow) {
        return Err(reason);
    }
    if !launch {
        if let apps::Resolved::Installed(installed) = &resolved {
            return Err(format!("{} isn't running. Open it with computer_open first.", installed.name));
        }
    }
    ensure_access(app, manager, task_id, &identity, launch && matches!(resolved, apps::Resolved::Installed(_)), token).await?;
    Ok(resolved)
}

async fn running_target(
    app: &AppHandle,
    manager: &ComputerUseManager,
    task_id: &str,
    query: &str,
    settings: ComputerUseConfig,
    token: &StopToken,
) -> Result<AppIdentity, String> {
    match resolve_target(app, manager, task_id, query, &settings, token, false).await? {
        apps::Resolved::Running(running) => Ok(running.identity()),
        apps::Resolved::Installed(installed) => Err(format!("{} isn't running. Open it with computer_open first.", installed.name)),
    }
}

async fn ensure_access(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, identity: &AppIdentity, launch: bool, token: &StopToken) -> Result<(), String> {
    let key = AppKey::of(identity);
    match manager.lock()?.grants.decision(task_id, &key) {
        Grant::Allowed => return Ok(()),
        Grant::Denied => return Err(format!("The user denied access to {} for this chat. Ask them before trying again.", identity.name)),
        Grant::Unknown => {}
    }
    let icon = match identity.path.clone() {
        Some(path) => manager.engine().run(move |_| apps::icon_data_url(&path)).await.ok().flatten(),
        None => None,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let (reply, mut receiver) = oneshot::channel();
    manager.lock()?.access.insert(request_id.clone(), PendingAccess { task_id: task_id.to_string(), identity: identity.clone(), reply });
    emit(app, task_id, json!({
        "type": "computer_access_request", "taskId": task_id, "requestId": request_id, "launch": launch,
        "app": { "name": identity.name, "bundleId": identity.bundle_id, "path": identity.path, "icon": icon },
    }));
    // Like ask_user_question, the card waits as long as the user needs; a stop cancels it.
    let decision = loop {
        match tokio::time::timeout(Duration::from_millis(200), &mut receiver).await {
            Ok(decision) => break decision.unwrap_or(AccessDecision::Cancelled),
            Err(_) if token.is_stopped() => {
                if let Ok(mut inner) = manager.lock() {
                    inner.access.remove(&request_id);
                }
                break AccessDecision::Cancelled;
            }
            Err(_) => {}
        }
    };
    emit(app, task_id, json!({ "type": "computer_access_resolved", "taskId": task_id, "requestId": request_id, "decision": decision }));
    match decision {
        AccessDecision::Allow => {
            manager.lock()?.grants.allow(task_id, key);
            Ok(())
        }
        AccessDecision::Deny | AccessDecision::Never => {
            manager.lock()?.grants.deny(task_id, key);
            Err(if decision == AccessDecision::Never {
                format!("The user never allows computer use in {}. Don't try again; ask them what they'd like instead.", identity.name)
            } else {
                format!("The user denied access to {} for this chat. Don't try again; ask them what they'd like instead.", identity.name)
            })
        }
        AccessDecision::Cancelled => Err(STOPPED.into()),
    }
}

async fn list_apps(manager: &ComputerUseManager, task_id: &str, settings: ComputerUseConfig) -> Result<Value, String> {
    let (allowed, denied) = {
        let inner = manager.lock()?;
        (inner.grants.allowed(task_id), inner.grants.denied(task_id))
    };
    let own = manager.own.clone();
    let apps = manager
        .engine()
        .run(move |_| {
            let mut list = apps::running()
                .into_iter()
                .filter(|running| running.pid != own.pid)
                .map(|running| {
                    let identity = running.identity();
                    let key = AppKey::of(&identity);
                    let access = if policy::block_reason(&identity, &own, &settings.never_allow).is_some() {
                        "blocked"
                    } else if allowed.contains(&key) {
                        "granted"
                    } else if denied.contains(&key) {
                        "denied"
                    } else {
                        "ask"
                    };
                    // Window titles can be private (mail subjects, document names): only apps
                    // this chat may use show theirs.
                    let windows = if access == "granted" {
                        ax::windows(&ax::application(running.pid))
                            .iter()
                            .map(|window| window_json(window, input::window_number(&window.element, running.pid)))
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    (access == "granted", json!({
                        "name": running.name, "bundleId": running.bundle_id, "pid": running.pid,
                        "frontmost": running.frontmost, "access": access, "windows": windows,
                    }))
                })
                .collect::<Vec<_>>();
            list.sort_by_key(|(granted, _)| !granted);
            list.into_iter().map(|(_, value)| value).collect::<Vec<_>>()
        })
        .await?;
    Ok(json!({ "apps": apps }))
}

async fn open(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, query: &str, settings: ComputerUseConfig, token: &StopToken) -> Result<Value, String> {
    let resolved = resolve_target(app, manager, task_id, query, &settings, token, true).await?;
    let (identity, launched) = match resolved {
        apps::Resolved::Running(running) => (running.identity(), false),
        apps::Resolved::Installed(installed) => {
            let path = installed.path.clone();
            let pid = manager.engine().run(move |_| apps::launch(&path)).await??;
            let mut identity = installed.identity();
            identity.pid = Some(pid);
            (identity, true)
        }
    };
    begin_session(app, manager, task_id, &identity.name);
    let pid = identity.pid.ok_or("The app has no process.")?;
    let deadline = Instant::now() + if launched { LAUNCH_WINDOW_WAIT } else { Duration::ZERO };
    let windows = loop {
        let windows = manager
            .engine()
            .run(move |_| {
                let app_element = ax::application(pid);
                ax::enable_manual_accessibility(&app_element);
                ax::windows(&app_element).iter().map(|window| window_json(window, input::window_number(&window.element, pid))).collect::<Vec<_>>()
            })
            .await?;
        if !windows.is_empty() || Instant::now() >= deadline || token.is_stopped() {
            break windows;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    Ok(json!({ "app": app_json(&identity), "launched": launched, "windows": windows }))
}

/// The app's windows with their window-server numbers.
fn numbered_windows(app_element: &ax::Element, pid: i32) -> Vec<(ax::AxWindow, Option<u32>)> {
    ax::windows(app_element)
        .into_iter()
        .map(|window| {
            let number = input::window_number(&window.element, pid);
            (window, number)
        })
        .collect()
}

/// Picks the requested window (by number) or the app's main window.
fn pick_window(pid: i32, name: &str, requested: Option<u32>) -> Result<(ax::AxWindow, Option<u32>, Vec<Value>), String> {
    let app_element = ax::application(pid);
    ax::enable_manual_accessibility(&app_element);
    let windows = numbered_windows(&app_element, pid);
    if windows.is_empty() {
        return Err(format!(
            "{name} has no windows. Take a computer_snapshot of it for an app-level stateId, then open one with a computer_act menu action (for example [\"File\", \"New\"])."
        ));
    }
    let listed = windows.iter().map(|(window, number)| window_json(window, *number)).collect::<Vec<_>>();
    let index = match requested {
        Some(id) => windows.iter().position(|(_, number)| *number == Some(id)).ok_or_else(|| format!("Window {id} isn't one of {name}'s windows. Its windows: {}.", Value::Array(listed.clone())))?,
        None => windows.iter().position(|(window, _)| !window.minimized).unwrap_or(0),
    };
    let (window, number) = windows.into_iter().nth(index).expect("index is in range");
    Ok((window, number, listed))
}

async fn snapshot(manager: &ComputerUseManager, task_id: &str, target: AppIdentity, window: Option<u32>) -> Result<Value, String> {
    let task = task_id.to_string();
    manager
        .engine()
        .run(move |state: &mut EngineState| {
            let pid = target.pid.ok_or("The app has no process.")?;
            let app_element = ax::application(pid);
            ax::enable_manual_accessibility(&app_element);
            // A running app with no windows (just launched, or closed its last one) still has a
            // menu bar and keyboard: an app-level state lets the agent open a window.
            if window.is_none() && numbered_windows(&app_element, pid).is_empty() {
                let id = state.next_id();
                let state_id = format!("s{id}");
                let menus = ax::menu_titles(&app_element);
                let chat = state.chats.entry(task).or_default();
                chat.insert((pid, 0), Observation { id: state_id.clone(), pid, window: app_element, ref_prefix: None, refs: Vec::new(), capture: None });
                let outline = format!(
                    "(no windows — this stateId covers the app: use it with menu, keypress or wait)\nMenu bar: {}\n",
                    if menus.is_empty() { "(none)".to_string() } else { menus.join(", ") }
                );
                return Ok(json!({
                    "stateId": state_id, "app": app_json(&target), "window": Value::Null, "windows": [],
                    "outline": outline, "elements": 0, "truncated": false,
                }));
            }
            let (window, number, windows) = pick_window(pid, &target.name, window)?;
            let (tree, elements) = ax::walk(&window.element, &ax::SNAPSHOT_LIMITS);
            let id = state.next_id();
            let prefix = format!("e{id}");
            let outline = outline::render(&tree, &prefix, outline::MAX_OUTLINE_BYTES);
            let refs = outline.refs.iter().map(|handle| elements[*handle].clone()).collect::<Vec<_>>();
            let key = (pid, number.unwrap_or(0));
            let chat = state.chats.entry(task).or_default();
            // A screenshot of the same, unmoved window stays usable for coordinates.
            let capture = chat.remove(&key).and_then(|previous| previous.capture).filter(|capture| window.frame.is_some_and(|frame| frame.approx_eq(&capture.frame)));
            let state_id = format!("s{id}");
            let elements_count = refs.len();
            chat.insert(key, Observation { id: state_id.clone(), pid, window: window.element.clone(), ref_prefix: Some(prefix), refs, capture });
            Ok(json!({
                "stateId": state_id, "app": app_json(&target), "window": window_json(&window, number), "windows": windows,
                "outline": outline.text, "elements": elements_count, "truncated": outline.truncated,
            }))
        })
        .await?
}

async fn screenshot(manager: &ComputerUseManager, task_id: &str, target: AppIdentity, window: Option<u32>) -> Result<Value, String> {
    let pid = target.pid.ok_or("The app has no process.")?;
    let name = target.name.clone();
    let (window_value, number) = manager
        .engine()
        .run(move |_| -> Result<(Value, u32), String> {
            let (window, number, _) = pick_window(pid, &name, window)?;
            let number = number.ok_or("That window can't be identified for capture.")?;
            Ok((window_json(&window, Some(number)), number))
        })
        .await??;
    let capture = capture::window(number).await?;
    let task = task_id.to_string();
    let geometry = CaptureGeometry { frame: capture.frame, output: (capture.width, capture.height) };
    let state_id = manager
        .engine()
        .run(move |state: &mut EngineState| -> Result<String, String> {
            let app_element = ax::application(pid);
            let window = ax::windows(&app_element)
                .into_iter()
                .find(|window| input::window_number(&window.element, pid) == Some(number))
                .ok_or("The window closed while it was being captured.")?;
            let id = format!("s{}", state.next_id());
            let chat = state.chats.entry(task).or_default();
            // The last snapshot's refs stay valid: they are re-checked when used.
            let previous = chat.remove(&(pid, number));
            let (ref_prefix, refs) = previous.map(|previous| (previous.ref_prefix, previous.refs)).unwrap_or_default();
            chat.insert((pid, number), Observation { id: id.clone(), pid, window: window.element, ref_prefix, refs, capture: Some(geometry) });
            Ok(id)
        })
        .await??;
    Ok(json!({
        "stateId": state_id, "app": app_json(&target), "window": window_value,
        "image": { "mimeType": "image/jpeg", "data": BASE64.encode(&capture.jpeg), "width": capture.width, "height": capture.height },
        "coordinateScale": geometry::coordinate_scale(&capture.frame, capture.width),
    }))
}

// ---------------------------------------------------------------------------------------------
// Acting

struct ActContext<'a> {
    pid: i32,
    own_pid: i32,
    observation: &'a Observation,
    app_element: ax::Element,
    foreground: Option<input::Foreground>,
    never_allow: Vec<String>,
    token: StopToken,
}

impl ActContext<'_> {
    fn element(&self, reference: &str) -> Result<ax::Element, String> {
        let prefix = self.observation.ref_prefix.as_deref().ok_or("This state has no element refs. Take a computer_snapshot first.")?;
        let index = outline::parse_ref(reference, prefix).ok_or_else(|| format!("Ref {reference} isn't from this state's snapshot (its refs look like {prefix}-0)."))?;
        let element = self.observation.refs.get(index).ok_or_else(|| format!("Ref {reference} doesn't exist."))?;
        if !ax::is_alive(element) || ax::pid(element) != Some(self.pid) {
            return Err(format!("Ref {reference} no longer exists. Take a new snapshot."));
        }
        Ok(element.clone())
    }

    fn foreground(&mut self) -> Result<&mut input::Foreground, String> {
        if self.foreground.is_none() {
            let own = OwnIdentity { pid: self.own_pid, ..OwnIdentity::default() };
            let never_allow = self.never_allow.clone();
            let blocked = Box::new(move |bundle_id: Option<&str>| {
                let identity = AppIdentity { pid: None, name: String::new(), bundle_id: bundle_id.map(str::to_string), path: None };
                policy::block_reason(&identity, &own, &never_allow).is_some()
            });
            let token = self.token.clone();
            self.foreground = Some(input::Foreground::begin(self.pid, self.own_pid, self.observation.window.clone(), blocked, &move || token.is_stopped())?);
        }
        Ok(self.foreground.as_mut().expect("just set"))
    }

    /// A screenshot pixel as a screen point, refusing if the window moved since the capture.
    fn pixel(&self, x: f64, y: f64) -> Result<(f64, f64), String> {
        let capture = self.observation.capture.ok_or("Coordinates need a stateId from computer_screenshot.")?;
        let frame = ax::frame(&self.observation.window).ok_or("The window is gone. Take a new screenshot.")?;
        if !frame.approx_eq(&capture.frame) {
            return Err("The window moved or was resized since the screenshot. Take a new one.".into());
        }
        geometry::pixel_to_point(&capture.frame, capture.output, x, y).ok_or_else(|| format!("({x}, {y}) is outside the {}×{} screenshot.", capture.output.0, capture.output.1))
    }

    fn point(&self, target: &Target) -> Result<(f64, f64), String> {
        match target {
            Target::Pixel(x, y) => self.pixel(*x, *y),
            Target::Ref(reference) => {
                let element = self.element(reference)?;
                ax::frame(&element).map(|frame| frame.center()).ok_or_else(|| format!("Ref {reference} has no position on screen."))
            }
        }
    }

    fn stopped(&self) -> bool {
        self.token.is_stopped()
    }

    /// Runs one action; `Ok((via, note))`.
    fn perform(&mut self, action: &Action) -> Result<(&'static str, Option<String>), String> {
        match action {
            Action::Press { target } => {
                let element = self.element(target)?;
                press(&element)?;
                Ok(("ax", None))
            }
            Action::Click { target, button, count } => {
                if let (Target::Ref(reference), 1) = (target, *count) {
                    let element = self.element(reference)?;
                    let background = match button {
                        Button::Left => press(&element).is_ok(),
                        Button::Right => ax::perform(&element, "AXShowMenu").is_ok(),
                        Button::Middle => false,
                    };
                    if background {
                        return Ok(("ax", None));
                    }
                }
                let point = self.point(target)?;
                let button = match button {
                    Button::Left => input::Button::Left,
                    Button::Right => input::Button::Right,
                    Button::Middle => input::Button::Middle,
                };
                let count = *count;
                self.foreground()?.click(point, button, count)?;
                Ok(("foreground", None))
            }
            Action::SetText { target, text } => {
                let element = self.element(target)?;
                if ax::is_secure(&element) {
                    return Err("That's a secure text field; computer use never types into one.".into());
                }
                ax::set_string(&element, "AXValue", text).map_err(|error| ax::describe_error(error, "setting the text"))?;
                let note = (ax::string(&element, "AXValue").as_deref() != Some(text.as_str())).then(|| "the field reads back a different value".to_string());
                Ok(("ax", note))
            }
            Action::TypeText { target, text } => {
                let element = match target {
                    Some(reference) => Some(self.element(reference)?),
                    None => ax::element(&self.app_element, "AXFocusedUIElement"),
                };
                if element.as_ref().is_some_and(|element| ax::is_secure(element)) || ax::focused_is_secure(&self.app_element) {
                    return Err("A secure text field has focus; computer use never types into one.".into());
                }
                if let Some(element) = &element {
                    if target.is_some() {
                        let _ = ax::set_bool(element, "AXFocused", true);
                    }
                    let before = ax::string(element, "AXValue");
                    if ax::set_string(element, "AXSelectedText", text).is_ok() && ax::string(element, "AXValue") != before {
                        return Ok(("ax", None));
                    }
                }
                let token = self.token.clone();
                input::text_to_pid(self.pid, text, &move || token.is_stopped())?;
                Ok(("pid", Some("typed as key events; verify the result".into())))
            }
            Action::Keypress { chord, .. } => {
                if ax::focused_is_secure(&self.app_element) {
                    return Err("A secure text field has focus; computer use won't press keys into it.".into());
                }
                input::key_to_pid(self.pid, chord)?;
                Ok(("pid", Some("some apps ignore keys while in the background; verify, or use menu".into())))
            }
            Action::Scroll { target, dx, dy } => {
                if let Target::Ref(reference) = target {
                    let element = self.element(reference)?;
                    if ax::scroll(&element, *dx, *dy)? {
                        return Ok(("ax", None));
                    }
                }
                let point = self.point(target)?;
                let (dx, dy) = (*dx, *dy);
                self.foreground()?.scroll(point, dx, dy)?;
                Ok(("foreground", None))
            }
            Action::Drag { from, to } => {
                let from = self.pixel(from.0, from.1)?;
                let to = self.pixel(to.0, to.1)?;
                let token = self.token.clone();
                self.foreground()?.drag(from, to, &move || token.is_stopped())?;
                Ok(("foreground", None))
            }
            Action::Menu { path } => {
                ax::press_menu(&self.app_element, path)?;
                Ok(("ax", None))
            }
            Action::Raise => {
                ax::perform(&self.observation.window, "AXRaise").map_err(|error| ax::describe_error(error, "raising the window"))?;
                let _ = ax::set_bool(&self.observation.window, "AXMain", true);
                Ok(("ax", None))
            }
            Action::Wait { ms } => {
                let deadline = Instant::now() + Duration::from_millis(*ms);
                while Instant::now() < deadline {
                    if self.stopped() {
                        return Err(STOPPED.into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Ok(("wait", None))
            }
        }
    }
}

fn press(element: &ax::Element) -> Result<(), String> {
    let mut last = None;
    for action in ["AXPress", "AXConfirm", "AXPick"] {
        match ax::perform(element, action) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    Err(ax::describe_error(last.expect("an action was tried"), "pressing it"))
}

fn check_expect(context: &ActContext<'_>, expect: &Expect) -> Value {
    let deadline = Instant::now() + Duration::from_millis(expect.timeout_ms);
    let criteria = ax::Criteria { role: expect.role.as_deref(), title_contains: expect.title_contains.as_deref(), value_equals: expect.value_equals.as_deref() };
    loop {
        let window_title = ax::string(&context.observation.window, "AXTitle").unwrap_or_default();
        let title_ok = expect
            .window_title_contains
            .as_deref()
            .is_none_or(|wanted| window_title.to_lowercase().contains(&wanted.to_lowercase()) || ax::windows(&context.app_element).iter().any(|window| window.title.to_lowercase().contains(&wanted.to_lowercase())));
        let found = if expect.role.is_some() || expect.title_contains.is_some() || expect.value_equals.is_some() {
            ax::find(&context.observation.window, &criteria)
        } else {
            Some(String::new())
        };
        let present = title_ok && found.is_some();
        let met = present == expect.exists;
        if met || Instant::now() >= deadline || context.stopped() {
            let observed = match (&found, title_ok) {
                (Some(description), true) if !description.is_empty() => format!("found {description}"),
                (Some(_), true) => "found".to_string(),
                (_, false) => format!("window title is \"{window_title}\""),
                (None, true) => "no matching element".to_string(),
            };
            return json!({ "met": met, "observed": observed });
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[allow(clippy::too_many_arguments)]
async fn act(
    manager: &ComputerUseManager,
    task_id: &str,
    target: AppIdentity,
    state_id: String,
    actions: Vec<Action>,
    expect: Option<Expect>,
    never_allow: Vec<String>,
    token: StopToken,
) -> Result<Value, String> {
    let task = task_id.to_string();
    let own_pid = manager.own.pid;
    manager
        .engine()
        .run(move |state: &mut EngineState| -> Result<Value, String> {
            let pid = target.pid.ok_or("The app has no process.")?;
            let observation = state.observation(&task, &state_id).ok_or_else(|| {
                format!("stateId {state_id} is out of date or unknown. Take a new computer_snapshot or computer_screenshot and use its stateId.")
            })?;
            if observation.pid != pid {
                return Err(format!("stateId {state_id} belongs to another app."));
            }
            let mut context = ActContext {
                pid,
                own_pid,
                observation,
                app_element: ax::application(pid),
                foreground: None,
                never_allow,
                token,
            };
            let mut results = Vec::new();
            let mut all_ok = true;
            for (index, action) in actions.iter().enumerate() {
                if context.stopped() {
                    all_ok = false;
                    results.push(json!({ "index": index, "kind": action.kind(), "ok": false, "error": STOPPED }));
                    break;
                }
                match context.perform(action) {
                    Ok((via, note)) => results.push(json!({ "index": index, "kind": action.kind(), "ok": true, "via": via, "note": note })),
                    Err(error) => {
                        all_ok = false;
                        results.push(json!({ "index": index, "kind": action.kind(), "ok": false, "error": error }));
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(60));
            }
            // Restores the front app and cursor before the postcondition is checked.
            context.foreground = None;
            let expect = match (&expect, all_ok) {
                (Some(expect), true) => check_expect(&context, expect),
                _ => Value::Null,
            };
            let focused = ax::element(&context.app_element, "AXFocusedUIElement").map(|element| ax::describe(&element));
            let window_title = ax::string(&context.observation.window, "AXTitle");
            Ok(json!({
                "stateId": state_id, "app": app_json(&target), "results": results, "expect": expect,
                "focused": focused, "windowTitle": window_title,
            }))
        })
        .await?
}

// ---------------------------------------------------------------------------------------------
// Sessions, the stop shortcut and the stop paths

pub fn stop_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), Code::Period)
}

/// The global-shortcut plugin, whose handler runs the emergency stop. Shortcuts are registered
/// from Rust only, so the renderer gets no global-shortcut capability (don't add one).
pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() == ShortcutState::Pressed && *shortcut == stop_shortcut() {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { emergency_stop(&app, "⌃⌥⌘.").await });
            }
        })
        .build()
}

fn state_event(app: &AppHandle, task_id: &str, active: bool, app_name: Option<&str>, hotkey: bool) {
    emit(app, task_id, json!({ "type": "computer_state", "taskId": task_id, "computer": { "active": active, "app": app_name, "hotkey": hotkey } }));
}

fn begin_session(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, app_name: &str) {
    let (first, changed) = match manager.lock() {
        Ok(mut inner) => {
            let first = inner.sessions.is_empty() && !inner.hotkey_registered;
            let previous = inner.sessions.insert(task_id.to_string(), app_name.to_string());
            (first, previous.as_deref() != Some(app_name))
        }
        Err(_) => return,
    };
    if first {
        let registered = app.global_shortcut().register(stop_shortcut()).is_ok();
        if let Ok(mut inner) = manager.lock() {
            inner.hotkey_registered = registered;
            inner.hotkey_available = registered;
        }
    }
    if changed {
        let hotkey = manager.lock().map(|inner| inner.hotkey_available).unwrap_or(false);
        state_event(app, task_id, true, Some(app_name), hotkey);
        let _ = crate::menu_bar::refresh(app);
    }
}

fn end_session(app: &AppHandle, task_id: &str) {
    let manager = app.state::<ComputerUseManager>();
    let (ended, unregister) = match manager.lock() {
        Ok(mut inner) => {
            let ended = inner.sessions.remove(task_id).is_some();
            let unregister = inner.sessions.is_empty() && inner.hotkey_registered;
            if unregister {
                inner.hotkey_registered = false;
            }
            (ended, unregister)
        }
        Err(_) => return,
    };
    if unregister {
        let _ = app.global_shortcut().unregister(stop_shortcut());
    }
    if ended {
        state_event(app, task_id, false, None, false);
        let _ = crate::menu_bar::refresh(app);
    }
}

/// The chat's run ended: its session ends with it.
pub fn on_run_state(app: &AppHandle, task_id: &str, state: &str) {
    if matches!(state, "idle" | "interrupted") {
        end_session(app, task_id);
    }
}

/// The chat's worker exited: nothing of its requests may continue. Grants stay (they belong to
/// the chat, not the worker).
pub fn on_worker_stopped(app: &AppHandle, task_id: &str) {
    let manager = app.state::<ComputerUseManager>();
    let replies = manager.lock().map(|mut inner| ComputerUseManager::stop_chat(&mut inner, task_id)).unwrap_or_default();
    cancel_replies(replies);
    end_session(app, task_id);
}

/// The chat was archived, deleted or moved to a worktree.
pub fn dispose(app: &AppHandle, task_id: &str) {
    on_worker_stopped(app, task_id);
    let manager = app.state::<ComputerUseManager>();
    if let Ok(mut inner) = manager.lock() {
        inner.grants.forget_chat(task_id);
    }
    if let Ok(mut operations) = manager.operations.lock() {
        operations.remove(task_id);
    }
    if let Some(engine) = manager.engine.get() {
        let task = task_id.to_string();
        engine.submit(move |state: &mut EngineState| {
            state.chats.remove(&task);
        });
    }
}

/// Stops every chat's computer use: the stop shortcut, the menu bar item, or switching the
/// feature off. Chats with a session also have their run stopped.
pub async fn emergency_stop(app: &AppHandle, source: &str) {
    let manager = app.state::<ComputerUseManager>();
    let (tasks, replies) = match manager.lock() {
        Ok(mut inner) => {
            let mut tasks = inner.sessions.keys().cloned().collect::<Vec<_>>();
            for (task, token) in inner.requests.values() {
                token.stop();
                if !tasks.contains(task) {
                    tasks.push(task.clone());
                }
            }
            let replies = inner.access.drain().map(|(_, pending)| pending.reply).collect::<Vec<_>>();
            (tasks, replies)
        }
        Err(_) => return,
    };
    cancel_replies(replies);
    for task in &tasks {
        let _ = crate::commands::stop_task(app.clone(), task.clone()).await;
        emit(app, task, json!({ "type": "extension_notice", "taskId": task, "message": format!("Computer use stopped ({source})."), "level": "info" }));
        end_session(app, task);
    }
}

/// Switching the feature off: stop everything and forget every grant.
async fn disable(app: &AppHandle) {
    emergency_stop(app, "switched off").await;
    let manager = app.state::<ComputerUseManager>();
    if let Ok(mut inner) = manager.lock() {
        inner.grants.clear();
    }
    if let Some(engine) = manager.engine.get() {
        engine.submit(|state: &mut EngineState| state.chats.clear());
    }
}

pub fn dispose_all(app: &AppHandle) {
    let manager = app.state::<ComputerUseManager>();
    let unregister = match manager.lock() {
        Ok(mut inner) => {
            for (_, token) in inner.requests.values() {
                token.stop();
            }
            inner.access.clear();
            inner.sessions.clear();
            std::mem::replace(&mut inner.hotkey_registered, false)
        }
        Err(_) => false,
    };
    if unregister {
        let _ = app.global_shortcut().unregister(stop_shortcut());
    }
}

// ---------------------------------------------------------------------------------------------
// Commands

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseStatus {
    supported: bool,
    accessibility: bool,
    screen_recording: bool,
    hotkey_available: bool,
    /// Dev builds are launched from a shell, so macOS credits permissions to whatever started them.
    dev_build: bool,
}

#[tauri::command]
pub async fn computer_use_status(app: AppHandle) -> ComputerUseStatus {
    let supported = permissions::is_supported();
    let accessibility = supported && permissions::accessibility();
    let screen_recording = supported && permissions::screen_recording_preflight() && capture::probe().await;
    let hotkey_available = app.state::<ComputerUseManager>().lock().map(|inner| inner.hotkey_available).unwrap_or(true);
    ComputerUseStatus { supported, accessibility, screen_recording, hotkey_available, dev_build: cfg!(debug_assertions) }
}

/// Shows macOS's own prompt (which also lists WackCode in the pane) and opens the pane.
#[tauri::command]
pub fn computer_use_request_permission(pane: permissions::Pane) -> Result<(), String> {
    if !permissions::is_supported() {
        return Err("Computer use needs macOS 14 or later.".into());
    }
    match pane {
        permissions::Pane::Accessibility => {
            permissions::request_accessibility();
        }
        permissions::Pane::ScreenRecording => {
            permissions::request_screen_recording();
        }
    }
    permissions::open_pane(pane)
}

#[tauri::command]
pub fn computer_use_open_settings(pane: permissions::Pane) -> Result<(), String> {
    permissions::open_pane(pane)
}

#[tauri::command]
pub fn computer_use_reset_permissions(app: AppHandle) -> Result<(), String> {
    permissions::reset(&app.config().identifier)
}

/// Screen Recording applies only to a fresh process.
#[tauri::command]
pub fn computer_use_relaunch(app: AppHandle) {
    crate::cleanup_before_exit(&app);
    app.restart();
}

#[tauri::command]
pub async fn set_computer_use_config(app: AppHandle, input: ComputerUseConfig) -> Result<ComputerUseConfig, String> {
    let config = ComputerUseConfig { enabled: input.enabled, never_allow: policy::validate_never_allow(&input.never_allow)? };
    let was_enabled = config_enabled(&app);
    app.state::<MetadataState>().mutate(|data| {
        data.computer_use = config.clone();
        Ok(())
    })?;
    if was_enabled && !config.enabled {
        disable(&app).await;
    }
    crate::worker::broadcast_computer_use(&app).await?;
    Ok(config)
}

fn config_enabled(app: &AppHandle) -> bool {
    config(app).enabled
}

/// The user answered an access card. Only the request id and decision cross from the renderer:
/// which app the answer covers comes from the card Rust raised. "Never allow" also adds the app
/// to the Settings list, and the new settings come back so Settings can show it.
#[tauri::command]
pub fn computer_use_respond_access(app: AppHandle, task_id: String, request_id: String, decision: AccessDecision) -> Result<Option<ComputerUseConfig>, String> {
    let manager = app.state::<ComputerUseManager>();
    let pending = {
        let mut inner = manager.lock()?;
        match inner.access.get(&request_id) {
            Some(pending) if pending.task_id == task_id => inner.access.remove(&request_id),
            _ => return Err("That access request is no longer open.".into()),
        }
    }
    .expect("checked above");
    let mut updated = None;
    if decision == AccessDecision::Never {
        if let Some(bundle_id) = pending.identity.bundle_id.clone() {
            let state = app.state::<MetadataState>();
            state.mutate(|data| {
                if !data.computer_use.never_allow.iter().any(|entry| entry.eq_ignore_ascii_case(&bundle_id)) {
                    data.computer_use.never_allow.push(bundle_id.clone());
                }
                Ok(())
            })?;
            updated = Some(config(&app));
        }
    }
    let _ = pending.reply.send(decision);
    Ok(updated)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningAppInfo {
    name: String,
    bundle_id: String,
}

/// Running apps with bundle ids, for the never-allow picker.
#[tauri::command]
pub async fn computer_use_list_apps(app: AppHandle) -> Result<Vec<RunningAppInfo>, String> {
    let manager = app.state::<ComputerUseManager>();
    let own = manager.own.pid;
    manager
        .engine()
        .run(move |_| {
            let mut list = apps::running()
                .into_iter()
                .filter(|running| running.pid != own)
                .filter_map(|running| Some(RunningAppInfo { bundle_id: running.bundle_id?, name: running.name }))
                .collect::<Vec<_>>();
            list.sort_by_key(|entry| entry.name.to_lowercase());
            list.dedup_by(|a, b| a.bundle_id.eq_ignore_ascii_case(&b.bundle_id));
            list
        })
        .await
}

pub fn supported() -> bool {
    permissions::is_supported()
}
