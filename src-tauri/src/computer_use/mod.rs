//! Computer use: the agent observes and operates native app windows, mainly to verify apps the
//! user is building. This module is the security boundary; the worker's tools
//! (`worker/src/builtin/computer-use/`) only describe requests. Every request re-checks, in
//! order: the setting and macOS support, the permission it needs, the target's block status
//! (`policy`), then this chat's denial/grant and exact-identity saved Always Allow entries —
//! raising the access card itself when there is none. Only a live card answer may create a
//! saved grant. Answers, stops and restrictive settings changes serialize on `inner`, always
//! before the metadata lock; a failed save leaves the card retryable and grants nothing.
//! The card is answered renderer → `computer_use_respond_access`, never through the worker.
//!
//! Background first: element actions go through Accessibility and keys go only to the target
//! process. Only pointer actions Accessibility can't do use the real cursor (`input::Foreground`).
//!
//! A chat has a computer-use *session* from its first request in a run until that run ends.
//! While any session exists the ⌃⌥⌘. stop shortcut is registered and the menu bar offers
//! "Stop Computer Use".

// The platform pieces (Accessibility, pid input, capture, app resolution, permissions and
// policy) live in the `wackdev` crate, shared with the dev-app driver. `pointer` keeps the
// real-cursor half of input; `engine`, `cursor` and `request` are app concerns.
pub use wackdev::{apps, ax, capture, geometry, input, keys, outline, permissions, policy};

mod cursor;
mod engine;
mod pointer;
mod request;

use crate::models::{ComputerAppGrant, ComputerUseConfig, TaskKind};
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
    Always,
    Deny,
    Never,
    Cancelled,
}

struct PendingAccess {
    task_id: String,
    identity: AppIdentity,
    token: StopToken,
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

impl Inner {
    /// Conservative restriction changes cancel every queued/in-flight computer request and
    /// access card, not the chat runs themselves. Already dispatched native calls can finish;
    /// every later step rechecks its stopped token before observing or acting again.
    fn cancel_all_requests(&mut self) {
        for (_, token) in self.requests.values() {
            token.stop();
        }
        cancel_replies(self.access.drain().map(|(_, pending)| { pending.token.stop(); pending.reply }).collect());
    }
}

pub struct ComputerUseManager {
    inner: Mutex<Inner>,
    operations: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    engine: OnceLock<Engine>,
    own: OwnIdentity,
    cursor: cursor::Cursor,
}

impl Default for ComputerUseManager {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner { hotkey_available: true, ..Inner::default() }),
            operations: Mutex::new(HashMap::new()),
            engine: OnceLock::new(),
            own: OwnIdentity::current(),
            cursor: cursor::Cursor::default(),
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
        self.cursor.wake();
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
        ids.into_iter().filter_map(|id| inner.access.remove(&id)).map(|pending| { pending.token.stop(); pending.reply }).collect()
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

fn saved_key(grant: &ComputerAppGrant) -> Option<AppKey> {
    AppKey::persistent(grant.bundle_id.as_deref(), &grant.path)
}

/// Derive persistence from host-resolved identity only. Recheck that the canonical path has
/// not disappeared or changed since the card was raised; never follow a new symlink for it.
fn persistable_grant(identity: &AppIdentity) -> Option<ComputerAppGrant> {
    let path = identity.path.as_deref()?;
    if path.canonicalize().ok()?.as_os_str() != path.as_os_str() {
        return None;
    }
    let path = path.to_str()?;
    AppKey::persistent(identity.bundle_id.as_deref(), path)?;
    Some(ComputerAppGrant { name: identity.name.clone(), bundle_id: identity.bundle_id.clone(), path: path.to_string() })
}

fn request_settings(metadata: &MetadataState, task_id: &str, token: &StopToken) -> Result<ComputerUseConfig, String> {
    if token.is_stopped() {
        return Err(STOPPED.into());
    }
    let data = metadata.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
    if data.tasks.iter().any(|task| task.id == task_id && task.kind == TaskKind::Chat) {
        return Err("Computer use isn't available in Chat mode.".into());
    }
    if !data.computer_use.enabled {
        return Err("Computer use is switched off in Settings › Packages.".into());
    }
    Ok(data.computer_use.clone())
}

fn effective_access(settings: &ComputerUseConfig, inner: &Inner, own: &OwnIdentity, task_id: &str, identity: &AppIdentity) -> Result<Grant, String> {
    if !settings.enabled {
        return Err("Computer use is switched off in Settings › Packages.".into());
    }
    let saved = settings.always_allow.iter().filter_map(saved_key).collect::<Vec<_>>();
    inner.grants.effective_decision(task_id, identity, own, &settings.never_allow, &saved)
}

fn denied_access(identity: &AppIdentity) -> String {
    format!("The user denied access to {} for this chat. Don't try again; ask them what they'd like instead.", identity.name)
}

/// Rechecked inside engine jobs too: waiting on a card, another app's AX work, launch or
/// capture must not keep using a settings/grant snapshot that has since been revoked.
#[derive(Clone)]
struct AccessScope {
    app: AppHandle,
    task_id: String,
    identity: AppIdentity,
    token: StopToken,
}

impl AccessScope {
    fn new(app: &AppHandle, task_id: &str, identity: AppIdentity, token: &StopToken) -> Self {
        Self { app: app.clone(), task_id: task_id.to_string(), identity, token: token.clone() }
    }

    fn check(&self) -> Result<ComputerUseConfig, String> {
        let manager = self.app.state::<ComputerUseManager>();
        let inner = manager.lock()?;
        let settings = request_settings(&self.app.state::<MetadataState>(), &self.task_id, &self.token)?;
        match effective_access(&settings, &inner, &manager.own, &self.task_id, &self.identity)? {
            Grant::Allowed => Ok(settings),
            Grant::Denied => Err(denied_access(&self.identity)),
            Grant::Unknown => Err(format!("Access to {} is no longer allowed. Ask the user for access again.", self.identity.name)),
        }
    }
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
    // Capture ownership before waiting on the operation lock. A request queued in an old run
    // may still settle later, but cannot acquire the next run's visual generation.
    let token = StopToken::default();
    let cursor = manager.cursor.session(&task_id, token.clone());
    {
        let mut inner = manager.lock()?;
        if inner.cancelled.remove(&request_id) {
            return Err(STOPPED.into());
        }
        // Register before the operation wait, so stop/revoke also cancels queued work.
        inner.requests.insert(request_id.clone(), (task_id.clone(), token.clone()));
    }
    let _guard = operation.lock().await;
    let result = if token.is_stopped() {
        Err(STOPPED.into())
    } else {
        // Keep the future (and any card receiver) alive until timeout cancellation takes the
        // answer lock. Otherwise a timeout could drop the receiver during a grant's save.
        let mut timed = Box::pin(tokio::time::timeout(OPERATION_TIMEOUT, run(&app, &manager, &task_id, parsed, &token, cursor)));
        match timed.as_mut().await {
            Ok(result) => result,
            Err(_) => {
                let _inner = manager.lock()?;
                token.stop();
                Err("The computer-use action took too long and was stopped.".into())
            }
        }
    };
    let stale = if let Ok(mut inner) = manager.lock() {
        if result.is_err() { token.stop(); }
        inner.requests.remove(&request_id);
        // A timeout drops the card's receiver. Retire its event too, and never let a late
        // renderer answer save a grant for an abandoned request.
        let ids = inner.access.iter().filter(|(_, pending)| pending.reply.is_closed()).map(|(id, _)| id.clone()).collect::<Vec<_>>();
        ids.into_iter().filter_map(|id| inner.access.remove(&id).map(|pending| (id, pending.task_id))).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    manager.cursor.wake();
    for (id, task) in stale {
        emit(&app, &task, json!({ "type": "computer_access_resolved", "taskId": task, "requestId": id, "decision": AccessDecision::Cancelled }));
    }
    if token.is_stopped() && result.is_ok() {
        return Err(STOPPED.into());
    }
    result
}

async fn run(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, request: ComputerRequest, token: &StopToken, cursor: Option<cursor::Session>) -> Result<Value, String> {
    let settings = request_settings(&app.state::<MetadataState>(), task_id, token)?;
    if !permissions::is_supported() {
        return Err("Computer use needs macOS 14 or later.".into());
    }
    if !permissions::accessibility() {
        return Err("WackCode doesn't have Accessibility permission, which computer use needs. Ask the user to allow it in Settings › Computer use.".into());
    }
    manager.cursor.set_enabled(app, settings.show_agent_cursor);
    match request {
        ComputerRequest::Apps => list_apps(app, manager, task_id, token).await,
        ComputerRequest::Open { app: query } => open(app, manager, task_id, &query, token, cursor).await,
        ComputerRequest::Snapshot { app: query, window } => {
            let target = running_target(app, manager, task_id, &query, token).await?;
            if let (Some(cursor), Some(pid)) = (&cursor, target.pid) { cursor.target_app(pid); }
            begin_session(app, manager, task_id, &target.name, token);
            snapshot(manager, AccessScope::new(app, task_id, target, token), window).await
        }
        ComputerRequest::Screenshot { app: query, window } => {
            if !permissions::screen_recording_preflight() {
                return Err("WackCode doesn't have Screen Recording permission, so it can't capture windows. Use computer_snapshot, or ask the user to allow it in Settings › Computer use.".into());
            }
            let target = running_target(app, manager, task_id, &query, token).await?;
            if let (Some(cursor), Some(pid)) = (&cursor, target.pid) { cursor.target_app(pid); }
            begin_session(app, manager, task_id, &target.name, token);
            screenshot(manager, AccessScope::new(app, task_id, target, token), window).await
        }
        ComputerRequest::Act { app: query, state_id, actions, expect } => {
            let target = running_target(app, manager, task_id, &query, token).await?;
            if let (Some(cursor), Some(pid)) = (&cursor, target.pid) { cursor.target_app(pid); }
            begin_session(app, manager, task_id, &target.name, token);
            act(manager, AccessScope::new(app, task_id, target, token), state_id, actions, expect, cursor).await
        }
    }
}

/// Resolves `query`, refuses blocked apps, and makes sure this chat may use it.
async fn resolve_target(
    app: &AppHandle,
    manager: &ComputerUseManager,
    task_id: &str,
    query: &str,
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
    {
        let inner = manager.lock()?;
        let settings = request_settings(&app.state::<MetadataState>(), task_id, token)?;
        effective_access(&settings, &inner, &manager.own, task_id, &identity)?;
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
    token: &StopToken,
) -> Result<AppIdentity, String> {
    match resolve_target(app, manager, task_id, query, token, false).await? {
        apps::Resolved::Running(running) => Ok(running.identity()),
        apps::Resolved::Installed(installed) => Err(format!("{} isn't running. Open it with computer_open first.", installed.name)),
    }
}

async fn ensure_access(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, identity: &AppIdentity, launch: bool, token: &StopToken) -> Result<(), String> {
    {
        let inner = manager.lock()?;
        let settings = request_settings(&app.state::<MetadataState>(), task_id, token)?;
        match effective_access(&settings, &inner, &manager.own, task_id, identity)? {
            Grant::Allowed => return Ok(()),
            Grant::Denied => return Err(denied_access(identity)),
            Grant::Unknown => {}
        }
    }
    let icon = match identity.path.clone() {
        Some(path) => manager.engine().run(move |_| apps::icon_data_url(&path)).await.ok().flatten(),
        None => None,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let (reply, mut receiver) = oneshot::channel();
    {
        let mut inner = manager.lock()?;
        let settings = request_settings(&app.state::<MetadataState>(), task_id, token)?;
        match effective_access(&settings, &inner, &manager.own, task_id, identity)? {
            Grant::Allowed => return Ok(()),
            Grant::Denied => return Err(denied_access(identity)),
            Grant::Unknown => {}
        }
        inner.access.insert(request_id.clone(), PendingAccess { task_id: task_id.to_string(), identity: identity.clone(), token: token.clone(), reply });
    }
    emit(app, task_id, json!({
        "type": "computer_access_request", "taskId": task_id, "requestId": request_id, "launch": launch,
        "canAlwaysAllow": persistable_grant(identity).is_some(),
        "app": { "name": identity.name, "bundleId": identity.bundle_id, "path": identity.path, "icon": icon },
    }));
    // Wait for an answer, or for the owning request to be stopped/cancelled/timed out.
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
    if decision == AccessDecision::Cancelled {
        return Err(STOPPED.into());
    }
    // Answers apply grants while holding `inner`, not here after the await: revoking or
    // stopping between the answer and this continuation must never recreate an allow.
    AccessScope::new(app, task_id, identity.clone(), token).check().map(|_| ())
}

async fn list_apps(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, token: &StopToken) -> Result<Value, String> {
    let app = app.clone();
    let task = task_id.to_string();
    let token = token.clone();
    let apps = manager
        .engine()
        .run(move |_| -> Result<Vec<Value>, String> {
            let manager = app.state::<ComputerUseManager>();
            request_settings(&app.state::<MetadataState>(), &task, &token)?;
            let mut list = Vec::new();
            for running in apps::running().into_iter().filter(|running| running.pid != manager.own.pid) {
                let identity = running.identity();
                let access = {
                    let inner = manager.lock()?;
                    let settings = request_settings(&app.state::<MetadataState>(), &task, &token)?;
                    match effective_access(&settings, &inner, &manager.own, &task, &identity) {
                        Err(_) => "blocked",
                        Ok(Grant::Allowed) => "granted",
                        Ok(Grant::Denied) => "denied",
                        Ok(Grant::Unknown) => "ask",
                    }
                };
                // Window titles can be private (mail subjects, document names): use the same
                // effective decision as actions, including saved grants and chat denials.
                let windows = if access == "granted" {
                    if token.is_stopped() { return Err(STOPPED.into()); }
                    ax::windows(&ax::application(running.pid))
                        .iter()
                        .map(|window| window_json(window, input::window_number(&window.element, running.pid)))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                list.push((access == "granted", json!({
                    "name": running.name, "bundleId": running.bundle_id, "pid": running.pid,
                    "frontmost": running.frontmost, "access": access, "windows": windows,
                })));
            }
            list.sort_by_key(|(granted, _)| !granted);
            Ok(list.into_iter().map(|(_, value)| value).collect())
        })
        .await??;
    Ok(json!({ "apps": apps }))
}

async fn open(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, query: &str, token: &StopToken, cursor: Option<cursor::Session>) -> Result<Value, String> {
    let resolved = resolve_target(app, manager, task_id, query, token, true).await?;
    let (identity, launched) = match resolved {
        apps::Resolved::Running(running) => (running.identity(), false),
        apps::Resolved::Installed(installed) => {
            let mut identity = installed.identity();
            // Launch the canonical path covered by access, not a potentially retargeted alias.
            let path = identity.path.clone().ok_or("The app's canonical path is no longer available.")?;
            let scope = AccessScope::new(app, task_id, identity.clone(), token);
            let pid = manager.engine().run(move |_| { scope.check()?; apps::launch(&path) }).await??;
            identity.pid = Some(pid);
            (identity, true)
        }
    };
    let scope = AccessScope::new(app, task_id, identity.clone(), token);
    scope.check()?;
    begin_session(app, manager, task_id, &identity.name, token);
    let pid = identity.pid.ok_or("The app has no process.")?;
    if let Some(cursor) = &cursor { cursor.target_app(pid); }
    let deadline = Instant::now() + if launched { LAUNCH_WINDOW_WAIT } else { Duration::ZERO };
    let windows = loop {
        let scope = scope.clone();
        let windows = manager
            .engine()
            .run(move |_| -> Result<Vec<Value>, String> {
                scope.check()?;
                let app_element = ax::application(pid);
                ax::enable_manual_accessibility(&app_element);
                Ok(ax::windows(&app_element).iter().map(|window| window_json(window, input::window_number(&window.element, pid))).collect())
            })
            .await??;
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

async fn snapshot(manager: &ComputerUseManager, scope: AccessScope, window: Option<u32>) -> Result<Value, String> {
    let task = scope.task_id.clone();
    let target = scope.identity.clone();
    manager
        .engine()
        .run(move |state: &mut EngineState| {
            scope.check()?;
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

async fn screenshot(manager: &ComputerUseManager, scope: AccessScope, window: Option<u32>) -> Result<Value, String> {
    let target = scope.identity.clone();
    let pid = target.pid.ok_or("The app has no process.")?;
    let name = target.name.clone();
    let before_capture = scope.clone();
    let (window_value, number) = manager
        .engine()
        .run(move |_| -> Result<(Value, u32), String> {
            before_capture.check()?;
            let (window, number, _) = pick_window(pid, &name, window)?;
            let number = number.ok_or("That window can't be identified for capture.")?;
            Ok((window_json(&window, Some(number)), number))
        })
        .await??;
    scope.check()?;
    let capture = capture::window(number, None).await?;
    scope.check()?;
    let task = scope.task_id.clone();
    let geometry = CaptureGeometry { frame: capture.frame, output: (capture.width, capture.height) };
    let state_id = manager
        .engine()
        .run(move |state: &mut EngineState| -> Result<String, String> {
            scope.check()?;
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
    own_bundle_id: Option<String>,
    pid: i32,
    own_pid: i32,
    observation: &'a Observation,
    app_element: ax::Element,
    foreground: Option<pointer::Foreground>,
    never_allow: Vec<String>,
    token: StopToken,
    cursor: Option<cursor::Session>,
    feedback: Option<cursor::Feedback>,
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

    fn foreground(&mut self) -> Result<&mut pointer::Foreground, String> {
        if self.foreground.is_none() {
            let own = OwnIdentity { pid: self.own_pid, bundle_id: self.own_bundle_id.clone(), ..OwnIdentity::default() };
            let never_allow = self.never_allow.clone();
            let blocked = Box::new(move |bundle_id: Option<&str>| {
                let identity = AppIdentity { pid: None, name: String::new(), bundle_id: bundle_id.map(str::to_string), path: None };
                policy::block_reason(&identity, &own, &never_allow).is_some()
            });
            let token = self.token.clone();
            self.foreground = Some(pointer::Foreground::begin(self.pid, self.own_pid, self.observation.window.clone(), blocked, &move || token.is_stopped())?);
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

    /// Visual geometry is best-effort and cannot make an otherwise valid AX action fail.
    fn visual_point(&self, window: &ax::Element, point: (f64, f64)) -> Option<cursor::Target> {
        let frame = ax::frame(window)?;
        if !point.0.is_finite() || !point.1.is_finite() || !frame.contains(point.0, point.1) { return None; }
        Some(cursor::Target { pid: self.pid, window: input::window_number(window, self.pid)?, frame, point })
    }

    fn show_element(&self, element: &ax::Element) {
        if let Some(feedback) = &self.feedback {
            let target = ax::frame(element).filter(|frame| frame.width > 0.0 && frame.height > 0.0).and_then(|frame| {
                let window = ax::element(element, "AXWindow").unwrap_or_else(|| self.observation.window.clone());
                self.visual_point(&window, frame.center())
            });
            feedback.at(target);
        }
    }

    fn pointer(&mut self, point: (f64, f64)) -> Result<&mut pointer::Foreground, String> {
        let target = self.visual_point(&self.observation.window, point);
        let feedback = self.feedback.clone().zip(target);
        let foreground = self.foreground()?;
        foreground.set_feedback(feedback);
        Ok(foreground)
    }

    /// Runs one action; `Ok((via, note))`.
    fn perform(&mut self, action: &Action) -> Result<(&'static str, Option<String>), String> {
        match action {
            Action::Press { target } => {
                let element = self.element(target)?;
                self.show_element(&element);
                press(&element)?;
                Ok(("ax", None))
            }
            Action::Click { target, button, count } => {
                if let (Target::Ref(reference), 1) = (target, *count) {
                    let element = self.element(reference)?;
                    self.show_element(&element);
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
                self.pointer(point)?.click(point, button, count)?;
                Ok(("foreground", None))
            }
            Action::SetText { target, text } => {
                let element = self.element(target)?;
                if ax::is_secure(&element) {
                    return Err("That's a secure text field; computer use never types into one.".into());
                }
                self.show_element(&element);
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
                if let Some(element) = &element { self.show_element(element); }
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
                if let Some(element) = ax::element(&self.app_element, "AXFocusedUIElement") { self.show_element(&element); }
                input::key_to_pid(self.pid, chord)?;
                Ok(("pid", Some("some apps ignore keys while in the background; verify, or use menu".into())))
            }
            Action::Scroll { target, dx, dy } => {
                if let Target::Ref(reference) = target {
                    let element = self.element(reference)?;
                    self.show_element(&element);
                    if ax::scroll(&element, *dx, *dy)? {
                        return Ok(("ax", None));
                    }
                }
                let point = self.point(target)?;
                let (dx, dy) = (*dx, *dy);
                self.pointer(point)?.scroll(point, dx, dy)?;
                Ok(("foreground", None))
            }
            Action::Drag { from, to } => {
                let from = self.pixel(from.0, from.1)?;
                let to = self.pixel(to.0, to.1)?;
                let token = self.token.clone();
                self.pointer(from)?.drag(from, to, &move || token.is_stopped())?;
                Ok(("foreground", None))
            }
            Action::Menu { path } => {
                ax::press_menu(&self.app_element, path, &|element| self.show_element(element))?;
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

async fn act(
    manager: &ComputerUseManager,
    scope: AccessScope,
    state_id: String,
    actions: Vec<Action>,
    expect: Option<Expect>,
    cursor: Option<cursor::Session>,
) -> Result<Value, String> {
    let task = scope.task_id.clone();
    let target = scope.identity.clone();
    let token = scope.token.clone();
    let own_pid = manager.own.pid;
    let own_bundle_id = manager.own.bundle_id.clone();
    manager
        .engine()
        .run(move |state: &mut EngineState| -> Result<Value, String> {
            let settings = scope.check()?;
            let pid = target.pid.ok_or("The app has no process.")?;
            let observation = state.observation(&task, &state_id).ok_or_else(|| {
                format!("stateId {state_id} is out of date or unknown. Take a new computer_snapshot or computer_screenshot and use its stateId.")
            })?;
            if observation.pid != pid {
                return Err(format!("stateId {state_id} belongs to another app."));
            }
            let mut context = ActContext {
                own_bundle_id,
                pid,
                own_pid,
                observation,
                app_element: ax::application(pid),
                foreground: None,
                never_allow: settings.never_allow,
                token,
                cursor,
                feedback: None,
            };
            let mut results = Vec::new();
            let mut all_ok = true;
            for (index, action) in actions.iter().enumerate() {
                context.never_allow = scope.check()?.never_allow;
                if context.stopped() {
                    all_ok = false;
                    results.push(json!({ "index": index, "kind": action.kind(), "ok": false, "error": STOPPED }));
                    break;
                }
                let kind = match action {
                    Action::Click { .. } => cursor::Kind::Clicking,
                    Action::Drag { .. } => cursor::Kind::Dragging,
                    Action::Scroll { .. } => cursor::Kind::Scrolling,
                    Action::SetText { .. } | Action::TypeText { .. } => cursor::Kind::Typing,
                    Action::Menu { .. } => cursor::Kind::Menu,
                    _ => cursor::Kind::Pressing,
                };
                context.feedback = context.cursor.as_ref().map(|cursor| cursor.begin(kind));
                let result = context.perform(action);
                if let Some(feedback) = &context.feedback { feedback.finish(result.is_ok()); }
                match result {
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
            scope.check()?;
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

fn begin_session(app: &AppHandle, manager: &ComputerUseManager, task_id: &str, app_name: &str, token: &StopToken) {
    let (first, changed) = match manager.lock() {
        Ok(mut inner) => {
            if token.is_stopped() { return; }
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
    manager.cursor.end_run(task_id);
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
    let manager = app.state::<ComputerUseManager>();
    if state == "running" { manager.cursor.start_run(app, task_id); }
    if matches!(state, "stopping" | "idle" | "interrupted") {
        stop_cursor(app, task_id);
    }
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

/// Retire visuals and access cards immediately when Stop is clicked, before an abort reaches
/// the worker. This shares the answer lock, so a late Always answer cannot survive a stop.
pub fn stop_cursor(app: &AppHandle, task_id: &str) {
    let manager = app.state::<ComputerUseManager>();
    let replies = manager.lock().map(|mut inner| ComputerUseManager::stop_chat(&mut inner, task_id)).unwrap_or_default();
    cancel_replies(replies);
    manager.cursor.end_run(task_id);
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
    for task in &tasks { manager.cursor.end_run(task); }
    cancel_replies(replies);
    for task in &tasks {
        let _ = crate::commands::stop_task(app.clone(), task.clone()).await;
        emit(app, task, json!({ "type": "extension_notice", "taskId": task, "message": format!("Computer use stopped ({source})."), "level": "info" }));
        end_session(app, task);
    }
}

/// Switching the feature off: stop everything and forget transient grants only. Saved Always
/// Allow entries remain on disk, but cannot authorize anything until the feature is enabled.
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
    manager.cursor.shutdown();
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
    /// Development builds may need their ad-hoc approvals renewed after recompilation.
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

/// Renews a denied permission for this build, shows macOS's prompt and opens the pane.
#[tauri::command]
pub async fn computer_use_request_permission(app: AppHandle, pane: permissions::Pane) -> Result<(), String> {
    if !permissions::is_supported() {
        return Err("Computer use needs macOS 14 or later.".into());
    }
    let granted = match pane {
        permissions::Pane::Accessibility => permissions::accessibility(),
        permissions::Pane::ScreenRecording => permissions::screen_recording_preflight() && capture::probe().await,
    };
    // The effective ScreenCaptureKit check is asynchronous, but the macOS permission UI must
    // still be requested on the app's main thread, as it was with the synchronous command.
    let identifier = app.config().identifier.clone();
    let (reply, result) = oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = reply.send(permissions::request(pane, &identifier, granted));
    }).map_err(|_| "The macOS permission request couldn't be opened.".to_string())?;
    result.await.map_err(|_| "The macOS permission request was cancelled.".to_string())?
}

#[tauri::command]
pub fn computer_use_open_settings(pane: permissions::Pane) -> Result<(), String> {
    permissions::open_pane(pane)
}

#[tauri::command]
pub fn computer_use_reset_permissions(app: AppHandle) -> Result<(), String> {
    permissions::reset(&app.config().identifier)
}

/// Rechecks macOS approvals in a fresh process when their cached state hasn't caught up.
#[tauri::command]
pub fn computer_use_relaunch(app: AppHandle) {
    crate::cleanup_before_exit(&app);
    app.restart();
}

/// Settings deliberately has no grant field. A larger renderer config can deserialize here,
/// but its `alwaysAllow` is ignored: only `answer_access` can create a saved app identity.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ComputerUseSettings {
    enabled: bool,
    show_agent_cursor: bool,
    never_allow: Vec<String>,
}

impl Default for ComputerUseSettings {
    fn default() -> Self {
        let defaults = ComputerUseConfig::default();
        Self { enabled: defaults.enabled, show_agent_cursor: defaults.show_agent_cursor, never_allow: defaults.never_allow }
    }
}

/// Called with `inner` held, before taking the metadata lock (the same order as card answers).
fn save_settings(metadata: &MetadataState, inner: &mut Inner, input: ComputerUseSettings) -> Result<(ComputerUseConfig, bool), String> {
    let never_allow = policy::validate_never_allow(&input.never_allow)?;
    let (settings, disabled, restricted) = metadata.mutate(|data| {
        let previous = &data.computer_use;
        let disabled = previous.enabled && !input.enabled;
        let restricted = !input.enabled || never_allow.iter().any(|id| !previous.never_allow.iter().any(|old| old.eq_ignore_ascii_case(id)));
        data.computer_use.enabled = input.enabled;
        data.computer_use.show_agent_cursor = input.show_agent_cursor;
        data.computer_use.never_allow = never_allow.clone();
        data.computer_use.always_allow.retain(|grant| !grant.bundle_id.as_deref().is_some_and(|id| never_allow.iter().any(|never| never.eq_ignore_ascii_case(id))));
        Ok((data.computer_use.clone(), disabled, restricted))
    })?;
    for id in &settings.never_allow {
        inner.grants.revoke_bundle_id(id);
    }
    if restricted { inner.cancel_all_requests(); }
    Ok((settings, disabled))
}

#[tauri::command]
pub async fn set_computer_use_config(app: AppHandle, input: ComputerUseSettings) -> Result<ComputerUseConfig, String> {
    let manager = app.state::<ComputerUseManager>();
    let (settings, disabled) = {
        let mut inner = manager.lock()?;
        save_settings(&app.state::<MetadataState>(), &mut inner, input)?
    };
    manager.cursor.set_enabled(&app, settings.enabled && settings.show_agent_cursor);
    if disabled {
        disable(&app).await;
    }
    crate::worker::broadcast_computer_use(&app).await?;
    Ok(config(&app))
}

fn revoke_app(metadata: &MetadataState, inner: &mut Inner, key: &AppKey) -> Result<ComputerUseConfig, String> {
    let settings = metadata.mutate(|data| {
        data.computer_use.always_allow.retain(|grant| saved_key(grant).as_ref() != Some(key));
        Ok(data.computer_use.clone())
    })?;
    inner.grants.revoke_app(key);
    // Include all queued requests and cards, so no pre-revocation answer or engine wait can
    // restore access. This intentionally does not abort the chats' non-computer work.
    inner.cancel_all_requests();
    Ok(settings)
}

#[tauri::command]
pub async fn computer_use_revoke_app(app: AppHandle, bundle_id: Option<String>, path: String) -> Result<ComputerUseConfig, String> {
    // Match the saved key literally (apart from bundle-id case), even if the app was removed
    // or its old path is now a symlink. Never canonicalize renderer input into another grant.
    let key = AppKey::persistent(bundle_id.as_deref(), &path).ok_or("An app grant needs an absolute path.")?;
    let manager = app.state::<ComputerUseManager>();
    let settings = {
        let mut inner = manager.lock()?;
        revoke_app(&app.state::<MetadataState>(), &mut inner, &key)?
    };
    manager.cursor.wake();
    Ok(settings)
}

#[tauri::command]
pub fn computer_use_cursor_appearance(app: AppHandle, input: cursor::CursorAppearance) -> Result<(), String> {
    input.validate()?;
    app.state::<ComputerUseManager>().cursor.appearance(&app, input);
    Ok(())
}

/// The only grant-creation path. Keep the pending card until persistence succeeds; holding
/// `inner` across the save serializes it with stop/revoke/settings, and rollback on any save
/// error leaves both the receiver and effective permissions untouched so the user can retry.
fn answer_access(metadata: &MetadataState, inner: &mut Inner, own: &OwnIdentity, task_id: &str, request_id: &str, decision: AccessDecision) -> Result<Option<ComputerUseConfig>, String> {
    let pending = match inner.access.get(request_id) {
        Some(pending) if pending.task_id == task_id => pending,
        _ => return Err("That access request is no longer open.".into()),
    };
    if pending.token.is_stopped() || pending.reply.is_closed() {
        let pending = inner.access.remove(request_id).expect("checked above");
        let _ = pending.reply.send(AccessDecision::Cancelled);
        return Err("That access request is no longer open.".into());
    }
    let identity = pending.identity.clone();
    if decision != AccessDecision::Cancelled {
        let settings = request_settings(metadata, task_id, &pending.token)?;
        if effective_access(&settings, inner, own, task_id, &identity)? == Grant::Denied {
            return Err(denied_access(&identity));
        }
    }
    let updated = match decision {
        AccessDecision::Always => {
            let grant = persistable_grant(&identity).ok_or("This app doesn't have a canonical path to save. Allow it for this chat instead.")?;
            let key = saved_key(&grant).expect("host-derived canonical grant");
            Some(metadata.mutate(|data| {
                data.computer_use.always_allow.retain(|saved| saved_key(saved).as_ref() != Some(&key));
                data.computer_use.always_allow.push(grant);
                Ok(data.computer_use.clone())
            })?)
        }
        AccessDecision::Never => match identity.bundle_id.as_deref() {
            Some(bundle_id) => Some(metadata.mutate(|data| {
                let mut never = data.computer_use.never_allow.clone();
                never.push(bundle_id.to_string());
                data.computer_use.never_allow = policy::validate_never_allow(&never)?;
                data.computer_use.always_allow.retain(|saved| !saved.bundle_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(bundle_id)));
                Ok(data.computer_use.clone())
            })?),
            None => None,
        },
        _ => None,
    };
    let pending = inner.access.remove(request_id).expect("the answer lock still owns the card");
    let key = AppKey::of(&identity);
    match decision {
        AccessDecision::Allow => inner.grants.allow(task_id, key),
        // Always is saved only: a parallel transient allow would survive revoking the entry.
        AccessDecision::Always | AccessDecision::Cancelled => {}
        AccessDecision::Deny | AccessDecision::Never => inner.grants.deny(task_id, key),
    }
    if decision == AccessDecision::Never {
        if let Some(id) = identity.bundle_id.as_deref() {
            inner.grants.revoke_bundle_id(id);
            inner.cancel_all_requests();
        }
    }
    let _ = pending.reply.send(decision);
    Ok(updated)
}

/// Only task/request ids and the decision come from the renderer; the app identity belongs to
/// the live card Rust raised. Always/Never return the saved settings for Settings to display.
#[tauri::command]
pub async fn computer_use_respond_access(app: AppHandle, task_id: String, request_id: String, decision: AccessDecision) -> Result<Option<ComputerUseConfig>, String> {
    let manager = app.state::<ComputerUseManager>();
    let (result, cancelled) = {
        let mut inner = manager.lock()?;
        let result = answer_access(&app.state::<MetadataState>(), &mut inner, &manager.own, &task_id, &request_id, decision);
        // An expired answer can remove its card before the timeout sweep sees it. Retire the
        // renderer card here too; a failed save keeps its pending entry and remains retryable.
        let cancelled = result.is_err() && !inner.access.contains_key(&request_id);
        (result, cancelled)
    };
    manager.cursor.wake();
    if cancelled {
        emit(&app, &task_id, json!({ "type": "computer_access_resolved", "taskId": task_id, "requestId": request_id, "decision": AccessDecision::Cancelled }));
    }
    result
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

#[cfg(test)]
mod access_tests {
    use super::*;
    use crate::models::{AppData, TaskRecord};
    use crate::secrets::SecretStore;
    use std::{fs, path::PathBuf};

    fn own() -> OwnIdentity {
        OwnIdentity { pid: 100, bundle_id: Some("com.wackcode.desktop".into()), ..OwnIdentity::default() }
    }

    fn fixture() -> (tempfile::TempDir, MetadataState, AppIdentity) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Example.app");
        fs::create_dir(&path).unwrap();
        let state = MetadataState {
            data: Mutex::new(AppData { computer_use: ComputerUseConfig { enabled: true, ..ComputerUseConfig::default() }, ..AppData::default() }),
            data_path: directory.path().join("wackcode.json"),
            secrets: SecretStore::load(directory.path()).unwrap(),
        };
        state.save().unwrap();
        let identity = AppIdentity { pid: Some(5), name: "Host-resolved app".into(), bundle_id: Some("com.example.App".into()), path: Some(path.canonicalize().unwrap()) };
        (directory, state, identity)
    }

    fn settings(state: &MetadataState) -> ComputerUseConfig {
        state.data.lock().unwrap().computer_use.clone()
    }

    fn disk_settings(state: &MetadataState) -> ComputerUseConfig {
        serde_json::from_str::<AppData>(&fs::read_to_string(&state.data_path).unwrap()).unwrap().computer_use
    }

    fn pending(inner: &mut Inner, task: &str, request: &str, identity: &AppIdentity) -> (StopToken, oneshot::Receiver<AccessDecision>) {
        let token = StopToken::default();
        let (reply, receiver) = oneshot::channel();
        inner.requests.insert(format!("operation-{request}"), (task.to_string(), token.clone()));
        inner.access.insert(request.to_string(), PendingAccess { task_id: task.to_string(), identity: identity.clone(), token: token.clone(), reply });
        (token, receiver)
    }

    fn save_always(state: &MetadataState, inner: &mut Inner, identity: &AppIdentity, request: &str) {
        let (_, mut receiver) = pending(inner, "chat", request, identity);
        answer_access(state, inner, &own(), "chat", request, AccessDecision::Always).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Always);
    }

    #[test]
    fn always_saves_host_identity_without_a_transient_allow_and_reloads_across_chats() {
        let (_directory, state, identity) = fixture();
        let mut inner = Inner::default();
        assert_eq!(serde_json::to_value(AccessDecision::Always).unwrap(), "always");
        save_always(&state, &mut inner, &identity, "card");
        let saved = settings(&state);
        assert_eq!(saved.always_allow, vec![persistable_grant(&identity).unwrap()]);
        assert_eq!(disk_settings(&state), saved);
        assert_eq!(inner.grants.decision("chat", &AppKey::of(&identity)), Grant::Unknown);
        let fresh = Inner::default();
        for task in ["chat", "other-chat"] {
            assert_eq!(effective_access(&disk_settings(&state), &fresh, &own(), task, &identity), Ok(Grant::Allowed));
        }
        let renamed = AppIdentity { name: "Different display name".into(), bundle_id: Some("COM.EXAMPLE.APP".into()), ..identity.clone() };
        assert_eq!(effective_access(&saved, &inner, &own(), "chat", &renamed), Ok(Grant::Allowed));
        inner.grants.deny("chat", AppKey::of(&identity));
        assert_eq!(effective_access(&saved, &inner, &own(), "chat", &identity), Ok(Grant::Denied));
        assert_eq!(effective_access(&saved, &inner, &own(), "other-chat", &identity), Ok(Grant::Allowed));
    }

    #[test]
    fn only_live_cards_for_the_same_chat_can_create_grants() {
        let (_directory, state, identity) = fixture();
        let mut inner = Inner::default();
        assert!(answer_access(&state, &mut inner, &own(), "chat", "invented", AccessDecision::Always).is_err());
        let (_, mut receiver) = pending(&mut inner, "chat", "card", &identity);
        assert!(answer_access(&state, &mut inner, &own(), "other-chat", "card", AccessDecision::Always).is_err());
        assert!(inner.access.contains_key("card"));
        assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        drop(receiver); // The owning request was timed out/dropped, even without a worker cancel.
        assert!(answer_access(&state, &mut inner, &own(), "chat", "card", AccessDecision::Always).is_err());
        assert!(!inner.access.contains_key("card"));
        assert!(settings(&state).always_allow.is_empty());
        assert!(disk_settings(&state).always_allow.is_empty());
    }

    #[test]
    fn persistence_requires_a_canonical_absolute_path_and_supports_path_only_apps() {
        let (directory, state, identity) = fixture();
        let alias = directory.path().join("Alias.app");
        std::os::unix::fs::symlink(identity.path.as_ref().unwrap(), &alias).unwrap();
        for path in [None, Some(PathBuf::from("relative.app")), Some(alias), Some(directory.path().join("Gone.app"))] {
            let target = AppIdentity { path, ..identity.clone() };
            assert!(persistable_grant(&target).is_none());
            let mut inner = Inner::default();
            let (_, mut receiver) = pending(&mut inner, "chat", "card", &target);
            assert!(answer_access(&state, &mut inner, &own(), "chat", "card", AccessDecision::Always).is_err());
            assert!(inner.access.contains_key("card"));
            assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert!(settings(&state).always_allow.is_empty());
        }
        let mut inner = Inner::default();
        let path_only = AppIdentity { bundle_id: None, ..identity };
        save_always(&state, &mut inner, &path_only, "path-only");
        assert_eq!(effective_access(&settings(&state), &inner, &own(), "other", &path_only), Ok(Grant::Allowed));
        let with_id = AppIdentity { bundle_id: Some("com.example.App".into()), ..path_only.clone() };
        assert_eq!(effective_access(&settings(&state), &inner, &own(), "other", &with_id), Ok(Grant::Unknown));
    }

    #[test]
    fn running_and_installed_identities_canonicalize_the_same_symlink() {
        let (directory, _state, identity) = fixture();
        let alias = directory.path().join("Alias.app");
        std::os::unix::fs::symlink(identity.path.as_ref().unwrap(), &alias).unwrap();
        let installed = apps::InstalledApp { path: alias.clone(), name: identity.name.clone(), bundle_id: identity.bundle_id.clone() };
        let running = apps::RunningApp { pid: 5, name: identity.name, bundle_id: identity.bundle_id, bundle_path: Some(alias), executable: None, frontmost: false };
        assert_eq!(AppKey::of(&installed.identity()), AppKey::of(&running.identity()));
        assert_eq!(installed.identity().path, identity.path);
        assert!(persistable_grant(&installed.identity()).is_some());
    }

    #[test]
    fn failed_always_and_never_saves_keep_cards_retryable_and_grant_nothing() {
        for decision in [AccessDecision::Always, AccessDecision::Never] {
            let (_directory, state, identity) = fixture();
            let mut inner = Inner::default();
            let (_, mut receiver) = pending(&mut inner, "chat", "card", &identity);
            let before = settings(&state);
            fs::create_dir(state.data_path.with_extension("json.tmp")).unwrap();
            assert!(answer_access(&state, &mut inner, &own(), "chat", "card", decision).is_err());
            assert!(inner.access.contains_key("card"));
            assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert_eq!(inner.grants.decision("chat", &AppKey::of(&identity)), Grant::Unknown);
            assert_eq!(settings(&state), before);
            assert_eq!(disk_settings(&state), before);
            fs::remove_dir(state.data_path.with_extension("json.tmp")).unwrap();
            let updated = answer_access(&state, &mut inner, &own(), "chat", "card", decision).unwrap().unwrap();
            assert_eq!(receiver.try_recv().unwrap(), decision);
            assert_eq!(disk_settings(&state), updated);
            assert!(!inner.access.contains_key("card"));
            if decision == AccessDecision::Always {
                assert_eq!(inner.grants.decision("chat", &AppKey::of(&identity)), Grant::Unknown);
                assert_eq!(updated.always_allow.len(), 1);
            } else {
                assert_eq!(inner.grants.decision("chat", &AppKey::of(&identity)), Grant::Denied);
                assert_eq!(updated.never_allow, ["com.example.App"]);
            }
        }
    }

    #[test]
    fn stop_and_stopped_tokens_prevent_late_always_answers() {
        let (_directory, state, identity) = fixture();
        let mut inner = Inner::default();
        let (token, mut receiver) = pending(&mut inner, "chat", "card", &identity);
        cancel_replies(ComputerUseManager::stop_chat(&mut inner, "chat"));
        assert!(token.is_stopped());
        assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Cancelled);
        assert!(answer_access(&state, &mut inner, &own(), "chat", "card", AccessDecision::Always).is_err());
        let (token, mut receiver) = pending(&mut inner, "chat", "cancelled", &identity);
        token.stop(); // A tool abort may stop the token before the wait loop removes its card.
        assert!(answer_access(&state, &mut inner, &own(), "chat", "cancelled", AccessDecision::Always).is_err());
        assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Cancelled);
        assert!(settings(&state).always_allow.is_empty());
    }

    #[test]
    fn revocation_is_exact_clears_all_chats_and_cancels_every_pending_and_queued_request() {
        let (directory, state, identity) = fixture();
        let other_path = directory.path().join("Other.app");
        fs::create_dir(&other_path).unwrap();
        let other = AppIdentity { path: Some(other_path.canonicalize().unwrap()), ..identity.clone() };
        let mut inner = Inner::default();
        save_always(&state, &mut inner, &identity, "first");
        save_always(&state, &mut inner, &other, "other");
        let key = AppKey::of(&identity);
        for task in ["a", "b"] { inner.grants.allow(task, key.clone()); }
        inner.grants.deny("denied", key.clone());
        inner.grants.allow("a", AppKey::of(&other));
        let (pending_token, mut receiver) = pending(&mut inner, "other-chat", "pending", &other);
        let queued = StopToken::default();
        inner.requests.insert("queued".into(), ("queued-chat".into(), queued.clone()));
        let revoke_key = AppKey::persistent(Some("COM.EXAMPLE.APP"), identity.path.as_ref().unwrap().to_str().unwrap()).unwrap();
        let updated = revoke_app(&state, &mut inner, &revoke_key).unwrap();
        assert_eq!(updated.always_allow, [persistable_grant(&other).unwrap()]);
        assert_eq!(disk_settings(&state), updated);
        for task in ["a", "b", "chat"] {
            assert_eq!(effective_access(&updated, &inner, &own(), task, &identity), Ok(Grant::Unknown));
        }
        assert_eq!(inner.grants.decision("denied", &key), Grant::Denied);
        assert_eq!(inner.grants.decision("a", &AppKey::of(&other)), Grant::Allowed);
        assert!(pending_token.is_stopped() && queued.is_stopped());
        assert!(inner.requests.values().all(|(_, token)| token.is_stopped()));
        assert!(inner.access.is_empty());
        assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Cancelled);
        assert!(answer_access(&state, &mut inner, &own(), "other-chat", "pending", AccessDecision::Always).is_err());
    }

    #[test]
    fn settings_cannot_create_or_replace_grants_and_disabling_keeps_them_inactive() {
        let (_directory, state, identity) = fixture();
        let mut inner = Inner::default();
        let input = || serde_json::from_value::<ComputerUseSettings>(json!({
            "enabled": true, "showAgentCursor": false, "neverAllow": [],
            "alwaysAllow": [{ "name": "Forged", "bundleId": "com.example.Evil", "path": "/tmp/Evil.app" }],
        })).unwrap();
        save_settings(&state, &mut inner, input()).unwrap();
        assert!(settings(&state).always_allow.is_empty());
        save_always(&state, &mut inner, &identity, "card");
        let saved = settings(&state).always_allow;
        save_settings(&state, &mut inner, input()).unwrap();
        assert_eq!(settings(&state).always_allow, saved);
        let (token, mut receiver) = pending(&mut inner, "other", "pending", &identity);
        let (off, disabled) = save_settings(&state, &mut inner, ComputerUseSettings::default()).unwrap();
        assert!(disabled && !off.enabled && token.is_stopped());
        assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Cancelled);
        assert_eq!(off.always_allow, saved);
        assert!(effective_access(&off, &inner, &own(), "chat", &identity).is_err());
        let (on, _) = save_settings(&state, &mut inner, input()).unwrap();
        assert_eq!(on.always_allow, saved);
        assert_eq!(effective_access(&on, &inner, &own(), "chat", &identity), Ok(Grant::Allowed));
    }

    #[test]
    fn adding_never_by_settings_or_card_removes_all_saved_paths_and_transient_allows() {
        for from_card in [false, true] {
            let (directory, state, identity) = fixture();
            let path = directory.path().join("Second.app");
            fs::create_dir(&path).unwrap();
            let second = AppIdentity { path: Some(path.canonicalize().unwrap()), bundle_id: Some("COM.EXAMPLE.APP".into()), ..identity.clone() };
            let unrelated = AppIdentity { bundle_id: None, ..identity.clone() };
            let mut inner = Inner::default();
            save_always(&state, &mut inner, &identity, "first");
            save_always(&state, &mut inner, &second, "second");
            save_always(&state, &mut inner, &unrelated, "unrelated");
            inner.grants.allow("other", AppKey::of(&identity));
            let (token, mut receiver) = pending(&mut inner, "other", "pending", &second);
            if from_card {
                let (_, mut answer) = pending(&mut inner, "new-chat", "never", &identity);
                answer_access(&state, &mut inner, &own(), "new-chat", "never", AccessDecision::Never).unwrap();
                assert_eq!(answer.try_recv().unwrap(), AccessDecision::Never);
            } else {
                save_settings(&state, &mut inner, ComputerUseSettings { enabled: true, never_allow: vec!["com.example.app".into()], ..ComputerUseSettings::default() }).unwrap();
            }
            assert!(token.is_stopped());
            assert_eq!(receiver.try_recv().unwrap(), AccessDecision::Cancelled);
            assert_eq!(settings(&state).always_allow, [persistable_grant(&unrelated).unwrap()]);
            assert_eq!(inner.grants.decision("other", &AppKey::of(&identity)), Grant::Unknown);
            assert!(effective_access(&settings(&state), &inner, &own(), "other", &identity).is_err());
            let (unblocked, _) = save_settings(&state, &mut inner, ComputerUseSettings { enabled: true, ..ComputerUseSettings::default() }).unwrap();
            assert_eq!(effective_access(&unblocked, &inner, &own(), "other", &identity), Ok(Grant::Unknown));
        }
    }

    #[test]
    fn answer_rechecks_disabled_blocked_and_denied_policy_and_excludes_chat_mode() {
        for restriction in ["disabled", "never", "hard", "own", "denied", "chat-mode"] {
            let (_directory, state, mut identity) = fixture();
            let mut inner = Inner::default();
            match restriction {
                "disabled" => state.data.lock().unwrap().computer_use.enabled = false,
                "never" => state.data.lock().unwrap().computer_use.never_allow.push("COM.EXAMPLE.APP".into()),
                "hard" => identity.bundle_id = Some("com.apple.Terminal".into()),
                "own" => identity.pid = Some(100),
                "denied" => inner.grants.deny("chat", AppKey::of(&identity)),
                "chat-mode" => {
                    let task: TaskRecord = serde_json::from_value(json!({
                        "id": "chat", "kind": "chat", "name": "Chat", "projectId": null,
                        "workspacePath": "/tmp/scratch", "worktreePath": null, "branch": null,
                        "usesWorktree": false, "providerId": "p", "modelId": "m", "thinkingLevel": "off",
                        "sessionFile": null, "archived": false, "lastError": null, "createdAt": "now", "updatedAt": "now",
                    })).unwrap();
                    state.data.lock().unwrap().tasks.push(task);
                }
                _ => unreachable!(),
            }
            let (_, _receiver) = pending(&mut inner, "chat", "card", &identity);
            assert!(answer_access(&state, &mut inner, &own(), "chat", "card", AccessDecision::Always).is_err(), "{restriction}");
            assert!(settings(&state).always_allow.is_empty());
        }
    }

    #[test]
    fn simultaneous_revoke_and_answer_cannot_resurrect_a_saved_grant() {
        for _ in 0..8 {
            let (_directory, state, identity) = fixture();
            let state = Arc::new(state);
            let mut inner = Inner::default();
            let (token, _receiver) = pending(&mut inner, "chat", "card", &identity);
            let inner = Arc::new(Mutex::new(inner));
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let answer = {
                let state = state.clone();
                let inner = inner.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    answer_access(&state, &mut inner.lock().unwrap(), &own(), "chat", "card", AccessDecision::Always)
                })
            };
            let revoke = {
                let state = state.clone();
                let inner = inner.clone();
                let key = AppKey::of(&identity);
                std::thread::spawn(move || {
                    barrier.wait();
                    revoke_app(&state, &mut inner.lock().unwrap(), &key)
                })
            };
            let _ = answer.join().unwrap();
            revoke.join().unwrap().unwrap();
            assert!(settings(&state).always_allow.is_empty());
            assert!(disk_settings(&state).always_allow.is_empty());
            assert!(token.is_stopped());
            assert!(request_settings(&state, "chat", &token).is_err());
            assert_eq!(inner.lock().unwrap().grants.decision("chat", &AppKey::of(&identity)), Grant::Unknown);
        }
    }
}
