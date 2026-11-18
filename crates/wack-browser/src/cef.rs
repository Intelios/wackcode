//! The engine: one CEF (Chromium) instance for the whole app, one shared profile, a page per tab.
//!
//! Lifetime:
//! - Nothing starts at launch. `backend` only records the configuration; CEF starts the first time
//!   a tab is created, by the user or the agent (docs/security.md: no traffic on launch).
//! - It runs on the app's main thread with an external message pump: CEF asks for work through
//!   `on_schedule_message_pump_work`, and a 30 Hz heartbeat backs that up, both on the main queue.
//! - `shutdown` closes every page and stops CEF; the app calls it when it quits.
//!
//! Pages:
//! - Every page is a native view (Alloy style; Chrome style does not render as a child view)
//!   inside the host view the UI gives `set_host`, kept beneath GPUI's overlay plane so menus and
//!   cards draw over it.
//! - A page not on screen is *parked*: moved outside the window but never hidden, so Chromium
//!   keeps rendering it and the agent can keep using it and take screenshots. `place` moves a page
//!   onto a pane's rectangle and back.
//! - Pages are created and closed only through `Backend` (by `ChatTabs`), never by a view.
//! - Closing a page never closes the window: `do_close` takes the page's view out itself, since
//!   CEF's default would send the close event to WackCode's own window.
//!
//! Main-thread state lives in a thread-local. CEF calls back synchronously, so the state is never
//! borrowed across a call into CEF.
//!
//! Security (docs/security.md):
//! - DevTools is reached only in process (`send_dev_tools_message`); there is no remote-debugging
//!   port or pipe.
//! - Chromium's own background services are switched off or their hosts mapped to nowhere
//!   (`QUIET_SWITCHES`, `DISABLED_FEATURES`, `BLOCKED_HOSTS`): idle, it reaches no host.
//! - While the agent drives a tab, every main-frame navigation passes `policy`: redirects and
//!   script navigations too, re-checked after a DNS lookup off the main thread.
//! - The profile folder is private (0700) and is what protects the user's sign-ins, as 0600
//!   protects `secrets.json`. Chromium's Keychain encryption is off (`use-mock-keychain`, owner's
//!   call): its key prompted after every rebuild of the ad hoc signed app, and until the prompt was
//!   answered the network service hung.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
};

// The wrap macros name CEF's traits unqualified, so they must all be in scope.
use cef::*;
use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use serde_json::{Value as Json, json};
use tokio::sync::oneshot;

use crate::{
    main_thread,
    policy::{self, Verdict},
    state::{Dialog, DialogKind, Level},
    tabs::{Backend, BoxFuture, Nav, Tab, TabId},
};

mod appkit;

/// Where the engine keeps its data.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// The browser's folder in the app's data folder (`<data>/browser`): the profile and the log.
    pub root: PathBuf,
}

/// A shown page's rectangle in the host view, in points from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame2 {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub corner_radius: f64,
}

const NO_BUNDLE: &str =
    "The browser needs WackCode to run from its app bundle. Start the dev app with `cargo dev`.";

/// Chromium's background traffic, off (docs/security.md, "Network").
const QUIET_SWITCHES: &[&str] = &[
    "disable-background-networking",
    "disable-component-update",
    "disable-sync",
    "disable-domain-reliability",
    "disable-breakpad",
    "no-pings",
    "no-first-run",
    "no-default-browser-check",
    // The profile is protected by its 0700 folder, never the Keychain (see the header).
    "use-mock-keychain",
    // Parked pages keep rendering, so the agent can drive and capture them.
    "disable-backgrounding-occluded-windows",
    "disable-renderer-backgrounding",
    "disable-background-timer-throttling",
];

/// Chromium features that call Google while idle. `AimEnabled` is AI Mode's kill switch: with it
/// on, a Google default search engine makes Chromium ask www.google.com/async/folae whether the
/// user may have AI Mode, at startup and after sign-in changes. That host can't be mapped away
/// like the others, since the user may browse it.
const DISABLED_FEATURES: &[&str] = &["AimEnabled"];

/// Adds `extra` to a `--disable-features` list. A second `--disable-features` switch would replace
/// CEF's own list, not add to it.
fn with_features(existing: &str, extra: &[&str]) -> String {
    let mut features: Vec<&str> = existing.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
    for feature in extra {
        if !features.contains(feature) {
            features.push(feature);
        }
    }
    features.join(",")
}

/// Hosts only Chromium's own services use (updates, network time, variations): mapped to
/// nowhere, so their requests fail on this Mac with no DNS lookup.
const BLOCKED_HOSTS: &[&str] = &[
    "update.googleapis.com",
    "clients2.google.com",
    "clientservices.googleapis.com",
    "*.gvt1.com",
];

/// A page parked outside the window: far to the left, its last size kept.
const PARK_X: f64 = -40_000.0;
const DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
/// How long a DevTools call may take before it is given up.
const CDP_TIMEOUT_MS: i64 = 30_000;

static CONFIG: OnceLock<EngineConfig> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
static PUMPING: AtomicBool = AtomicBool::new(false);
static NEXT_MESSAGE: AtomicI32 = AtomicI32::new(1);

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Stopped,
    /// `initialize` returned; waiting for the context.
    Starting,
    Ready,
    Failed(String),
}

struct Page {
    tab: Arc<Tab>,
    browser: Browser,
    _observer: Option<Registration>,
    /// Where it is now: `None` while parked.
    shown: Option<Frame2>,
    size: (f64, f64),
    /// Hosts that passed the address check after a lookup, so the retried navigation goes through.
    approved_hosts: HashSet<String>,
}

type Created = oneshot::Sender<Result<(), String>>;
type Answer = oneshot::Sender<Result<Json, String>>;

#[derive(Default)]
struct Main {
    phase: Phase,
    library: Option<cef::library_loader::LibraryLoader>,
    /// The UI's view that pages live in, and the overlay view they stay beneath.
    host: Option<(usize, usize)>,
    pages: HashMap<TabId, Page>,
    by_browser: HashMap<i32, TabId>,
    /// Pages asked for before the engine (or the host view) was ready.
    waiting: Vec<(Arc<Tab>, String, Created)>,
    /// Popup tabs whose page CEF is making.
    popups: HashMap<TabId, Arc<Tab>>,
    dialogs: HashMap<TabId, JsdialogCallback>,
    answers: HashMap<i32, (TabId, Answer)>,
}

thread_local! {
    static MAIN: RefCell<Main> = RefCell::new(Main::default());
}

fn with_main<T>(f: impl FnOnce(&mut Main) -> T) -> T {
    MAIN.with(|main| f(&mut main.borrow_mut()))
}

fn tab_of(id: TabId) -> Option<Arc<Tab>> {
    with_main(|main| {
        main.pages
            .get(&id)
            .map(|page| page.tab.clone())
            .or_else(|| main.popups.get(&id).cloned())
    })
}

fn browser_of(id: TabId) -> Option<Browser> {
    with_main(|main| main.pages.get(&id).map(|page| page.browser.clone()))
}

fn text(value: Option<&CefString>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// The message pump.

fn pump() {
    if !RUNNING.load(Ordering::Acquire) || PUMPING.swap(true, Ordering::AcqRel) {
        return;
    }
    cef::do_message_loop_work();
    PUMPING.store(false, Ordering::Release);
}

fn heartbeat() {
    if RUNNING.load(Ordering::Acquire) {
        pump();
        main_thread::post_after(33, heartbeat);
    }
}

// ---------------------------------------------------------------------------------------------
// Starting.

fn framework_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|_| NO_BUNDLE.to_string())?;
    let framework = exe
        .parent()
        .map(|dir| {
            dir.join(
                "../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
            )
        })
        .ok_or_else(|| NO_BUNDLE.to_string())?;
    if framework.exists() {
        Ok(exe)
    } else {
        Err(NO_BUNDLE.to_string())
    }
}

fn private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)
        .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("Could not protect {}: {e}", path.display()))
}

/// Loads Chromium's library into the process without starting it: no thread, no process, no
/// network. Call it first thing in `main`, before any other thread exists. Loading the framework
/// swaps the process's malloc zones (Chromium's allocator), and a thread freeing memory while
/// that happens crashes the app; so it is never loaded lazily, the first time a page is made.
/// Outside the app bundle (tests, `cargo run`) there is nothing to load and this does nothing.
pub fn load_library() {
    if with_main(|main| main.library.is_some()) {
        return;
    }
    let Ok(exe) = framework_path() else { return };
    let loader = cef::library_loader::LibraryLoader::new(&exe, false);
    if loader.load() {
        with_main(|main| main.library = Some(loader));
    }
}

/// Starts CEF. Main thread only; at most once per process.
fn start() -> Result<(), String> {
    let config = CONFIG
        .get()
        .ok_or("The browser was not configured.")?
        .clone();
    framework_path()?;
    if with_main(|main| main.library.is_none()) {
        return Err("Could not load the browser engine (Chromium Embedded Framework).".into());
    }
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    appkit::install();

    private_dir(&config.root)?;
    let profile = config.root.join("profile");
    private_dir(&profile)?;
    let settings = Settings {
        no_sandbox: 0,
        external_message_pump: 1,
        root_cache_path: CefString::from(profile.to_string_lossy().as_ref()),
        cache_path: CefString::from(profile.join("Default").to_string_lossy().as_ref()),
        persist_session_cookies: 1,
        log_file: CefString::from(config.root.join("browser.log").to_string_lossy().as_ref()),
        log_severity: cef::LogSeverity::WARNING,
        ..Default::default()
    };
    let args = cef::args::Args::new();
    let mut app = WackApp::new();
    if cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        return Err("The browser engine failed to start. Its log is in the app's data folder (browser/browser.log).".into());
    }
    RUNNING.store(true, Ordering::Release);
    heartbeat();
    Ok(())
}

wrap_app! {
    struct WackApp {}

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let browser_process = process_type.map(|t| t.to_string().is_empty()).unwrap_or(true);
            let Some(line) = command_line else { return };
            if !browser_process {
                return;
            }
            for switch in QUIET_SWITCHES {
                line.append_switch(Some(&CefString::from(*switch)));
            }
            let name = CefString::from("disable-features");
            let existing = CefString::from(&line.switch_value(Some(&name))).to_string();
            line.append_switch_with_value(Some(&name), Some(&CefString::from(with_features(&existing, DISABLED_FEATURES).as_str())));
            let rules = BLOCKED_HOSTS.iter().map(|host| format!("MAP {host} ~NOTFOUND")).collect::<Vec<_>>().join(", ");
            line.append_switch_with_value(Some(&CefString::from("host-resolver-rules")), Some(&CefString::from(rules.as_str())));
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(WackProcess::new())
        }
    }
}

wrap_browser_process_handler! {
    struct WackProcess {}

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            // Not from inside CEF's callback: flushing makes pages, which calls back into CEF.
            main_thread::post(|| {
                with_main(|main| main.phase = Phase::Ready);
                flush_waiting();
            });
        }

        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            main_thread::post_after(delay_ms.clamp(0, 33), pump);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Pages.

fn flush_waiting() {
    let ready = with_main(|main| main.phase == Phase::Ready && main.host.is_some());
    if !ready {
        return;
    }
    let waiting = with_main(|main| std::mem::take(&mut main.waiting));
    for (tab, url, created) in waiting {
        let _ = created.send(make_page(tab, &url));
    }
}

/// Asks for a page: now if the engine is ready, otherwise once it is.
fn request_page(tab: Arc<Tab>, url: String, created: Created) {
    let phase = with_main(|main| match &main.phase {
        Phase::Failed(error) => Err(error.clone()),
        Phase::Stopped => Ok(false),
        _ => Ok(true),
    });
    match phase {
        Err(error) => {
            let _ = created.send(Err(error));
            return;
        }
        Ok(false) => {
            with_main(|main| main.phase = Phase::Starting);
            if let Err(error) = start() {
                with_main(|main| main.phase = Phase::Failed(error.clone()));
                let _ = created.send(Err(error));
                return;
            }
        }
        Ok(true) => {}
    }
    with_main(|main| main.waiting.push((tab, url, created)));
    flush_waiting();
}

fn child_window_info(host: usize) -> WindowInfo {
    let rect = cef::Rect {
        x: PARK_X as i32,
        y: 0,
        width: DEFAULT_SIZE.0 as i32,
        height: DEFAULT_SIZE.1 as i32,
    };
    let mut info = WindowInfo::default().set_as_child(host as _, &rect);
    info.runtime_style = RuntimeStyle::ALLOY;
    info
}

fn make_page(tab: Arc<Tab>, url: &str) -> Result<(), String> {
    let host =
        with_main(|main| main.host).ok_or("The browser has no window to show pages in yet.")?;
    let info = child_window_info(host.0);
    let mut client = WackClient::new(tab.id);
    // Made blank and loaded once the DevTools domains are on, so the page's own document request
    // and its first console lines are captured too.
    let browser = cef::browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from("about:blank")),
        Some(&BrowserSettings::default()),
        None,
        None,
    )
    .ok_or("The browser could not open a page.")?;
    let id = tab.id;
    let ready = adopt_page(tab, browser);
    if url != "about:blank" {
        let url = url.to_string();
        // Off the main thread until the page confirms the domains are on, then back to load.
        std::thread::spawn(move || {
            let _ = ready.blocking_recv();
            main_thread::post(move || {
                if let Some(frame) = browser_of(id).and_then(|browser| browser.main_frame()) {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
            });
        });
    }
    Ok(())
}

/// Registers a page CEF made (ours or a popup), parks it and turns on the DevTools domains.
/// Answers once the page has confirmed them all (they are answered in order).
fn adopt_page(tab: Arc<Tab>, browser: Browser) -> oneshot::Receiver<Result<Json, String>> {
    let observer = browser
        .host()
        .and_then(|host| host.add_dev_tools_message_observer(Some(&mut WackDevTools::new(tab.id))));
    let id = tab.id;
    with_main(|main| {
        main.by_browser.insert(browser.identifier(), id);
        main.popups.remove(&id);
        main.pages.insert(
            id,
            Page {
                tab: tab.clone(),
                browser: browser.clone(),
                _observer: observer,
                shown: None,
                size: DEFAULT_SIZE,
                approved_hosts: HashSet::new(),
            },
        );
    });
    restack(id);
    position(id, None);
    let (confirmed, ready) = oneshot::channel();
    let mut confirmed = Some(confirmed);
    let methods = ["Runtime.enable", "Page.enable", "Network.enable", "Log.enable"];
    for (index, method) in methods.iter().enumerate() {
        let answer = if index + 1 == methods.len() { confirmed.take() } else { None };
        let _ = send(id, method, json!({}), answer);
    }
    tab.set_live(true);
    ready
}

fn view_of(id: TabId) -> Option<Retained<NSView>> {
    let host = browser_of(id)?.host()?;
    // SAFETY: CEF's window handle on macOS is the page's NSView, alive while the browser is.
    unsafe { Retained::retain(host.window_handle() as *mut NSView) }
}

fn host_view() -> Option<Retained<NSView>> {
    let (host, _) = with_main(|main| main.host)?;
    // SAFETY: the UI's view outlives the engine's pages (it is the window's content view).
    unsafe { Retained::retain(host as *mut NSView) }
}

/// Keeps a page's view beneath the UI's overlay plane.
fn restack(id: TabId) {
    let (Some(view), Some(host), Some((_, overlay))) =
        (view_of(id), host_view(), with_main(|main| main.host))
    else {
        return;
    };
    if overlay == 0 {
        return;
    }
    // SAFETY: the overlay view is a subview of `host`, alive as long as it is.
    if let Some(overlay) = unsafe { Retained::retain(overlay as *mut NSView) } {
        host.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(&overlay));
    }
}

/// The page's view, or one inside it, has the window's keyboard focus.
fn holds_focus(view: &NSView) -> bool {
    let Some(window) = view.window() else { return false };
    let Some(responder) = window.firstResponder() else { return false };
    // SAFETY: a plain class check and a view-hierarchy query on live AppKit objects.
    unsafe {
        let is_view: bool = msg_send![&*responder, isKindOfClass: objc2::class!(NSView)];
        if !is_view {
            return false;
        }
        let responder = &*(Retained::as_ptr(&responder) as *const NSView);
        responder.isDescendantOf(view)
    }
}

type PagePress = Box<dyn Fn(TabId)>;

thread_local! {
    /// Told of each press on a shown page, before it is routed (`on_page_press`).
    static ON_PAGE_PRESS: RefCell<Option<PagePress>> = RefCell::new(None);
}

/// Sets what is told of a press on a shown page, as the page takes the keyboard. The app moves
/// its own focus to that page's pane, because a press the page handles (in a text field, say)
/// never reaches the app's views. Called inside AppKit's event dispatch: it must not re-enter the
/// app synchronously.
pub fn on_page_press(callback: impl Fn(TabId) + 'static) {
    ON_PAGE_PRESS.with(|cell| *cell.borrow_mut() = Some(Box::new(callback)));
}

/// Before AppKit routes a press in `window` at `point` (window coordinates), the keyboard goes
/// where the press is, at once: to the page under it (Chromium would take it only after a round
/// trip through the page, so keys typed straight after the click went astray), or, off every
/// page, back to the app's view if a page has it.
fn before_press(window: *mut AnyObject, point: NSPoint) {
    let Some(host) = host_view() else { return };
    let Some(host_window) = host.window() else { return };
    if Retained::as_ptr(&host_window) as *mut AnyObject != window {
        return;
    }
    // SAFETY: hit-testing the live window's frame view, as AppKit is about to.
    let hit: Option<Retained<NSView>> = unsafe {
        let content: *mut AnyObject = msg_send![window, contentView];
        let frame: *mut AnyObject = msg_send![content, superview];
        let hit: *mut AnyObject = msg_send![frame, hitTest: point];
        Retained::retain(hit as *mut NSView)
    };
    let ids: Vec<TabId> = with_main(|main| main.pages.keys().copied().collect());
    let pages: Vec<(TabId, Retained<NSView>)> = ids.into_iter().filter_map(|id| Some((id, view_of(id)?))).collect();
    let pressed = hit.as_ref().and_then(|hit| pages.iter().find(|(_, page)| hit.isDescendantOf(page)));
    match pressed {
        Some((id, page)) => {
            let shown = with_main(|main| main.pages.get(id).is_some_and(|page| page.shown.is_some()));
            if !shown {
                return;
            }
            if !holds_focus(page) && let Some(hit) = &hit {
                host_window.makeFirstResponder(Some(hit));
            }
            ON_PAGE_PRESS.with(|cell| {
                if let Some(callback) = cell.borrow().as_ref() {
                    callback(*id);
                }
            });
        }
        None => {
            if pages.iter().any(|(_, page)| holds_focus(page)) {
                host_window.makeFirstResponder(Some(&host));
            }
        }
    }
}

/// Gives keyboard focus back to the app's own view when a page has it. Main thread only.
pub fn release_focus() {
    let Some(host) = host_view() else { return };
    let ids: Vec<TabId> = with_main(|main| main.pages.keys().copied().collect());
    for id in ids {
        if let Some(view) = view_of(id)
            && holds_focus(&view)
            && let Some(window) = host.window()
        {
            window.makeFirstResponder(Some(&host));
            return;
        }
    }
}

/// Moves a page onto `frame`, or parks it with `None`. A parked page never keeps the keyboard.
fn position(id: TabId, frame: Option<Frame2>) {
    let (Some(view), Some(host)) = (view_of(id), host_view()) else {
        return;
    };
    let size = with_main(|main| {
        let page = main.pages.get_mut(&id)?;
        if let Some(frame) = frame {
            page.size = (frame.width, frame.height);
        }
        page.shown = frame;
        Some(page.size)
    });
    let Some((width, height)) = size else { return };
    let host_height = host.bounds().size.height;
    let rect = match frame {
        Some(f) => NSRect::new(
            NSPoint::new(f.x, host_height - f.y - f.height),
            NSSize::new(f.width, f.height),
        ),
        None => NSRect::new(NSPoint::new(PARK_X, 0.0), NSSize::new(width, height)),
    };
    if frame.is_none()
        && holds_focus(&view)
        && let Some(window) = host.window()
    {
        window.makeFirstResponder(Some(&host));
    }
    view.setFrame(rect);
    view.setHidden(false);
    view.setWantsLayer(true);
    // SAFETY: a layer-backed view always has a layer.
    unsafe {
        let layer: *mut AnyObject = msg_send![&*view, layer];
        if !layer.is_null() {
            let radius = frame.map(|f| f.corner_radius).unwrap_or(0.0);
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
}

fn close_page(id: TabId) {
    let browser = with_main(|main| {
        main.dialogs.remove(&id);
        main.pages.get(&id).map(|page| page.browser.clone())
    });
    if let Some(host) = browser.and_then(|b| b.host()) {
        host.close_browser(1);
    }
}

// ---------------------------------------------------------------------------------------------
// DevTools, in process.

/// Sends one DevTools method; the answer arrives through `WackDevTools`.
fn send(id: TabId, method: &str, params: Json, answer: Option<Answer>) -> Result<(), String> {
    let browser = browser_of(id).ok_or("That tab has no page.")?;
    let message_id = NEXT_MESSAGE.fetch_add(1, Ordering::Relaxed);
    let message = json!({"id": message_id, "method": method, "params": params}).to_string();
    if let Some(answer) = answer {
        with_main(|main| main.answers.insert(message_id, (id, answer)));
        main_thread::post_after(CDP_TIMEOUT_MS, move || {
            if let Some((_, answer)) = with_main(|main| main.answers.remove(&message_id)) {
                let _ = answer.send(Err("The page did not answer in time.".into()));
            }
        });
    }
    let sent = browser
        .host()
        .map(|host| host.send_dev_tools_message(Some(message.as_bytes())))
        .unwrap_or(0);
    if sent == 1 {
        Ok(())
    } else {
        with_main(|main| main.answers.remove(&message_id));
        Err("The page is not ready for that yet.".into())
    }
}

wrap_dev_tools_message_observer! {
    struct WackDevTools {
        tab: TabId,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(&self, _browser: Option<&mut Browser>, message: Option<&[u8]>) -> i32 {
            let Ok(message) = serde_json::from_slice::<Json>(message.unwrap_or_default()) else { return 1 };
            if let Some(message_id) = message["id"].as_i64() {
                let answer = with_main(|main| main.answers.remove(&(message_id as i32)));
                if let Some((_, answer)) = answer {
                    let result = match message.get("error") {
                        Some(error) => Err(error["message"].as_str().unwrap_or("The page refused that.").to_string()),
                        None => Ok(message.get("result").cloned().unwrap_or(Json::Null)),
                    };
                    let _ = answer.send(result);
                }
            } else if let Some(method) = message["method"].as_str()
                && let Some(tab) = tab_of(self.tab)
            {
                tab.update(|state| state.apply_cdp(method, &message["params"]));
            }
            // Handled here: CEF need not parse it again.
            1
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The page's handlers.

wrap_client! {
    struct WackClient {
        tab: TabId,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(WackLifeSpan::new(self.tab))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(WackDisplay::new(self.tab))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(WackLoad::new(self.tab))
        }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(WackDialogs::new(self.tab))
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(WackRequests::new(self.tab))
        }
        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(WackKeys::new(self.tab))
        }
        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(WackDownloads::new(self.tab))
        }
        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(WackFocus::new(self.tab))
        }
    }
}

wrap_focus_handler! {
    struct WackFocus {
        tab: TabId,
    }

    impl FocusHandler {
        // A parked page never takes the keyboard, or keys would go to a page nobody can see. A
        // shown page takes it from a press (`before_press`), never from loading: Chromium
        // focuses a page as it loads, which would take the keyboard from the composer mid-word
        // whenever the user or the agent opened a page. A page that already has it keeps it.
        fn on_set_focus(&self, _browser: Option<&mut Browser>, source: FocusSource) -> i32 {
            let parked = with_main(|main| main.pages.get(&self.tab).is_none_or(|page| page.shown.is_none()));
            let loading = source.as_ref() == FocusSource::NAVIGATION.as_ref();
            let refuse = parked || (loading && !view_of(self.tab).is_some_and(|view| holds_focus(&view)));
            i32::from(refuse)
        }

    }
}

wrap_life_span_handler! {
    struct WackLifeSpan {
        tab: TabId,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: cef::WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&cef::PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<cef::DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // A popup becomes a tab of its own in the same chat, with `window.opener` intact so
            // sign-in popups work. Without a tab to put it in, it is refused.
            let (Some(parent), Some(host)) = (tab_of(self.tab), with_main(|main| main.host)) else { return 1 };
            let Some(child) = parent.spawn_popup() else { return 1 };
            if let Some(info) = window_info {
                *info = child_window_info(host.0);
            }
            if let Some(client) = client {
                *client = Some(WackClient::new(child.id));
            }
            with_main(|main| main.popups.insert(child.id, child));
            0
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            // Our own pages are adopted when `create_browser_sync` returns; this is for popups.
            let popup = with_main(|main| main.popups.get(&self.tab).cloned());
            if let (Some(tab), Some(browser)) = (popup, browser) {
                drop(adopt_page(tab, browser.clone()));
            }
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            // CEF's default sends the OS close event to the window holding the page, which is
            // WackCode's own window. Take the page's view out instead and let CEF finish.
            if let Some(view) = browser.and_then(|b| b.host()).map(|host| host.window_handle() as *mut NSView) {
                // SAFETY: the page's view, alive until CEF destroys the browser after this.
                if let Some(view) = unsafe { Retained::retain(view) } {
                    view.removeFromSuperview();
                }
            }
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let id = self.tab;
            let tab = with_main(|main| {
                if let Some(browser) = browser {
                    main.by_browser.remove(&browser.identifier());
                }
                main.answers.retain(|_, (tab, _)| *tab != id);
                main.dialogs.remove(&id);
                main.pages.remove(&id).map(|page| page.tab)
            });
            if let Some(tab) = tab {
                tab.set_live(false);
            }
        }
    }
}

wrap_display_handler! {
    struct WackDisplay {
        tab: TabId,
    }

    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.map(|f| f.is_main() == 1).unwrap_or(false)
                && let Some(tab) = tab_of(self.tab)
            {
                let url = text(url);
                tab.update(|state| state.set_address(&url));
            }
        }

        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(tab) = tab_of(self.tab) {
                let title = text(title);
                tab.update(|state| state.set_title(&title));
            }
        }
    }
}

wrap_load_handler! {
    struct WackLoad {
        tab: TabId,
    }

    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
            if let Some(tab) = tab_of(self.tab) {
                tab.update(|state| state.set_loading(is_loading == 1, can_go_back == 1, can_go_forward == 1));
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            // A navigation we cancelled (or the user stopped) is not a failure to show.
            if *error_code.as_ref() == cef::sys::cef_errorcode_t::ERR_ABORTED {
                return;
            }
            if frame.map(|f| f.is_main() == 1).unwrap_or(false)
                && let Some(tab) = tab_of(self.tab)
            {
                let (url, error) = (text(failed_url), text(error_text));
                tab.update(|state| state.load_error = Some((url, error)));
            }
        }
    }
}

wrap_jsdialog_handler! {
    struct WackDialogs {
        tab: TabId,
    }

    impl JsdialogHandler {
        fn on_jsdialog(
            &self,
            _browser: Option<&mut Browser>,
            _origin_url: Option<&CefString>,
            dialog_type: JsdialogType,
            message_text: Option<&CefString>,
            default_prompt_text: Option<&CefString>,
            callback: Option<&mut JsdialogCallback>,
            _suppress_message: Option<&mut i32>,
        ) -> i32 {
            let (Some(tab), Some(callback)) = (tab_of(self.tab), callback) else { return 0 };
            let kind = if dialog_type == JsdialogType::CONFIRM {
                DialogKind::Confirm
            } else if dialog_type == JsdialogType::PROMPT {
                DialogKind::Prompt
            } else {
                DialogKind::Alert
            };
            with_main(|main| main.dialogs.insert(self.tab, callback.clone()));
            let dialog = Dialog { kind, message: text(message_text), default_prompt: text(default_prompt_text) };
            tab.update(|state| state.dialog = Some(dialog));
            1
        }

        fn on_before_unload_dialog(
            &self,
            _browser: Option<&mut Browser>,
            message_text: Option<&CefString>,
            _is_reload: i32,
            callback: Option<&mut JsdialogCallback>,
        ) -> i32 {
            let (Some(tab), Some(callback)) = (tab_of(self.tab), callback) else { return 0 };
            with_main(|main| main.dialogs.insert(self.tab, callback.clone()));
            let dialog = Dialog { kind: DialogKind::BeforeUnload, message: text(message_text), default_prompt: String::new() };
            tab.update(|state| state.dialog = Some(dialog));
            1
        }

        fn on_reset_dialog_state(&self, _browser: Option<&mut Browser>) {
            with_main(|main| main.dialogs.remove(&self.tab));
            if let Some(tab) = tab_of(self.tab) {
                tab.update(|state| state.dialog = None);
            }
        }
    }
}

wrap_request_handler! {
    struct WackRequests {
        tab: TabId,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let Some(tab) = tab_of(self.tab) else { return 0 };
            if !tab.agent_driving() || !frame.map(|f| f.is_main() == 1).unwrap_or(false) {
                return 0;
            }
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            match policy::check(&url) {
                Ok(Verdict::Allowed(_)) => 0,
                Ok(Verdict::NeedsLookup(_, host)) => {
                    let approved = with_main(|main| main.pages.get(&self.tab).is_some_and(|page| page.approved_hosts.contains(&host)));
                    if approved {
                        return 0;
                    }
                    // Look the host up off the main thread, then go there if every address passes.
                    let id = self.tab;
                    std::thread::spawn(move || {
                        use std::net::ToSocketAddrs;
                        let addresses: Vec<_> = (host.as_str(), 0).to_socket_addrs().map(|a| a.map(|s| s.ip()).collect()).unwrap_or_default();
                        let verdict = policy::judge_addresses(&host, &addresses);
                        main_thread::post(move || match verdict {
                            Ok(()) => {
                                with_main(|main| {
                                    if let Some(page) = main.pages.get_mut(&id) {
                                        page.approved_hosts.insert(host);
                                    }
                                });
                                if let Some(frame) = browser_of(id).and_then(|b| b.main_frame()) {
                                    frame.load_url(Some(&CefString::from(url.as_str())));
                                }
                            }
                            Err(error) => refuse(id, &url, &error),
                        });
                    });
                    1
                }
                Err(error) => {
                    refuse(self.tab, &url, &error);
                    1
                }
            }
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            _status: TerminationStatus,
            _error_code: i32,
            error_string: Option<&CefString>,
        ) {
            if let Some(tab) = tab_of(self.tab) {
                // Chromium's own words, which may be none: the pane says the rest.
                let reason = text(error_string);
                tab.update(|state| state.crashed = Some(reason));
            }
        }
    }
}

/// Records a refused navigation where the agent will read it.
fn refuse(id: TabId, url: &str, error: &str) {
    if let Some(tab) = tab_of(id) {
        tab.update(|state| {
            state.push_console(Level::Error, &format!("Navigation blocked: {error}"), None);
            state.refused = Some((url.to_string(), error.to_string()));
        });
    }
}

// The ⌘ keys. Chromium expects the Edit menu's standard selectors for copy, paste and the rest,
// which GPUI's menus don't send, so they are done here; any other ⌘ key the page leaves alone
// goes to the app's own shortcuts.
const SHIFT: u32 = 2;
const COMMAND: u32 = 128;

wrap_keyboard_handler! {
    struct WackKeys {
        tab: TabId,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: *mut u8,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let (Some(event), Some(browser)) = (event, browser) else { return 0 };
            if event.type_ != KeyEventType::RAWKEYDOWN || event.modifiers & COMMAND == 0 {
                return 0;
            }
            let Some(frame) = browser.focused_frame().or_else(|| browser.main_frame()) else { return 0 };
            let key = char::from_u32(event.unmodified_character as u32).map(|c| c.to_ascii_lowercase());
            let shift = event.modifiers & SHIFT != 0;
            match (key, shift) {
                (Some('a'), false) => frame.select_all(),
                (Some('c'), false) => frame.copy(),
                (Some('v'), false) => frame.paste(),
                (Some('x'), false) => frame.cut(),
                (Some('z'), false) => frame.undo(),
                (Some('z'), true) => frame.redo(),
                _ => return 0,
            }
            1
        }

        fn on_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, os_event: *mut u8) -> i32 {
            let Some(event) = event else { return 0 };
            if event.type_ != KeyEventType::RAWKEYDOWN || event.modifiers & COMMAND == 0 || os_event.is_null() {
                return 0;
            }
            let Some(host) = host_view() else { return 0 };
            // SAFETY: `os_event` is the NSEvent CEF is handling; the host view takes key
            // equivalents (GPUI's key bindings).
            let handled: bool = unsafe { msg_send![&*host, performKeyEquivalent: os_event.cast::<AnyObject>()] };
            i32::from(handled)
        }
    }
}

wrap_download_handler! {
    struct WackDownloads {
        tab: TabId,
    }

    impl DownloadHandler {
        fn on_before_download(
            &self,
            _browser: Option<&mut Browser>,
            _download_item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let Some(tab) = tab_of(self.tab) else { return 0 };
            // The agent does not download files; refusing is the default.
            if tab.agent_driving() {
                tab.update(|state| state.push_console(Level::Warning, "A download was refused: the agent does not download files.", None));
                return 0;
            }
            let (Some(callback), Some(home)) = (callback, std::env::var_os("HOME")) else { return 0 };
            let path = unique_path(&PathBuf::from(home).join("Downloads"), &text(suggested_name));
            callback.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            1
        }
    }
}

/// `dir/name`, or `dir/name (2).ext` and so on when it exists. Names are reduced to their last
/// component, so a page cannot write outside the folder.
fn unique_path(dir: &Path, suggested: &str) -> PathBuf {
    let name = Path::new(suggested)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("download");
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(first)
}

// ---------------------------------------------------------------------------------------------
// The public face: the backend, and what the UI calls on the main thread.

struct CefBackend;

/// The engine for `ChatTabs`. Records the configuration; CEF starts on first use.
pub fn backend(config: EngineConfig) -> Arc<dyn Backend> {
    let _ = CONFIG.set(config);
    Arc::new(CefBackend)
}

impl Backend for CefBackend {
    fn create(&self, tab: Arc<Tab>, url: String) -> BoxFuture<Result<(), String>> {
        Box::pin(async move {
            let (created, done) = oneshot::channel();
            main_thread::post(move || request_page(tab, url, created));
            done.await.map_err(|_| "The browser stopped.".to_string())?
        })
    }

    fn close(&self, id: TabId) {
        main_thread::post(move || close_page(id));
    }

    fn navigate(&self, id: TabId, nav: Nav) -> BoxFuture<Result<(), String>> {
        Box::pin(async move {
            main_thread::call(move || -> Result<(), String> {
                let browser = browser_of(id).ok_or("That tab has no page.")?;
                match nav {
                    Nav::Open(url) => browser
                        .main_frame()
                        .ok_or("That tab has no page.")?
                        .load_url(Some(&CefString::from(url.as_str()))),
                    Nav::Reload => browser.reload(),
                    Nav::Back => browser.go_back(),
                    Nav::Forward => browser.go_forward(),
                    Nav::Stop => browser.stop_load(),
                }
                Ok(())
            })
            .await?
        })
    }

    fn cdp(&self, id: TabId, method: String, params: Json) -> BoxFuture<Result<Json, String>> {
        Box::pin(async move {
            let (answer, result) = oneshot::channel();
            main_thread::call(move || send(id, &method, params, Some(answer))).await??;
            result
                .await
                .map_err(|_| "The page closed before it answered.".to_string())?
        })
    }

    fn answer_dialog(
        &self,
        id: TabId,
        accept: bool,
        reply: Option<String>,
    ) -> BoxFuture<Result<(), String>> {
        Box::pin(async move {
            main_thread::call(move || -> Result<(), String> {
                let callback = with_main(|main| main.dialogs.remove(&id))
                    .ok_or("The page has no dialog open.")?;
                callback.cont(
                    i32::from(accept),
                    reply.as_deref().map(CefString::from).as_ref(),
                );
                if let Some(tab) = tab_of(id) {
                    tab.update(|state| state.dialog = None);
                }
                Ok(())
            })
            .await?
        })
    }
}

/// The UI's view to host pages in, and the overlay view they must stay beneath (null for none).
/// Main thread only. Pages asked for before this wait for it.
pub fn set_host(view: *mut c_void, overlay: *mut c_void) {
    debug_assert!(main_thread::is_main());
    with_main(|main| main.host = Some((view as usize, overlay as usize)));
    let ids: Vec<TabId> = with_main(|main| main.pages.keys().copied().collect());
    for id in ids {
        restack(id);
    }
    flush_waiting();
}

/// Shows a tab's page at `frame`, or parks it (`None`). Main thread only; cheap when nothing
/// changed, so the UI may call it every frame.
pub fn place(id: TabId, frame: Option<Frame2>) {
    debug_assert!(main_thread::is_main());
    let unchanged =
        with_main(|main| main.pages.get(&id).map(|page| page.shown == frame)).unwrap_or(true);
    if !unchanged {
        position(id, frame);
    }
}

/// Gives a shown page keyboard focus, as a click on it would. Main thread only.
pub fn focus(id: TabId) {
    if let Some(host) = browser_of(id).and_then(|b| b.host()) {
        host.set_focus(1);
    }
}

/// Closes every page and stops CEF. Main thread only; call once, when the app quits.
pub fn shutdown() {
    if !RUNNING.load(Ordering::Acquire) {
        return;
    }
    let ids: Vec<TabId> = with_main(|main| main.pages.keys().copied().collect());
    for id in ids {
        close_page(id);
    }
    // Pages close asynchronously: pump until they have, for at most two seconds.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while with_main(|main| !main.pages.is_empty()) && std::time::Instant::now() < deadline {
        cef::do_message_loop_work();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    RUNNING.store(false, Ordering::Release);
    cef::shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_get_a_free_name_inside_the_folder() {
        let dir =
            std::env::temp_dir().join(format!("wack-browser-downloads-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report.pdf"));
        std::fs::write(dir.join("report.pdf"), "").unwrap();
        std::fs::write(dir.join("report (2).pdf"), "").unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report (3).pdf"));
        assert_eq!(unique_path(&dir, "../../etc/passwd"), dir.join("passwd"));
        assert_eq!(unique_path(&dir, ""), dir.join("download"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_features_join_cefs_own() {
        assert_eq!(with_features("", &["AimEnabled"]), "AimEnabled");
        assert_eq!(with_features("LensOverlay,GlicActorUi", &["AimEnabled"]), "LensOverlay,GlicActorUi,AimEnabled");
        assert_eq!(with_features("AimEnabled", &["AimEnabled"]), "AimEnabled");
    }

    #[test]
    fn blocked_hosts_are_only_chromium_services() {
        for host in BLOCKED_HOSTS {
            assert!(host.contains("google") || host.contains("gvt1"), "{host}");
        }
    }
}
