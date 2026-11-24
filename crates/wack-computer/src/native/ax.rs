//! Accessibility (AX) reads and writes. Everything here runs on the app's engine thread
//! (`engine.rs`): AX handles are not `Send`. Each call is bounded by the messaging timeout set on
//! the app element, so a hung app (say, one paused in a debugger) costs about a second per call
//! and stalls only its own thread. Ported from version 2's `wackdev/src/ax.rs`.
//!
//! Invariants:
//! - A walk reads each element's attributes in **one** message
//!   (`AXUIElementCopyMultipleAttributeValues`); version 2 sent about nine.
//! - Secure text fields are never read: `AXValue` is asked for separately, only for roles that
//!   carry a value, and never for a secure field (the batch reads its subrole first).
//! - `AXManualAccessibility` is set only on Electron and Chromium apps (`wants_manual`): on a
//!   WKWebView app it removes the app's own windows from `AXWindows` until it restarts, which
//!   version 2 did to every app.

use std::{
    collections::HashMap,
    ffi::c_void,
    path::Path,
    ptr::NonNull,
    sync::OnceLock,
    time::{Duration, Instant},
};

use objc2_application_services::{AXCopyMultipleAttributeOptions, AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFBoolean, CFNumber, CFRetained, CFString, CFType, CGPoint, CGSize};

use crate::{geometry::Rect, outline::Node};

pub type Element = CFRetained<AXUIElement>;

/// Seconds an app may take to answer one AX message.
const MESSAGING_TIMEOUT: f32 = 1.0;
const MAX_DEPTH: usize = 40;
const MAX_CHILDREN: usize = 80;

pub struct WalkLimits {
    pub max_nodes: usize,
    pub budget: Duration,
}

pub const SNAPSHOT_LIMITS: WalkLimits = WalkLimits { max_nodes: 1_500, budget: Duration::from_millis(3_000) };
pub const SEARCH_LIMITS: WalkLimits = WalkLimits { max_nodes: 2_000, budget: Duration::from_millis(1_000) };

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

fn as_string(value: &CFType) -> Option<String> {
    value.downcast_ref::<CFString>().map(|text| text.to_string())
}

fn as_bool(value: &CFType) -> Option<bool> {
    if let Some(flag) = value.downcast_ref::<CFBoolean>() {
        return Some(flag.as_bool());
    }
    value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64).map(|number| number != 0)
}

pub fn string(element: &AXUIElement, name: &str) -> Option<String> {
    let value = attribute(element, name).ok()??;
    as_string(&value)
}

pub fn boolean(element: &AXUIElement, name: &str) -> Option<bool> {
    let value = attribute(element, name).ok()??;
    as_bool(&value)
}

pub fn number(element: &AXUIElement, name: &str) -> Option<f64> {
    attribute(element, name).ok()??.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64)
}

/// A value as text: strings as they are, numbers and booleans formatted.
fn format_value(value: &CFType) -> Option<String> {
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

/// The element's value as text. Never call it on a secure field (`is_secure`).
pub fn value_text(element: &AXUIElement) -> Option<String> {
    let value = attribute(element, "AXValue").ok()??;
    format_value(&value)
}

pub fn element(element: &AXUIElement, name: &str) -> Option<Element> {
    attribute(element, name).ok()??.downcast::<AXUIElement>().ok()
}

fn as_elements(value: &CFType, limit: usize) -> (Vec<Element>, usize) {
    let Some(array) = value.downcast_ref::<CFArray>() else { return (Vec::new(), 0) };
    // SAFETY: an AX array attribute holds CF objects; each is checked before use.
    let array: &CFArray<CFType> = unsafe { array.cast_unchecked() };
    let total = array.len();
    let items = (0..total.min(limit)).filter_map(|index| array.get(index)).filter_map(|item| item.downcast::<AXUIElement>().ok()).collect();
    (items, total)
}

/// Up to `limit` elements of an array attribute, plus how many there were in total.
pub fn elements(element: &AXUIElement, name: &str, limit: usize) -> (Vec<Element>, usize) {
    match attribute(element, name).ok().flatten() {
        Some(value) => as_elements(&value, limit),
        None => (Vec::new(), 0),
    }
}

fn as_ax_value<T>(value: &CFType, kind: AXValueType) -> Option<T> {
    let value = value.downcast_ref::<AXValue>()?;
    if unsafe { value.r#type() } != kind {
        return None;
    }
    let mut out = std::mem::MaybeUninit::<T>::uninit();
    // SAFETY: the value's type was checked to be `kind`, whose C type is `T`.
    let ok = unsafe { value.value(kind, NonNull::new_unchecked(out.as_mut_ptr()).cast::<c_void>()) };
    ok.then(|| unsafe { out.assume_init() })
}

/// The element's frame in global screen points (top-left origin).
pub fn frame(element: &AXUIElement) -> Option<Rect> {
    let (position, size) = (attribute(element, "AXPosition").ok()??, attribute(element, "AXSize").ok()??);
    let origin: CGPoint = as_ax_value(&position, AXValueType::CGPoint)?;
    let size: CGSize = as_ax_value(&size, AXValueType::CGSize)?;
    Some(Rect::new(origin.x, origin.y, size.width, size.height))
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

pub fn actions(element: &AXUIElement) -> Vec<String> {
    let mut names: *const CFArray = std::ptr::null();
    let error = unsafe { element.copy_action_names(NonNull::from(&mut names)) };
    if error != AXError::Success || names.is_null() {
        return Vec::new();
    }
    let names = unsafe { CFRetained::from_raw(NonNull::new_unchecked(names as *mut CFArray)) };
    let names: &CFArray<CFType> = unsafe { names.cast_unchecked() };
    names.iter().filter_map(|name| as_string(&name)).collect()
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

/// Whether the attribute can be written.
pub fn settable(element: &AXUIElement, name: &str) -> bool {
    let mut settable: u8 = 0;
    let error = unsafe { element.is_attribute_settable(&cf(name), NonNull::from(&mut settable).cast()) };
    error == AXError::Success && settable != 0
}

/// A sentence for an AX failure.
pub fn describe_error(error: AXError, what: &str) -> String {
    match error {
        error if error == AXError::CannotComplete => format!("The app isn't responding to Accessibility, so {what} failed. It may be busy or paused in a debugger."),
        error if error == AXError::InvalidUIElement => format!("That element no longer exists, so {what} failed. Take a new snapshot."),
        error if error == AXError::ActionUnsupported || error == AXError::AttributeUnsupported => format!("That element doesn't support {what}."),
        error if error == AXError::APIDisabled => "Accessibility access is off for WackCode. Turn it on in Settings › Computer use.".into(),
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

/// The element at a global screen point, as the system-wide hit test finds it.
pub fn element_at(x: f64, y: f64) -> Option<Element> {
    let system = system_wide();
    let mut found: *const AXUIElement = std::ptr::null();
    let error = unsafe { system.copy_element_at_position(x as f32, y as f32, NonNull::from(&mut found)) };
    if error != AXError::Success || found.is_null() {
        return None;
    }
    Some(unsafe { CFRetained::from_raw(NonNull::new_unchecked(found as *mut AXUIElement)) })
}

/// Whether an app builds its web contents' tree only when asked: Electron and Chromium-based
/// apps, which carry their framework in the bundle.
pub fn wants_manual(bundle: &Path) -> bool {
    let frameworks = bundle.join("Contents/Frameworks");
    ["Electron Framework.framework", "Chromium Embedded Framework.framework"].iter().any(|name| frameworks.join(name).exists())
        || std::fs::read_dir(&frameworks).is_ok_and(|entries| {
            entries.flatten().any(|entry| entry.file_name().to_string_lossy().ends_with(" Framework.framework") && entry.path().join("Versions/Current/Helpers").exists())
        })
}

/// Asks an Electron or Chromium app to build its accessibility tree. Never on any other app
/// (see the header).
pub fn enable_manual_accessibility(app: &AXUIElement) {
    let _ = set_bool(app, "AXManualAccessibility", true);
}

type GetWindow = unsafe extern "C-unwind" fn(*const AXUIElement, *mut u32) -> i32;

/// `_AXUIElementGetWindow` maps an AX window to its window-server number. It is private but
/// long-lived; it is looked up at runtime so its absence only disables the direct mapping (the
/// caller falls back to matching frames).
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
    pub subrole: Option<String>,
}

/// The app's windows, main first.
pub fn windows(app: &AXUIElement) -> Vec<AxWindow> {
    let (list, _) = elements(app, "AXWindows", 50);
    let mut windows = list
        .into_iter()
        .map(|element| AxWindow {
            title: string(&element, "AXTitle").unwrap_or_default(),
            frame: frame(&element),
            main: boolean(&element, "AXMain").unwrap_or(false),
            minimized: boolean(&element, "AXMinimized").unwrap_or(false),
            subrole: string(&element, "AXSubrole"),
            element,
        })
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| (!window.main, window.minimized));
    windows
}

/// The attributes a walk reads in one message, in this order.
const WALK: [&str; 11] = ["AXRole", "AXSubrole", "AXTitle", "AXDescription", "AXPlaceholderValue", "AXEnabled", "AXFocused", "AXSelected", "AXPosition", "AXSize", "AXChildren"];

/// Roles whose `AXValue` the outline shows. Anything else is never asked for its value.
const VALUE_ROLES: &[&str] = &[
    "AXStaticText", "AXTextField", "AXTextArea", "AXComboBox", "AXCheckBox", "AXRadioButton", "AXSlider", "AXIncrementor", "AXValueIndicator",
    "AXPopUpButton", "AXProgressIndicator", "AXLevelIndicator", "AXDisclosureTriangle", "AXHeading", "AXCell", "AXMenuButton", "AXDateField",
    "AXColorWell", "AXSearchField",
];

fn walk_names() -> &'static CFRetained<CFArray<CFString>> {
    struct Names(CFRetained<CFArray<CFString>>);
    // SAFETY: an immutable CFArray of CFStrings, built once and never changed.
    unsafe impl Send for Names {}
    unsafe impl Sync for Names {}
    static NAMES: OnceLock<Names> = OnceLock::new();
    &NAMES
        .get_or_init(|| {
            let names: Vec<CFRetained<CFString>> = WALK.iter().map(|name| cf(name)).collect();
            let refs: Vec<&CFString> = names.iter().map(|name| &**name).collect();
            Names(CFArray::from_objects(&refs))
        })
        .0
}

/// One element's walk attributes in one message: each `Some` or, for an attribute it lacks or
/// an error, `None`.
fn read_all(element: &AXUIElement) -> Option<Vec<Option<CFRetained<CFType>>>> {
    let names = walk_names();
    let mut values: *const CFArray = std::ptr::null();
    let error = unsafe { element.copy_multiple_attribute_values(names.as_opaque(), AXCopyMultipleAttributeOptions(0), NonNull::from(&mut values)) };
    if error != AXError::Success || values.is_null() {
        return None;
    }
    let values = unsafe { CFRetained::from_raw(NonNull::new_unchecked(values as *mut CFArray)) };
    let values: &CFArray<CFType> = unsafe { values.cast_unchecked() };
    Some(
        (0..WALK.len())
            .map(|index| {
                let value = values.get(index)?;
                // A missing attribute comes back as an AXValue holding its error.
                let is_error = value.downcast_ref::<AXValue>().is_some_and(|value| unsafe { value.r#type() } == AXValueType::AXError);
                (!is_error).then_some(value)
            })
            .collect(),
    )
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
        self.elements.push(element.clone());
        let values = read_all(&element).unwrap_or_else(|| vec![None; WALK.len()]);
        let text = |index: usize| values[index].as_deref().and_then(as_string);
        let flag = |index: usize| values[index].as_deref().and_then(as_bool);
        let role = text(0).unwrap_or_else(|| "AXUnknown".into());
        let subrole = text(1);
        let secure = subrole.as_deref() == Some("AXSecureTextField");
        let origin: Option<CGPoint> = values[8].as_deref().and_then(|value| as_ax_value(value, AXValueType::CGPoint));
        let size: Option<CGSize> = values[9].as_deref().and_then(|value| as_ax_value(value, AXValueType::CGSize));
        let mut node = Node {
            handle,
            title: text(2),
            description: text(3),
            placeholder: text(4),
            enabled: flag(5),
            focused: flag(6).unwrap_or(false),
            selected: flag(7).unwrap_or(false),
            frame: origin.zip(size).map(|(origin, size)| Rect::new(origin.x, origin.y, size.width, size.height)),
            // Asked for only on roles that show one, and never on a secure field.
            value: if secure || !VALUE_ROLES.contains(&role.as_str()) { None } else { value_text(&element) },
            role,
            subrole,
            ..Node::default()
        };
        // A scroll bar's parts (its thumb, its arrows) are noise: the scroll area is the target.
        let leaf = node.role == "AXScrollBar";
        if depth < MAX_DEPTH && !leaf && !self.exhausted() {
            let (children, total) = values[10].as_deref().map(|value| as_elements(value, MAX_CHILDREN)).unwrap_or_default();
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

/// Walks the tree under `root` within `limits`. The returned elements are indexed by each node's
/// `handle`. `truncated` when the limits cut it short.
pub fn walk(root: &Element, limits: &WalkLimits) -> (Node, Vec<Element>, bool) {
    let mut walker = Walker { elements: Vec::new(), deadline: Instant::now() + limits.budget, max_nodes: limits.max_nodes };
    let node = walker.visit(root.clone(), 0);
    let truncated = walker.exhausted();
    (node, walker.elements, truncated)
}

pub fn is_secure(element: &AXUIElement) -> bool {
    string(element, "AXSubrole").as_deref() == Some("AXSecureTextField")
}

/// The app's focused element.
pub fn focused(app: &AXUIElement) -> Option<Element> {
    element(app, "AXFocusedUIElement")
}

/// Whether the app's focused element is a secure text field (a password is being typed).
pub fn focused_is_secure(app: &AXUIElement) -> bool {
    focused(app).is_some_and(|focused| is_secure(&focused))
}

/// Menu titles compare without case or a trailing ellipsis: an app may show "Save" in the
/// background and "Save…" in front.
fn normalize_title(title: &str) -> String {
    title.trim().trim_end_matches("...").trim_end_matches('…').trim().to_lowercase()
}

/// The titles under the menu bar (`path` empty) or under the menu at `path`, with `›` marking
/// items that open a submenu and `(disabled)` the ones that can't be chosen.
pub fn menu_list(app: &AXUIElement, path: &[String]) -> Result<Vec<String>, String> {
    let mut container = element(app, "AXMenuBar").ok_or("This app has no menu bar that Accessibility can reach.")?;
    for wanted in path {
        let item = find_menu_item(&container, wanted)?;
        container = elements(&item, "AXChildren", 1).0.into_iter().next().ok_or_else(|| format!("“{wanted}” has no submenu."))?;
    }
    let (items, _) = elements(&container, "AXChildren", 200);
    Ok(items
        .iter()
        .filter_map(|item| {
            let title = string(item, "AXTitle").filter(|title| !title.trim().is_empty())?;
            let submenu = !path.is_empty() && elements(item, "AXChildren", 1).1 > 0;
            let disabled = boolean(item, "AXEnabled") == Some(false);
            Some(format!("{title}{}{}", if submenu { " ›" } else { "" }, if disabled { " (disabled)" } else { "" }))
        })
        .collect())
}

fn find_menu_item(container: &Element, wanted: &str) -> Result<Element, String> {
    let wanted_normalized = normalize_title(wanted);
    let (items, _) = elements(container, "AXChildren", 200);
    items.into_iter().find(|item| string(item, "AXTitle").is_some_and(|title| normalize_title(&title) == wanted_normalized)).ok_or_else(|| {
        let (siblings, _) = elements(container, "AXChildren", 60);
        let names = siblings.iter().filter_map(|item| string(item, "AXTitle")).filter(|title| !title.trim().is_empty()).collect::<Vec<_>>();
        format!("There is no menu item “{wanted}” there. Available: {}.", names.join(", "))
    })
}

/// Scrolls the scroll area containing `element` by moving its scroll bars, a tenth of the range
/// per step. `false` when there is no scroll bar Accessibility can move.
pub fn scroll(element: &Element, dx: f64, dy: f64) -> bool {
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
                let next = (position + delta * 0.1).clamp(0.0, 1.0);
                if set_number(&bar, "AXValue", next).is_ok() {
                    moved = true;
                }
            }
            return moved;
        }
        current = self::element(&candidate, "AXParent");
    }
    false
}

/// The element's window.
pub fn window_of(element: &AXUIElement) -> Option<Element> {
    self::element(element, "AXWindow")
}

/// Identity for the engine's ref registry: AX elements compare and hash by the UI element they
/// name (`CFEqual`, `CFHash`), so the same button found by two walks is one key.
#[derive(Clone)]
pub struct Key(pub Element);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        let (a, b): (&CFType, &CFType) = (&self.0, &other.0);
        a == b
    }
}

impl Eq for Key {}

impl std::hash::Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let element: &CFType = &self.0;
        element.hash(state);
    }
}

/// One app's ref numbers: an element keeps its number for as long as it exists. Lives on the
/// app's engine thread; `next` is shared so numbers never repeat after the thread ends.
pub struct Registry {
    by_element: HashMap<Key, u32>,
    by_ref: HashMap<u32, Element>,
    next: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

/// Past this many refs, dead elements are dropped.
const REGISTRY_PRUNE_AT: usize = 8_000;

impl Registry {
    pub fn new(next: std::sync::Arc<std::sync::atomic::AtomicU32>) -> Self {
        Self { by_element: HashMap::new(), by_ref: HashMap::new(), next }
    }

    pub fn ref_for(&mut self, element: &Element) -> u32 {
        let key = Key(element.clone());
        if let Some(number) = self.by_element.get(&key) {
            return *number;
        }
        if self.by_element.len() >= REGISTRY_PRUNE_AT {
            self.prune();
        }
        let number = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.by_element.insert(key, number);
        self.by_ref.insert(number, element.clone());
        number
    }

    pub fn element(&self, number: u32) -> Option<&Element> {
        self.by_ref.get(&number)
    }

    fn prune(&mut self) {
        self.by_element.retain(|key, _| is_alive(&key.0));
        let alive: std::collections::HashSet<u32> = self.by_element.values().copied().collect();
        self.by_ref.retain(|number, _| alive.contains(number));
    }
}

fn ax_value_of<T>(kind: AXValueType, value: &T) -> Option<CFRetained<AXValue>> {
    unsafe { AXValue::new(kind, NonNull::from(value).cast()) }
}

/// The character range at a global point in a text element (`AXRangeForPosition`).
pub fn range_for_position(element: &AXUIElement, x: f64, y: f64) -> Option<objc2_core_foundation::CFRange> {
    let point = ax_value_of(AXValueType::CGPoint, &CGPoint::new(x, y))?;
    let mut result: *const CFType = std::ptr::null();
    let error = unsafe { element.copy_parameterized_attribute_value(&cf("AXRangeForPosition"), &point, NonNull::from(&mut result)) };
    if error != AXError::Success || result.is_null() {
        return None;
    }
    let result = unsafe { CFRetained::from_raw(NonNull::new_unchecked(result as *mut CFType)) };
    as_ax_value(&result, AXValueType::CFRange)
}

/// The selected range of a text element.
pub fn selected_range(element: &AXUIElement) -> Option<objc2_core_foundation::CFRange> {
    let value = attribute(element, "AXSelectedTextRange").ok()??;
    as_ax_value(&value, AXValueType::CFRange)
}

/// Selects `range` in a text element (an empty range places the caret).
pub fn select_range(element: &AXUIElement, range: objc2_core_foundation::CFRange) -> Result<(), AXError> {
    let value = ax_value_of(AXValueType::CFRange, &range).ok_or(AXError::Failure)?;
    set(element, "AXSelectedTextRange", &value)
}

/// What an element is, without its value: `button "Save"`.
pub fn describe(element: &AXUIElement) -> String {
    let role = string(element, "AXRole").unwrap_or_else(|| "AXUnknown".into());
    let subrole = string(element, "AXSubrole");
    let mut text = crate::outline::display_role(&role, subrole.as_deref());
    if let Some(label) = string(element, "AXTitle").or_else(|| string(element, "AXDescription")).filter(|label| !label.trim().is_empty()) {
        text.push_str(&format!(" \"{}\"", crate::outline::clip(&label, 80)));
    }
    text
}

/// The menu item at `path` from the menu bar down.
pub fn menu_item(app: &AXUIElement, path: &[String]) -> Result<Element, String> {
    if path.is_empty() {
        return Err("menu needs a path, like [\"File\", \"Save\"].".into());
    }
    let mut container = element(app, "AXMenuBar").ok_or("This app has no menu bar that Accessibility can reach.")?;
    for (depth, wanted) in path.iter().enumerate() {
        let item = find_menu_item(&container, wanted)?;
        if depth + 1 == path.len() {
            return Ok(item);
        }
        container = elements(&item, "AXChildren", 1).0.into_iter().next().ok_or_else(|| format!("“{wanted}” has no submenu."))?;
    }
    unreachable!("the loop returns at the last item")
}
