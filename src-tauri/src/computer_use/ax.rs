//! Accessibility (AX) reads and actions. Everything here runs on the engine thread: AX handles
//! are not `Send`, and one thread serializes every conversation with a target app. Each call is
//! bounded by the messaging timeout set on the element, so a hung app (say, one paused in a
//! debugger) costs at most about a second per call.
//!
//! Secure text fields are never read: their value attribute is not even requested.

use super::geometry::Rect;
use super::outline::Node;
use objc2_application_services::{AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFBoolean, CFNumber, CFRetained, CFString, CFType, CGPoint, CGSize};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub type Element = CFRetained<AXUIElement>;

/// Seconds an app may take to answer one AX message.
const MESSAGING_TIMEOUT: f32 = 1.0;
const MAX_DEPTH: usize = 40;
const MAX_CHILDREN: usize = 60;

pub struct WalkLimits {
    pub max_nodes: usize,
    pub budget: Duration,
}

pub const SNAPSHOT_LIMITS: WalkLimits = WalkLimits { max_nodes: 700, budget: Duration::from_millis(3_000) };
pub const SEARCH_LIMITS: WalkLimits = WalkLimits { max_nodes: 1_000, budget: Duration::from_millis(1_000) };

fn cf(name: &str) -> CFRetained<CFString> {
    CFString::from_str(name)
}

/// The attribute's value; `Ok(None)` when the element simply has none.
pub fn attribute(element: &AXUIElement, name: &str) -> Result<Option<CFRetained<CFType>>, AXError> {
    let mut value: *const CFType = std::ptr::null();
    let error = unsafe { element.copy_attribute_value(&cf(name), NonNull::from(&mut value)) };
    if error == AXError::Success {
        Ok(NonNull::new(value as *mut CFType).map(|value| unsafe { CFRetained::from_raw(value) }))
    } else if error == AXError::NoValue || error == AXError::AttributeUnsupported {
        Ok(None)
    } else {
        Err(error)
    }
}

pub fn string(element: &AXUIElement, name: &str) -> Option<String> {
    let value = attribute(element, name).ok()??;
    value.downcast_ref::<CFString>().map(|text| text.to_string())
}

pub fn boolean(element: &AXUIElement, name: &str) -> Option<bool> {
    let value = attribute(element, name).ok()??;
    if let Some(flag) = value.downcast_ref::<CFBoolean>() {
        return Some(flag.as_bool());
    }
    value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64).map(|number| number != 0)
}

pub fn number(element: &AXUIElement, name: &str) -> Option<f64> {
    let value = attribute(element, name).ok()??;
    value.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64)
}

/// An element's value as text: strings as they are, numbers and booleans formatted.
fn value_text(element: &AXUIElement) -> Option<String> {
    let value = attribute(element, "AXValue").ok()??;
    if let Some(text) = value.downcast_ref::<CFString>() {
        return Some(text.to_string());
    }
    if let Some(flag) = value.downcast_ref::<CFBoolean>() {
        return Some(if flag.as_bool() { "1".into() } else { "0".into() });
    }
    let number = value.downcast_ref::<CFNumber>()?;
    match number.as_i64() {
        Some(integer) if number.as_f64().is_some_and(|float| float.fract() == 0.0) => Some(integer.to_string()),
        _ => number.as_f64().map(|float| format!("{float:.3}").trim_end_matches('0').trim_end_matches('.').to_string()),
    }
}

pub fn element(element: &AXUIElement, name: &str) -> Option<Element> {
    attribute(element, name).ok()??.downcast::<AXUIElement>().ok()
}

/// Up to `limit` elements of an array attribute, plus how many there were in total.
pub fn elements(element: &AXUIElement, name: &str, limit: usize) -> (Vec<Element>, usize) {
    let Some(value) = attribute(element, name).ok().flatten() else { return (Vec::new(), 0) };
    let Some(array) = value.downcast_ref::<CFArray>() else { return (Vec::new(), 0) };
    // SAFETY: an AX array attribute holds CF objects; each is checked before use.
    let array: &CFArray<CFType> = unsafe { array.cast_unchecked() };
    let total = array.len();
    let items = (0..total.min(limit))
        .filter_map(|index| array.get(index))
        .filter_map(|item| item.downcast::<AXUIElement>().ok())
        .collect();
    (items, total)
}

fn ax_value<T: Default>(element: &AXUIElement, name: &str, kind: AXValueType) -> Option<T> {
    let value = attribute(element, name).ok()??;
    let value = value.downcast_ref::<AXValue>()?;
    let mut out = T::default();
    let ok = unsafe { value.value(kind, NonNull::from(&mut out).cast::<c_void>()) };
    ok.then_some(out)
}

/// The element's frame in global screen points (top-left origin).
pub fn frame(element: &AXUIElement) -> Option<Rect> {
    let origin: CGPoint = ax_value(element, "AXPosition", AXValueType::CGPoint)?;
    let size: CGSize = ax_value(element, "AXSize", AXValueType::CGSize)?;
    Some(Rect { x: origin.x, y: origin.y, width: size.width, height: size.height })
}

pub fn pid(element: &AXUIElement) -> Option<i32> {
    let mut pid: libc::pid_t = 0;
    let error = unsafe { element.pid(NonNull::from(&mut pid)) };
    (error == AXError::Success).then_some(pid)
}

/// Whether the element still exists (a dead one reports `kAXErrorInvalidUIElement`).
pub fn is_alive(element: &AXUIElement) -> bool {
    !matches!(attribute(element, "AXRole"), Err(error) if error == AXError::InvalidUIElement)
}

pub fn perform(element: &AXUIElement, action: &str) -> Result<(), AXError> {
    let error = unsafe { element.perform_action(&cf(action)) };
    if error == AXError::Success { Ok(()) } else { Err(error) }
}

fn set(element: &AXUIElement, name: &str, value: &CFType) -> Result<(), AXError> {
    let error = unsafe { element.set_attribute_value(&cf(name), value) };
    if error == AXError::Success { Ok(()) } else { Err(error) }
}

pub fn set_bool(element: &AXUIElement, name: &str, value: bool) -> Result<(), AXError> {
    set(element, name, CFBoolean::new(value).as_ref())
}

pub fn set_string(element: &AXUIElement, name: &str, value: &str) -> Result<(), AXError> {
    set(element, name, cf(value).as_ref())
}

pub fn set_number(element: &AXUIElement, name: &str, value: f64) -> Result<(), AXError> {
    set(element, name, CFNumber::new_f64(value).as_ref())
}

/// A sentence for an AX failure, naming the app when it is unresponsive.
pub fn describe_error(error: AXError, what: &str) -> String {
    match error {
        error if error == AXError::CannotComplete => format!("The app isn't responding to Accessibility, so {what} failed. It may be busy or paused in a debugger."),
        error if error == AXError::InvalidUIElement => format!("That element no longer exists, so {what} failed. Take a new snapshot."),
        error if error == AXError::ActionUnsupported || error == AXError::AttributeUnsupported => format!("That element doesn't support {what}."),
        error if error == AXError::APIDisabled => "Accessibility access is off for WackCode. Turn it on in System Settings › Privacy & Security › Accessibility.".into(),
        error => format!("{what} failed (Accessibility error {}).", error.0),
    }
}

pub fn application(pid: i32) -> Element {
    let app = unsafe { AXUIElement::new_application(pid) };
    unsafe { app.set_messaging_timeout(MESSAGING_TIMEOUT) };
    app
}

pub fn system_wide() -> Element {
    let system = unsafe { AXUIElement::new_system_wide() };
    unsafe { system.set_messaging_timeout(MESSAGING_TIMEOUT) };
    system
}

/// Electron and Chromium build their web contents' tree only when asked. Harmless elsewhere.
pub fn enable_manual_accessibility(app: &AXUIElement) {
    let _ = set_bool(app, "AXManualAccessibility", true);
}

type GetWindow = unsafe extern "C-unwind" fn(*const AXUIElement, *mut u32) -> i32;

/// `_AXUIElementGetWindow` maps an AX window to its window-server number. It is private but
/// long-lived; it is looked up at runtime so its absence only disables the direct mapping.
fn get_window_fn() -> Option<GetWindow> {
    static FUNCTION: OnceLock<Option<usize>> = OnceLock::new();
    let address = *FUNCTION.get_or_init(|| {
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"_AXUIElementGetWindow".as_ptr()) };
        (!symbol.is_null()).then_some(symbol as usize)
    });
    // SAFETY: the symbol has had this signature since macOS 10.x.
    address.map(|address| unsafe { std::mem::transmute::<usize, GetWindow>(address) })
}

pub fn window_number(window: &AXUIElement) -> Option<u32> {
    let function = get_window_fn()?;
    let mut number = 0u32;
    let error = unsafe { function(window as *const AXUIElement, &mut number) };
    (error == 0 && number != 0).then_some(number)
}

pub struct AxWindow {
    pub element: Element,
    pub title: String,
    pub frame: Option<Rect>,
    pub main: bool,
    pub minimized: bool,
}

/// The app's standard windows, main first.
pub fn windows(app: &AXUIElement) -> Vec<AxWindow> {
    let (list, _) = elements(app, "AXWindows", 50);
    let mut windows = list
        .into_iter()
        .map(|element| AxWindow {
            title: string(&element, "AXTitle").unwrap_or_default(),
            frame: frame(&element),
            main: boolean(&element, "AXMain").unwrap_or(false),
            minimized: boolean(&element, "AXMinimized").unwrap_or(false),
            element,
        })
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| (!window.main, window.minimized));
    windows
}

struct Walker {
    elements: Vec<Element>,
    deadline: Instant,
    max_nodes: usize,
}

impl Walker {
    fn exhausted(&self) -> bool {
        self.elements.len() >= self.max_nodes || Instant::now() >= self.deadline
    }

    fn visit(&mut self, element: Element, depth: usize) -> Node {
        let handle = self.elements.len();
        let role = string(&element, "AXRole").unwrap_or_else(|| "AXUnknown".into());
        let subrole = string(&element, "AXSubrole");
        let secure = subrole.as_deref() == Some("AXSecureTextField");
        let mut node = Node {
            handle,
            title: string(&element, "AXTitle"),
            description: string(&element, "AXDescription"),
            placeholder: string(&element, "AXPlaceholderValue"),
            // Never requested for a secure field.
            value: if secure { None } else { value_text(&element) },
            enabled: boolean(&element, "AXEnabled"),
            focused: boolean(&element, "AXFocused").unwrap_or(false),
            selected: boolean(&element, "AXSelected").unwrap_or(false),
            role,
            subrole,
            ..Node::default()
        };
        self.elements.push(element.clone());
        if depth < MAX_DEPTH && !self.exhausted() {
            let (children, total) = elements(&element, "AXChildren", MAX_CHILDREN);
            let mut visited = 0;
            for child in children {
                if self.exhausted() {
                    break;
                }
                node.children.push(self.visit(child, depth + 1));
                visited += 1;
            }
            node.omitted_children = total - visited;
        }
        node
    }
}

/// Walks the tree under `root` within `limits`. The returned elements are indexed by each
/// node's `handle`.
pub fn walk(root: &Element, limits: &WalkLimits) -> (Node, Vec<Element>) {
    let mut walker = Walker { elements: Vec::new(), deadline: Instant::now() + limits.budget, max_nodes: limits.max_nodes };
    let node = walker.visit(root.clone(), 0);
    (node, walker.elements)
}

/// A short description of an element for results: `textField "Name"`.
pub fn describe(element: &AXUIElement) -> String {
    let role = string(element, "AXRole").unwrap_or_else(|| "AXUnknown".into());
    let name = role.strip_prefix("AX").unwrap_or(&role);
    let mut chars = name.chars();
    let mut text = match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect::<String>(),
        None => "element".to_string(),
    };
    if let Some(label) = string(element, "AXTitle").or_else(|| string(element, "AXDescription")).filter(|label| !label.trim().is_empty()) {
        let label: String = label.chars().take(80).collect();
        text.push_str(&format!(" \"{label}\""));
    }
    text
}

pub fn is_secure(element: &AXUIElement) -> bool {
    string(element, "AXSubrole").as_deref() == Some("AXSecureTextField")
}

/// Whether the app's focused element is a secure text field (a password is being typed).
pub fn focused_is_secure(app: &AXUIElement) -> bool {
    element(app, "AXFocusedUIElement").is_some_and(|focused| is_secure(&focused))
}

fn normalize_title(title: &str) -> String {
    title.trim().replace("...", "…").to_lowercase()
}

/// The app's menu-bar titles (File, Edit, …), for a window-less app's snapshot.
pub fn menu_titles(app: &AXUIElement) -> Vec<String> {
    let Some(bar) = element(app, "AXMenuBar") else { return Vec::new() };
    elements(&bar, "AXChildren", 40).0.iter().filter_map(|item| string(item, "AXTitle")).filter(|title| !title.trim().is_empty()).collect()
}

/// Presses the menu-bar item at `path` (e.g. File › Save As…) without opening the menus.
pub fn press_menu(app: &AXUIElement, path: &[String], before_press: &dyn Fn(&Element)) -> Result<(), String> {
    let mut container = element(app, "AXMenuBar").ok_or("This app has no menu bar that Accessibility can reach.")?;
    for (depth, wanted) in path.iter().enumerate() {
        let wanted_normalized = normalize_title(wanted);
        let (items, _) = elements(&container, "AXChildren", 200);
        // Menu bar items and menu items live directly under the bar or a menu.
        let found = items.into_iter().find(|item| string(item, "AXTitle").is_some_and(|title| normalize_title(&title) == wanted_normalized));
        let item = found.ok_or_else(|| {
            let (siblings, _) = elements(&container, "AXChildren", 60);
            let names = siblings.iter().filter_map(|item| string(item, "AXTitle")).filter(|title| !title.is_empty()).collect::<Vec<_>>();
            format!("There is no menu item “{wanted}” there. Available: {}.", names.join(", "))
        })?;
        if depth + 1 == path.len() {
            if boolean(&item, "AXEnabled") == Some(false) {
                return Err(format!("The menu item “{wanted}” is disabled."));
            }
            before_press(&item);
            return perform(&item, "AXPress").map_err(|error| describe_error(error, "choosing the menu item"));
        }
        // A menu bar item or submenu item holds its menu as its only child.
        let (menus, _) = elements(&item, "AXChildren", 1);
        container = menus.into_iter().next().ok_or_else(|| format!("“{wanted}” has no submenu."))?;
    }
    Err("menu needs a path.".into())
}

/// Scrolls the scroll area containing `element` by moving its scroll bars. `false` when there
/// is no scroll bar Accessibility can move, so the caller can fall back to a wheel event.
pub fn scroll(element: &Element, dx: f64, dy: f64) -> Result<bool, String> {
    let mut current = Some(element.clone());
    for _ in 0..12 {
        let Some(candidate) = current else { break };
        if string(&candidate, "AXRole").as_deref() == Some("AXScrollArea") {
            let mut moved = false;
            for (bar, delta) in [("AXVerticalScrollBar", dy), ("AXHorizontalScrollBar", dx)] {
                if delta == 0.0 {
                    continue;
                }
                let Some(bar) = self::element(&candidate, bar) else { continue };
                let Some(position) = number(&bar, "AXValue") else { continue };
                // One step is a tenth of the scroll range.
                let next = (position + delta * 0.1).clamp(0.0, 1.0);
                if set_number(&bar, "AXValue", next).is_ok() {
                    moved = true;
                }
            }
            return Ok(moved);
        }
        current = self::element(&candidate, "AXParent");
    }
    Ok(false)
}

pub struct Criteria<'a> {
    pub role: Option<&'a str>,
    pub title_contains: Option<&'a str>,
    pub value_equals: Option<&'a str>,
}

fn role_matches(role: &str, wanted: &str) -> bool {
    let wanted = wanted.strip_prefix("AX").unwrap_or(wanted);
    role.strip_prefix("AX").unwrap_or(role).eq_ignore_ascii_case(wanted)
}

fn node_matches(node: &Node, criteria: &Criteria<'_>) -> bool {
    if criteria.role.is_some_and(|role| !role_matches(&node.role, role)) {
        return false;
    }
    if let Some(needle) = criteria.title_contains.map(str::to_lowercase) {
        let haystacks = [&node.title, &node.description, &node.value];
        if !haystacks.iter().any(|text| text.as_deref().is_some_and(|text| text.to_lowercase().contains(&needle))) {
            return false;
        }
    }
    if criteria.value_equals.is_some_and(|wanted| node.value.as_deref() != Some(wanted)) {
        return false;
    }
    true
}

fn first_match<'a>(node: &'a Node, criteria: &Criteria<'_>) -> Option<&'a Node> {
    if node_matches(node, criteria) {
        return Some(node);
    }
    node.children.iter().find_map(|child| first_match(child, criteria))
}

/// A short description of the first element under `root` matching `criteria`.
pub fn find(root: &Element, criteria: &Criteria<'_>) -> Option<String> {
    if criteria.role.is_none() && criteria.title_contains.is_none() && criteria.value_equals.is_none() {
        return Some(String::new());
    }
    let (tree, elements) = walk(root, &SEARCH_LIMITS);
    first_match(&tree, criteria).map(|node| describe(&elements[node.handle]))
}
