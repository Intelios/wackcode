//! Synthetic input. Keys and text only ever go to the target process (`CGEventPostToPid`),
//! never into the system's event stream, so they can't reach WackCode or another app. Pointer
//! events can only be delivered through the real cursor; they go through `Foreground`, which
//! waits for the user to be idle, brings the target forward, hit-tests every point so it lands
//! on the target's own window (never WackCode or a blocked app), and restores the previous
//! front app and cursor when dropped — also on errors and stops.

use super::apps;
use super::ax::{self, Element};
use super::geometry::{self, HitOwner, Rect, WindowEntry};
use super::keys::{self, Chord};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect};
use objc2_core_graphics::{
    kCGWindowBounds, kCGWindowNumber, kCGWindowOwnerPID, CGEvent, CGEventField, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton, CGRectMakeWithDictionaryRepresentation,
    CGScrollEventUnit, CGSessionCopyCurrentDictionary, CGWarpMouseCursorPosition, CGWindowListCopyWindowInfo,
    CGWindowListOption,
};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// Tags WackCode's synthetic events (`kCGEventSourceUserData`).
const EVENT_MARKER: i64 = 0x5741_434B;
const TEXT_CHUNK: usize = 20;
const IDLE_REQUIRED: f64 = 0.5;
const IDLE_WAIT: Duration = Duration::from_secs(2);
const ACTIVATE_WAIT: Duration = Duration::from_millis(800);

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    /// True while any app has secure keyboard entry on (a password field is focused).
    fn IsSecureEventInputEnabled() -> u8;
}

fn source() -> Result<CFRetained<CGEventSource>, String> {
    let source = CGEventSource::new(CGEventSourceStateID::Private).ok_or("macOS refused to create an input source.")?;
    CGEventSource::set_user_data(Some(&source), EVENT_MARKER);
    Ok(source)
}

fn flags(modifiers: &keys::Modifiers) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    if modifiers.command {
        flags |= CGEventFlags::MaskCommand;
    }
    if modifiers.control {
        flags |= CGEventFlags::MaskControl;
    }
    if modifiers.option {
        flags |= CGEventFlags::MaskAlternate;
    }
    if modifiers.shift {
        flags |= CGEventFlags::MaskShift;
    }
    if modifiers.function {
        flags |= CGEventFlags::MaskSecondaryFn;
    }
    flags
}

/// Presses and releases `chord` in process `pid`.
pub fn key_to_pid(pid: i32, chord: &Chord) -> Result<(), String> {
    let source = source()?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(Some(&source), chord.key_code, down).ok_or("macOS refused to create a key event.")?;
        CGEvent::set_flags(Some(&event), flags(&chord.modifiers));
        CGEvent::post_to_pid(pid, Some(&event));
        sleep(Duration::from_millis(12));
    }
    Ok(())
}

/// Types `text` into process `pid` as Unicode key events, in small chunks.
pub fn text_to_pid(pid: i32, text: &str, stopped: &dyn Fn() -> bool) -> Result<(), String> {
    let source = source()?;
    for chunk in keys::utf16_chunks(text, TEXT_CHUNK) {
        if stopped() {
            return Err("Typing was stopped.".into());
        }
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(Some(&source), 0, down).ok_or("macOS refused to create a key event.")?;
            unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), chunk.len() as _, chunk.as_ptr()) };
            CGEvent::post_to_pid(pid, Some(&event));
        }
        sleep(Duration::from_millis(10));
    }
    Ok(())
}

pub fn secure_input_on() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

fn number_value(dictionary: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<f64> {
    dictionary.get(key).and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64))
}

/// On-screen windows, front to back, without desktop elements. Needs no permission: titles
/// (the part Screen Recording guards) aren't read.
pub fn window_list() -> Vec<WindowEntry> {
    let option = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(option, 0) else { return Vec::new() };
    // SAFETY: the window list is an array of dictionaries keyed by strings.
    let list: &CFArray<CFDictionary<CFString, CFType>> = unsafe { list.cast_unchecked() };
    list.iter()
        .filter_map(|window| {
            let number = number_value(&window, unsafe { kCGWindowNumber })? as u32;
            let pid = number_value(&window, unsafe { kCGWindowOwnerPID })? as i32;
            let bounds = window.get(unsafe { kCGWindowBounds })?;
            let bounds = bounds.downcast_ref::<CFDictionary>()?;
            let mut rect = CGRect::default();
            let ok = unsafe { CGRectMakeWithDictionaryRepresentation(Some(bounds), &mut rect) };
            ok.then_some(WindowEntry {
                number,
                pid,
                bounds: Rect { x: rect.origin.x, y: rect.origin.y, width: rect.size.width, height: rect.size.height },
            })
        })
        .collect()
}

/// The window-server number of an AX window: the private mapping, else the on-screen window
/// of that pid with the same frame.
pub fn window_number(window: &Element, pid: i32) -> Option<u32> {
    ax::window_number(window).or_else(|| {
        let frame = ax::frame(window)?;
        window_list().into_iter().find(|entry| entry.pid == pid && entry.bounds.approx_eq(&frame)).map(|entry| entry.number)
    })
}

fn screen_locked() -> bool {
    let Some(session) = CGSessionCopyCurrentDictionary() else { return false };
    // SAFETY: the session dictionary is keyed by strings.
    let session: &CFDictionary<CFString, CFType> = unsafe { session.cast_unchecked() };
    session
        .get(&CFString::from_static_str("CGSSessionScreenIsLocked"))
        .is_some_and(|value| value.downcast_ref::<CFBoolean>().is_some_and(CFBoolean::as_bool) || value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64).is_some_and(|number| number != 0))
}

pub fn cursor() -> CGPoint {
    CGEvent::new(None).map(|event| CGEvent::location(Some(&event))).unwrap_or_default()
}

fn seconds_since_input() -> f64 {
    // Any input event type (`kCGAnyInputEventType`), from the hardware.
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, CGEventType(u32::MAX))
}

fn set_frontmost(pid: i32) -> bool {
    let app = ax::application(pid);
    ax::set_bool(&app, "AXFrontmost", true).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn events(self) -> (CGEventType, CGEventType, CGEventType, CGMouseButton) {
        match self {
            Self::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGEventType::LeftMouseDragged, CGMouseButton::Left),
            Self::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGEventType::RightMouseDragged, CGMouseButton::Right),
            Self::Middle => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGEventType::OtherMouseDragged, CGMouseButton::Center),
        }
    }
}

/// Says whether a bundle id is blocked (`policy::block_reason`), for naming whoever covers a point.
pub type BlockCheck = Box<dyn Fn(Option<&str>) -> bool>;

/// Pointer delivery through the real cursor, with the target in front. Dropping it releases any
/// held button and restores the front app and — if the user hasn't moved it — the cursor.
pub struct Foreground {
    target_pid: i32,
    own_pid: i32,
    window: Element,
    previous_front: Option<i32>,
    previous_cursor: CGPoint,
    last_posted: Option<CGPoint>,
    held: Option<Button>,
    source: CFRetained<CGEventSource>,
    blocked: BlockCheck,
}

impl Foreground {
    /// Waits for the user to be idle and brings the target (and `window`) to the front.
    pub fn begin(
        target_pid: i32,
        own_pid: i32,
        window: Element,
        blocked: BlockCheck,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        if screen_locked() {
            return Err("The screen is locked, so computer use can't click anything.".into());
        }
        if secure_input_on() {
            return Err("Secure keyboard entry is on (a password field has focus somewhere), so computer use won't take over the pointer. Try again later or use element refs.".into());
        }
        let started = Instant::now();
        while seconds_since_input() < IDLE_REQUIRED {
            if stopped() {
                return Err("Stopped.".into());
            }
            if started.elapsed() > IDLE_WAIT {
                return Err("The user is using the mouse or keyboard right now, so computer use won't move the pointer. Try again in a moment, or use element refs.".into());
            }
            sleep(Duration::from_millis(100));
        }
        let foreground = Self {
            target_pid,
            own_pid,
            previous_front: apps::frontmost_pid(),
            previous_cursor: cursor(),
            last_posted: None,
            held: None,
            source: source()?,
            blocked,
            window,
        };
        if foreground.previous_front != Some(target_pid) {
            set_frontmost(target_pid);
        }
        let _ = ax::perform(&foreground.window, "AXRaise");
        let deadline = Instant::now() + ACTIVATE_WAIT;
        while apps::frontmost_pid() != Some(target_pid) {
            if Instant::now() > deadline {
                return Err("The app couldn't be brought to the front for a pointer action.".into());
            }
            sleep(Duration::from_millis(40));
        }
        // Let the window server settle the new window order before hit-testing.
        sleep(Duration::from_millis(60));
        Ok(foreground)
    }

    /// Refuses unless the topmost window at `point` belongs to the target.
    fn check(&self, point: (f64, f64)) -> Result<(), String> {
        let windows = window_list();
        let blocked = &self.blocked;
        let owner = geometry::topmost_owner(&windows, point, self.target_pid, self.own_pid, &[], |pid| {
            let (name, bundle_id) = apps::name_of(pid);
            let is_blocked = blocked(bundle_id.as_deref());
            (name, is_blocked)
        });
        match owner {
            HitOwner::Target => {}
            HitOwner::Own => return Err("WackCode's own window is in front of that point, so the click was refused.".into()),
            HitOwner::Blocked(name) => return Err(format!("{name} is in front of that point, and computer use never clicks it.")),
            HitOwner::Other(name) => return Err(format!("{name} covers that point, so the click was refused. Move or close it, or use element refs.")),
            HitOwner::Nothing => return Err("No window of the app is at that point.".into()),
        }
        // A second opinion from Accessibility, when it has one.
        let system = ax::system_wide();
        let mut hit: *const objc2_application_services::AXUIElement = std::ptr::null();
        let error = unsafe { system.copy_element_at_position(point.0 as f32, point.1 as f32, std::ptr::NonNull::from(&mut hit)) };
        if error == objc2_application_services::AXError::Success {
            if let Some(hit) = std::ptr::NonNull::new(hit as *mut objc2_application_services::AXUIElement) {
                let hit = unsafe { CFRetained::from_raw(hit) };
                if ax::pid(&hit).is_some_and(|pid| pid != self.target_pid) {
                    return Err("Another app's element is at that point, so the click was refused.".into());
                }
            }
        }
        Ok(())
    }

    fn post(&mut self, kind: CGEventType, point: CGPoint, button: CGMouseButton, click_state: i64) -> Result<(), String> {
        let event = CGEvent::new_mouse_event(Some(&self.source), kind, point, button).ok_or("macOS refused to create a mouse event.")?;
        if click_state > 0 {
            CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, click_state);
        }
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        self.last_posted = Some(point);
        Ok(())
    }

    pub fn click(&mut self, point: (f64, f64), button: Button, count: u8) -> Result<(), String> {
        self.check(point)?;
        let at = CGPoint::new(point.0, point.1);
        let (down, up, _, cg_button) = button.events();
        self.post(CGEventType::MouseMoved, at, cg_button, 0)?;
        sleep(Duration::from_millis(20));
        for click in 1..=i64::from(count.max(1)) {
            self.held = Some(button);
            self.post(down, at, cg_button, click)?;
            sleep(Duration::from_millis(30));
            self.post(up, at, cg_button, click)?;
            self.held = None;
            sleep(Duration::from_millis(40));
        }
        Ok(())
    }

    pub fn drag(&mut self, from: (f64, f64), to: (f64, f64), stopped: &dyn Fn() -> bool) -> Result<(), String> {
        self.check(from)?;
        self.check(to)?;
        let (down, up, dragged, cg_button) = Button::Left.events();
        let start = CGPoint::new(from.0, from.1);
        self.post(CGEventType::MouseMoved, start, cg_button, 0)?;
        sleep(Duration::from_millis(20));
        self.held = Some(Button::Left);
        self.post(down, start, cg_button, 1)?;
        const STEPS: u32 = 16;
        for step in 1..=STEPS {
            if stopped() {
                return Err("The drag was stopped.".into());
            }
            let t = f64::from(step) / f64::from(STEPS);
            let point = CGPoint::new(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            self.post(dragged, point, cg_button, 1)?;
            sleep(Duration::from_millis(14));
        }
        self.post(up, CGPoint::new(to.0, to.1), cg_button, 1)?;
        self.held = None;
        Ok(())
    }

    /// Scrolls by `dx`/`dy` steps (positive dy scrolls down) with the pointer at `point`.
    pub fn scroll(&mut self, point: (f64, f64), dx: f64, dy: f64) -> Result<(), String> {
        self.check(point)?;
        let at = CGPoint::new(point.0, point.1);
        self.post(CGEventType::MouseMoved, at, CGMouseButton::Left, 0)?;
        sleep(Duration::from_millis(20));
        let lines = |steps: f64| (-steps * 3.0).round().clamp(-500.0, 500.0) as i32;
        let event = CGEvent::new_scroll_wheel_event2(Some(&self.source), CGScrollEventUnit::Line, 2, lines(dy), lines(dx), 0)
            .ok_or("macOS refused to create a scroll event.")?;
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        self.last_posted = Some(at);
        Ok(())
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        if let Some(button) = self.held.take() {
            let (_, up, _, cg_button) = button.events();
            let at = self.last_posted.unwrap_or(self.previous_cursor);
            let _ = self.post(up, at, cg_button, 1);
        }
        sleep(Duration::from_millis(60));
        if let Some(previous) = self.previous_front.filter(|pid| *pid != self.target_pid) {
            set_frontmost(previous);
        }
        // Put the cursor back only if the user hasn't moved it since the last event we posted.
        if let Some(last) = self.last_posted {
            let now = cursor();
            if (now.x - last.x).abs() < 1.0 && (now.y - last.y).abs() < 1.0 {
                let _ = CGWarpMouseCursorPosition(self.previous_cursor);
            }
        }
    }
}
