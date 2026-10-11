//! Synthetic input every caller shares. Keys and text only ever go to the target process
//! (`CGEventPostToPid`), never into the system's event stream, so they can't reach WackCode or
//! another app. Pointer delivery needs the real cursor and lives in the app's
//! `computer_use::pointer` (`Foreground`); nothing here can move the mouse.

use super::ax::{self, Element};
use super::geometry::{Rect, WindowEntry};
use super::keys::{self, Chord};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect};
use objc2_core_graphics::{
    kCGWindowBounds, kCGWindowNumber, kCGWindowOwnerPID, CGEvent, CGEventField, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton, CGRectMakeWithDictionaryRepresentation,
    CGScrollEventUnit, CGSessionCopyCurrentDictionary, CGWindowListCopyWindowInfo, CGWindowListOption,
};
use std::thread::sleep;
use std::time::Duration;

/// Tags WackCode's synthetic events (`kCGEventSourceUserData`).
pub(crate) const EVENT_MARKER: i64 = 0x5741_434B;
const TEXT_CHUNK: usize = 20;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    /// True while any app has secure keyboard entry on (a password field is focused).
    fn IsSecureEventInputEnabled() -> u8;
}

/// A private event source carrying WackCode's marker. `pub` because the app's `Foreground`
/// posts pointer events through it.
pub fn source() -> Result<CFRetained<CGEventSource>, String> {
    let source = CGEventSource::new(CGEventSourceStateID::Private).ok_or("macOS refused to create an input source.")?;
    CGEventSource::set_user_data(Some(&source), EVENT_MARKER);
    Ok(source)
}

pub(crate) fn flags(modifiers: &keys::Modifiers) -> CGEventFlags {
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

/// Whether the login session's screen is locked (computer-use pointer paths check it).
pub fn screen_locked() -> bool {
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

pub fn seconds_since_input() -> f64 {
    // Any input event type (`kCGAnyInputEventType`), from the hardware.
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, CGEventType(u32::MAX))
}

/// Brings `pid` to the front through Accessibility (no LaunchServices, no activation side
/// effects it can't see). Pointer delivery uses this; the driver never does.
pub fn set_frontmost(pid: i32) -> bool {
    let app = ax::application(pid);
    ax::set_bool(&app, "AXFrontmost", true).is_ok()
}

/// One mouse button's CG event triple. Kept here because both the app's `Foreground` and
/// anything that ever posts a mouse event needs the same mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    pub fn events(self) -> (CGEventType, CGEventType, CGEventType, CGMouseButton) {
        match self {
            Self::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGEventType::LeftMouseDragged, CGMouseButton::Left),
            Self::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGEventType::RightMouseDragged, CGMouseButton::Right),
            Self::Middle => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGEventType::OtherMouseDragged, CGMouseButton::Center),
        }
    }
}

/// Posts one mouse event of `kind` at `point` (computer-use pointer delivery only; the driver
/// does not use it).
pub fn post_mouse(source: &CGEventSource, kind: CGEventType, point: CGPoint, button: CGMouseButton, click_state: i64) -> Result<(), String> {
    let event = CGEvent::new_mouse_event(Some(source), kind, point, button).ok_or("macOS refused to create a mouse event.")?;
    if click_state > 0 {
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, click_state);
    }
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

/// Posts one scroll-wheel event (computer-use pointer delivery only).
pub fn post_scroll(source: &CGEventSource, dx: f64, dy: f64) -> Result<(), String> {
    let lines = |steps: f64| (-steps * 3.0).round().clamp(-500.0, 500.0) as i32;
    let event = CGEvent::new_scroll_wheel_event2(Some(source), CGScrollEventUnit::Line, 2, lines(dy), lines(dx), 0)
        .ok_or("macOS refused to create a scroll event.")?;
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

/// Sends a scroll-wheel event to process `pid` at `point` (global points, usually an
/// element's centre). The webview honors wheel events where AX scroll-bar writes don't; the
/// event lands only in that pid and never moves the real cursor.
pub fn scroll_to_pid(pid: i32, point: CGPoint, dx: f64, dy: f64) -> Result<(), String> {
    let source = source()?;
    let lines = |steps: f64| (-steps * 3.0).round().clamp(-500.0, 500.0) as i32;
    let event = CGEvent::new_scroll_wheel_event2(Some(&source), CGScrollEventUnit::Line, 2, lines(dy), lines(dx), 0)
        .ok_or("macOS refused to create a scroll event.")?;
    CGEvent::set_location(Some(&event), point);
    CGEvent::post_to_pid(pid, Some(&event));
    Ok(())
}
