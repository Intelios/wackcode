//! `Mac`: the real `Desktop`. Accessibility work runs on each app's engine thread
//! (`engine.rs`); pictures go through ScreenCaptureKit; input goes to the app's process.
//!
//! Invariants (see `ladder.rs` and lib.rs for why):
//! - Every action tries Accessibility first and reads its write back. Raw input follows only
//!   where `ladder::next` allows it, and its answer says it is unverified.
//! - A password field, or secure input anywhere, refuses typing, focus, keys and clicks into it.
//! - Typing never replaces text unless asked to (`replace`); what a replace wrote over comes back,
//!   at most 500 characters, when it could be read.
//! - A menu command that reads disabled while the app is in the background is refused with the
//!   way forward (the screen), never pressed: macOS would hold it until the app came forward.
//! - Right-clicks and controls that open a pop-up menu are refused in the background: the menu
//!   would take the keyboard from the user's app.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use async_trait::async_trait;
use objc2_core_foundation::{CFRange, CGPoint};

use crate::{
    backend::{Act, AppInfo, Button, Desktop, Done, Fingerprint, Found, Permissions, Target, Tree, Via, WindowInfo},
    engine::{AppThread, Engine},
    geometry::Rect,
    keys::Layout,
    ladder::{self, AppKind, Next, Reason, Step},
    native::{
        apps, ax, capture,
        capture::{Shot, WindowShot, Zoom},
        clipboard, input, keyboard, permissions,
    },
    outline::Node,
    policy::{AppIdentity, OwnIdentity},
};

pub struct Mac {
    engine: Engine,
    own: OwnIdentity,
    layout: Mutex<Layout>,
    home: Option<PathBuf>,
}

impl Mac {
    /// Main thread only: reads the keyboard layout.
    pub fn new() -> Self {
        Self { engine: Engine::default(), own: OwnIdentity::current(), layout: Mutex::new(keyboard::current()), home: std::env::var_os("HOME").map(PathBuf::from) }
    }

    /// Main thread only: reads the keyboard layout again (the user may have switched it).
    pub fn refresh_layout(&self) {
        *self.layout.lock().unwrap_or_else(|e| e.into_inner()) = keyboard::current();
    }
}

impl Default for Mac {
    fn default() -> Self {
        Self::new()
    }
}

fn info(app: apps::RunningApp) -> AppInfo {
    AppInfo { pid: app.pid, identity: app.identity(), frontmost: app.frontmost, hidden: app.hidden }
}

/// The AX window with window-server number `number`: the private mapping, else the same pid's
/// window with the same frame.
fn find_window(thread: &AppThread, number: u32) -> Result<ax::AxWindow, Reason> {
    let windows = ax::windows(&thread.app);
    let listed = input::all_windows();
    windows
        .into_iter()
        .find(|window| {
            ax::window_number(&window.element) == Some(number)
                || window.frame.is_some_and(|frame| listed.iter().any(|entry| entry.number == number && entry.pid == thread.pid && entry.bounds.approx_eq(&frame)))
        })
        .ok_or(Reason::Unreachable)
}

fn number_of(window: &ax::AxWindow, pid: i32, listed: &[crate::geometry::WindowEntry]) -> Option<u32> {
    ax::window_number(&window.element).or_else(|| {
        let frame = window.frame?;
        listed.iter().find(|entry| entry.pid == pid && entry.bounds.approx_eq(&frame)).map(|entry| entry.number)
    })
}

/// Turns walk handles into ref numbers.
fn to_refs(node: &mut Node, elements: &[ax::Element], registry: &mut ax::Registry) {
    node.handle = registry.ref_for(&elements[node.handle]) as usize;
    for child in &mut node.children {
        to_refs(child, elements, registry);
    }
}

const TEXT_ROLES: &[&str] = &["AXTextArea", "AXTextField", "AXComboBox"];
const MENU_ROLES: &[&str] = &["AXPopUpButton", "AXMenuButton"];
const PRESS_ACTIONS: [&str; 3] = ["AXPress", "AXConfirm", "AXPick"];

fn role(element: &ax::Element) -> String {
    ax::string(element, "AXRole").unwrap_or_default()
}

fn element_for(thread: &AppThread, target: Target) -> Result<Option<ax::Element>, Reason> {
    let element = match target {
        Target::Ref(number) | Target::At { element: Some(number), .. } => {
            let element = thread.registry.element(number).cloned().ok_or_else(|| Reason::Other(format!("e{number} is from an element that is gone or from another app: take a new computer_snapshot.")))?;
            if !ax::is_alive(&element) || ax::pid(&element) != Some(thread.pid) {
                return Err(Reason::Other(format!("e{number} no longer exists: take a new computer_snapshot.")));
            }
            Some(element)
        }
        Target::At { element: None, .. } => None,
        Target::Focused => Some(ax::focused(&thread.app).ok_or(Reason::NoElement)?),
    };
    Ok(element)
}

fn point_of(target: Target, element: Option<&ax::Element>) -> Option<(f64, f64)> {
    match target {
        Target::At { point, .. } => Some(point),
        _ => element.and_then(|element| ax::frame(element)).map(|frame| frame.center()),
    }
}

fn clip500(text: &str) -> String {
    text.chars().take(500).collect()
}

fn click(thread: &AppThread, window: u32, target: Target, button: Button, count: u32, modifiers: crate::keys::Modifiers, kind: AppKind) -> Result<Done, Reason> {
    let element = element_for(thread, target)?;
    let point = point_of(target, element.as_ref());
    let raw = |reason: Reason| -> Result<Done, Reason> {
        match (ladder::next(Step::Click, &reason, kind), point) {
            (Next::Raw, Some((x, y))) => {
                let button = if button == Button::Right { input::Button::Right } else { input::Button::Left };
                input::click_to_pid(thread.pid, window, CGPoint::new(x, y), button, count, &modifiers).map_err(Reason::Other)?;
                Ok(Done { what: format!("clicked at ({x:.0}, {y:.0})"), via: Via::Raw, note: Some("posted to the app as a raw click: check the result".into()), replaced: None })
            }
            _ => Err(reason),
        }
    };
    let Some(element) = element else { return raw(Reason::NoElement) };
    if ax::is_secure(&element) {
        return Err(Reason::Secure);
    }
    if button == Button::Right {
        return Err(Reason::OpensMenu);
    }
    let role = role(&element);
    if MENU_ROLES.contains(&role.as_str()) {
        return Err(Reason::OpensMenu);
    }
    if ax::boolean(&element, "AXEnabled") == Some(false) {
        return Err(Reason::Disabled);
    }
    let described = ax::describe(&element);
    if count == 1 && !modifiers.any() {
        let actions = ax::actions(&element);
        if TEXT_ROLES.contains(&role.as_str()) {
            // A click in text focuses it and places the caret.
            let _ = ax::set_bool(&element, "AXFocused", true);
            if let Some((x, y)) = point
                && let Some(range) = ax::range_for_position(&element, x, y)
                && ax::select_range(&element, CFRange { location: range.location, length: 0 }).is_ok()
            {
                return Ok(Done { what: format!("placed the caret in {described}"), via: Via::Ax, note: None, replaced: None });
            }
            return Ok(Done { what: format!("focused {described}"), via: Via::Ax, note: None, replaced: None });
        }
        if let Some(action) = PRESS_ACTIONS.iter().find(|action| actions.iter().any(|name| name == *action)) {
            return match ax::perform(&element, action) {
                Ok(()) => Ok(Done { what: format!("pressed {described}"), via: Via::Ax, note: None, replaced: None }),
                Err(error) => Err(Reason::Other(ax::describe_error(error, "pressing it"))),
            };
        }
        // Rows and cells are chosen by selecting them.
        if matches!(role.as_str(), "AXRow" | "AXCell" | "AXOutlineRow") && ax::settable(&element, "AXSelected") && ax::set_bool(&element, "AXSelected", true).is_ok() {
            return Ok(Done { what: format!("selected {described}"), via: Via::Ax, note: None, replaced: None });
        }
    }
    raw(Reason::NotPressable)
}

fn type_text(thread: &AppThread, target: Target, text: &str, replace: bool, kind: AppKind, app: &str, layout: &Layout) -> Result<Done, Reason> {
    if input::secure_input_on() {
        return Err(Reason::Secure);
    }
    let element = match element_for(thread, target)? {
        Some(element) => element,
        None => ax::focused(&thread.app).ok_or(Reason::NoElement)?,
    };
    if ax::is_secure(&element) {
        return Err(Reason::Secure);
    }
    let described = ax::describe(&element);
    let before = ax::value_text(&element);
    if replace {
        if ax::settable(&element, "AXValue") && ax::set_string(&element, "AXValue", text).is_ok() && ax::value_text(&element).as_deref() == Some(text) {
            return Ok(Done { what: format!("replaced the text of {described}"), via: Via::Ax, note: None, replaced: before.as_deref().filter(|old| !old.is_empty()).map(clip500) });
        }
        let length = before.as_deref().map(|value| value.encode_utf16().count()).unwrap_or(0) as isize;
        if ax::select_range(&element, CFRange { location: 0, length }).is_ok()
            && ax::set_string(&element, "AXSelectedText", text).is_ok()
            && ax::value_text(&element).as_deref() == Some(text)
        {
            return Ok(Done { what: format!("replaced the text of {described}"), via: Via::Ax, note: None, replaced: before.as_deref().filter(|old| !old.is_empty()).map(clip500) });
        }
        return Err(Reason::Other(ladder::advice(Step::Type, &Reason::WriteUnsupported, app)));
    }
    if ax::string(&element, "AXSelectedText").is_some_and(|selected| !selected.is_empty()) {
        return Err(Reason::WouldReplace);
    }
    let reason = match ax::set_string(&element, "AXSelectedText", text) {
        Ok(()) => {
            let after = ax::value_text(&element);
            if after != before && after.as_deref().is_some_and(|after| after.contains(text.trim())) {
                return Ok(Done { what: format!("typed into {described}"), via: Via::Ax, note: None, replaced: None });
            }
            Reason::WriteIgnored
        }
        Err(_) => Reason::WriteUnsupported,
    };
    if ladder::next(Step::Type, &reason, kind) != Next::Raw {
        return Err(reason);
    }
    // Raw text goes to whatever has focus, so the target must hold it.
    let _ = ax::set_bool(&element, "AXFocused", true);
    let focused = ax::focused(&thread.app);
    let holds_focus = focused.as_ref().is_some_and(|focused| ax::Key(focused.clone()) == ax::Key(element.clone()));
    if !holds_focus && target != Target::Focused {
        return Err(Reason::Other(format!("{app} didn't take the text through Accessibility, and the keyboard focus couldn't be put in {described}, so typing would land elsewhere. {}", ladder::advice(Step::Type, &Reason::WriteUnsupported, app))));
    }
    if focused.as_ref().is_some_and(|focused| ax::is_secure(focused)) {
        return Err(Reason::Secure);
    }
    input::text_to_pid(thread.pid, text, layout, &|| false).map_err(Reason::Other)?;
    Ok(Done { what: format!("typed into {described}"), via: Via::Raw, note: Some("typed as key events: check the result".into()), replaced: None })
}

fn scroll(thread: &AppThread, target: Target, dx: f64, dy: f64, kind: AppKind) -> Result<Done, Reason> {
    let element = element_for(thread, target)?;
    if let Some(element) = &element
        && ax::scroll(element, dx, dy)
    {
        return Ok(Done { what: format!("scrolled {}", ax::describe(element)), via: Via::Ax, note: None, replaced: None });
    }
    let reason = Reason::NoScrollBar;
    match (ladder::next(Step::Scroll, &reason, kind), point_of(target, element.as_ref())) {
        (Next::Raw, Some((x, y))) => {
            input::scroll_to_pid(thread.pid, CGPoint::new(x, y), dx, dy).map_err(Reason::Other)?;
            Ok(Done { what: format!("scrolled at ({x:.0}, {y:.0})"), via: Via::Raw, note: Some("posted to the app as a raw scroll: check the result".into()), replaced: None })
        }
        _ => Err(reason),
    }
}

fn menu(thread: &AppThread, path: &[String], app: &str) -> Result<Done, Reason> {
    let item = ax::menu_item(&thread.app, path).map_err(Reason::Other)?;
    let joined = path.join(" › ");
    if ax::boolean(&item, "AXEnabled") == Some(false) {
        let front = apps::frontmost_pid() == Some(thread.pid);
        return Err(Reason::Other(if front {
            format!("{joined} is disabled.")
        } else {
            format!(
                "{joined} reads disabled while {app} is in the background: macOS holds commands on the front document, and their shortcuts, until the app is in front. Do it another way (typing into a field by ref, or computer_open with a file), or tell the user what is left for them to do."
            )
        }));
    }
    ax::perform(&item, "AXPress").map_err(|error| Reason::Other(ax::describe_error(error, "choosing the menu item")))?;
    Ok(Done { what: format!("chose {joined}"), via: Via::Ax, note: None, replaced: None })
}

fn perform(thread: &mut AppThread, window: u32, act: Act, kind: AppKind, app: &str, layout: &Layout) -> Result<Done, Reason> {
    match act {
        Act::Click { target, button, count, modifiers } => click(thread, window, target, button, count, modifiers, kind),
        Act::Type { target, text, replace } => type_text(thread, target, &text, replace, kind, app, layout),
        Act::Key { chord, times } => {
            if ax::focused_is_secure(&thread.app) || input::secure_input_on() {
                return Err(Reason::Secure);
            }
            for _ in 0..times.clamp(1, 50) {
                input::key_to_pid(thread.pid, &chord).map_err(Reason::Other)?;
            }
            Ok(Done { what: "pressed".into(), via: Via::Raw, note: Some("keys go to whatever has focus in the app: check the result".into()), replaced: None })
        }
        Act::Scroll { target, dx, dy } => scroll(thread, target, dx, dy, kind),
        Act::Drag { path } => match ladder::next(Step::Drag, &Reason::NoAxDrag, kind) {
            Next::Raw => {
                let points: Vec<CGPoint> = path.iter().map(|(x, y)| CGPoint::new(*x, *y)).collect();
                input::drag_to_pid(thread.pid, window, &points).map_err(Reason::Other)?;
                Ok(Done { what: "dragged".into(), via: Via::Raw, note: Some("posted to the app as a raw drag: check the result".into()), replaced: None })
            }
            Next::Stop => Err(Reason::NoAxDrag),
        },
        Act::Menu { path } => menu(thread, &path, app),
        Act::Focus { target } => {
            let element = element_for(thread, target)?.ok_or(Reason::NoElement)?;
            if ax::is_secure(&element) {
                return Err(Reason::Secure);
            }
            ax::set_bool(&element, "AXFocused", true).map_err(|error| Reason::Other(ax::describe_error(error, "focusing it")))?;
            Ok(Done { what: format!("focused {}", ax::describe(&element)), via: Via::Ax, note: None, replaced: None })
        }
    }
}

fn app_name(pid: i32) -> String {
    apps::name_of(pid).0
}

#[async_trait]
impl Desktop for Mac {
    fn own(&self) -> OwnIdentity {
        self.own.clone()
    }

    fn layout(&self) -> Layout {
        self.layout.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn permissions(&self) -> Permissions {
        Permissions { accessibility: permissions::accessibility(), screen_recording: permissions::screen_recording_preflight() }
    }

    async fn running(&self) -> Vec<AppInfo> {
        apps::running().into_iter().map(info).collect()
    }

    async fn installed(&self, query: &str, limit: usize) -> Vec<AppIdentity> {
        apps::installed(query, self.home.as_deref(), limit).iter().map(apps::InstalledApp::identity).collect()
    }

    async fn resolve(&self, query: &str) -> Result<Found, String> {
        match apps::resolve(query, self.home.as_deref())? {
            apps::Resolved::Running(app) => Ok(Found::Running(info(app))),
            apps::Resolved::Installed(app) => Ok(Found::Installed(app.identity())),
        }
    }

    async fn launch(&self, app: &AppIdentity, file: Option<PathBuf>) -> Result<i32, String> {
        let path = app.path.clone().ok_or("That app has no bundle to launch.")?;
        tokio::task::spawn_blocking(move || apps::launch(&path, file.as_deref())).await.map_err(|_| "Launching was interrupted.".to_string())?
    }

    fn kind(&self, app: &AppIdentity) -> AppKind {
        match &app.path {
            Some(path) if ax::wants_manual(path) => AppKind::Chromium,
            _ => AppKind::Native,
        }
    }

    async fn windows(&self, pid: i32) -> Result<Vec<WindowInfo>, String> {
        self.engine
            .run(pid, move |thread| {
                let on_screen = input::window_list();
                let listed = input::all_windows();
                ax::windows(&thread.app)
                    .into_iter()
                    .filter_map(|window| {
                        let id = number_of(&window, thread.pid, &listed)?;
                        Some(WindowInfo {
                            id,
                            title: window.title,
                            frame: window.frame,
                            main: window.main,
                            minimized: window.minimized,
                            on_screen: on_screen.iter().any(|entry| entry.number == id),
                            kind: window.subrole,
                        })
                    })
                    .collect()
            })
            .await
    }

    async fn tree(&self, pid: i32, window: u32, from: Option<u32>) -> Result<Tree, String> {
        let manual = apps::running().into_iter().find(|app| app.pid == pid).and_then(|app| app.bundle_path).is_some_and(|path| ax::wants_manual(Path::new(&path)));
        self.engine
            .run(pid, move |thread| {
                if manual && !thread.manual {
                    ax::enable_manual_accessibility(&thread.app);
                    thread.manual = true;
                    // Chromium builds the tree on request; give it a moment.
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                let found = find_window(thread, window).map_err(|_| "That window is closed, minimized or in another Space. computer_apps lists the app's windows.".to_string())?;
                let frame = found.frame.unwrap_or_default();
                let root = match from {
                    Some(number) => thread.registry.element(number).cloned().filter(|element| ax::is_alive(element)).ok_or_else(|| format!("e{number} no longer exists: take a new computer_snapshot."))?,
                    None => found.element.clone(),
                };
                let (mut node, elements, truncated) = ax::walk(&root, &ax::SNAPSHOT_LIMITS);
                to_refs(&mut node, &elements, &mut thread.registry);
                Ok(Tree { root: node, truncated, window: frame })
            })
            .await?
    }

    async fn glance(&self, pid: i32, window: u32) -> Option<Node> {
        self.engine
            .run(pid, move |thread| {
                let found = find_window(thread, window).ok()?;
                let (mut node, elements, _) = ax::walk(&found.element, &ax::SEARCH_LIMITS);
                to_refs(&mut node, &elements, &mut thread.registry);
                Some(node)
            })
            .await
            .ok()
            .flatten()
    }

    async fn menus(&self, pid: i32, path: Vec<String>) -> Result<Vec<String>, String> {
        self.engine.run(pid, move |thread| ax::menu_list(&thread.app, &path)).await?
    }

    async fn picture(&self, window: u32, scale: f64, region: Option<Zoom>) -> Result<Shot, String> {
        capture::window(WindowShot { number: window, scale, region }).await
    }

    async fn act(&self, pid: i32, window: u32, act: Act) -> Result<Done, Reason> {
        let app = app_name(pid);
        let kind = apps::running()
            .into_iter()
            .find(|running| running.pid == pid)
            .and_then(|running| running.bundle_path)
            .map(|path| if ax::wants_manual(&path) { AppKind::Chromium } else { AppKind::Native })
            .unwrap_or(AppKind::Native);
        let layout = self.layout();
        self.engine.run(pid, move |thread| perform(thread, window, act, kind, &app, &layout)).await.map_err(Reason::Other)?
    }

    async fn fingerprint(&self, pid: i32, target: Option<u32>) -> Fingerprint {
        self.engine
            .run(pid, move |thread| {
                let focused = ax::focused(&thread.app).map(|element| ax::describe(&element));
                let listed = input::all_windows();
                let windows = ax::windows(&thread.app).iter().filter_map(|window| Some((number_of(window, thread.pid, &listed)?, window.title.clone()))).collect();
                let element = target.and_then(|number| thread.registry.element(number).cloned()).filter(|element| ax::is_alive(element));
                let value = element.as_ref().filter(|element| !ax::is_secure(element)).and_then(|element| ax::value_text(element));
                Fingerprint { focused, windows, frontmost: apps::frontmost_pid(), value, target: element.as_ref().map(|element| ax::describe(element)) }
            })
            .await
            .unwrap_or_default()
    }

    fn screen_windows(&self) -> Vec<crate::geometry::WindowEntry> {
        let skip: Vec<u32> = super::cursor::window_number().into_iter().collect();
        input::window_list().into_iter().filter(|window| !skip.contains(&window.number)).collect()
    }

    fn frontmost(&self) -> Option<i32> {
        apps::frontmost_pid()
    }

    async fn bring_forward(&self, pid: i32) -> bool {
        self.engine.run(pid, |thread| ax::set_bool(&thread.app, "AXFrontmost", true).is_ok()).await.unwrap_or(false)
    }

    fn seconds_since_typing(&self) -> f64 {
        input::seconds_since_typing()
    }

    async fn icon(&self, app: &AppIdentity) -> Option<Vec<u8>> {
        let path = app.path.clone()?;
        tokio::task::spawn_blocking(move || apps::icon_png(&path)).await.ok().flatten()
    }

    async fn clipboard_read(&self) -> Result<Option<String>, String> {
        match clipboard::read() {
            clipboard::Read::Text(text) => Ok(Some(text)),
            clipboard::Read::Empty => Ok(None),
            clipboard::Read::Private => Err("The clipboard holds something a password manager marked private, so it isn't read.".into()),
        }
    }

    async fn clipboard_write(&self, text: &str) -> Result<(), String> {
        if clipboard::write(text) { Ok(()) } else { Err("The clipboard couldn't be written.".into()) }
    }
}

/// A window's frame in points, for a caller that has only its id.
pub fn window_frame(number: u32) -> Option<Rect> {
    input::all_windows().into_iter().find(|entry| entry.number == number).map(|entry| entry.bounds)
}
