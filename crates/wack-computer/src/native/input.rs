//! Synthetic input and the window server's view of the screen. Ported from version 2's
//! `wackdev/src/input.rs`.
//!
//! Invariants:
//! - In the background, keys, text, clicks and scrolls go to the target **process**
//!   (`CGEventPostToPid`): they never enter the system's event stream, never move the user's
//!   pointer and can't reach WackCode or another app.
//! - Only full-screen control posts to the HID tap (`post_hid_*`), and only after its own checks
//!   (`screen.rs`).
//! - Every event comes from a source carrying [`EVENT_MARKER`], so the full-control watch can tell
//!   the user's own input from ours.

use std::{thread::sleep, time::Duration};

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMouseButton,
    CGRectMakeWithDictionaryRepresentation, CGScrollEventUnit, CGSessionCopyCurrentDictionary, CGWindowListCopyWindowInfo, CGWindowListOption,
    kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
};

use crate::{
    geometry::{Rect, WindowEntry},
    keys::{self, Chord},
};

/// Tags WackCode's synthetic events (`kCGEventSourceUserData`): "WACK".
pub const EVENT_MARKER: i64 = 0x5741_434B;
const TEXT_CHUNK: usize = 20;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    /// True while any app has secure keyboard entry on (a password field is focused).
    fn IsSecureEventInputEnabled() -> u8;
}

/// A private event source carrying WackCode's marker.
pub fn source() -> Result<CFRetained<CGEventSource>, String> {
    let source = CGEventSource::new(CGEventSourceStateID::Private).ok_or("macOS refused to create an input source.")?;
    CGEventSource::set_user_data(Some(&source), EVENT_MARKER);
    Ok(source)
}

pub fn flags(modifiers: &keys::Modifiers) -> CGEventFlags {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

impl Button {
    fn events(self) -> (CGEventType, CGEventType, CGEventType, CGMouseButton) {
        match self {
            Self::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGEventType::LeftMouseDragged, CGMouseButton::Left),
            Self::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGEventType::RightMouseDragged, CGMouseButton::Right),
        }
    }
}

fn mouse_event(source: &CGEventSource, kind: CGEventType, point: CGPoint, button: CGMouseButton, window: Option<u32>, click_state: i64) -> Result<CFRetained<CGEvent>, String> {
    let event = CGEvent::new_mouse_event(Some(source), kind, point, button).ok_or("macOS refused to create a mouse event.")?;
    if click_state > 0 {
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, click_state);
    }
    if let Some(window) = window {
        // Without these, an app in the background looks for the window under the real pointer.
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventWindowUnderMousePointer, window as i64);
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent, window as i64);
    }
    Ok(event)
}

/// Clicks `count` times at `point` (global points) in window `window` of process `pid`, without
/// moving the pointer or bringing the app forward.
pub fn click_to_pid(pid: i32, window: u32, point: CGPoint, button: Button, count: u32, modifiers: &keys::Modifiers) -> Result<(), String> {
    let source = source()?;
    let (down, up, _, cg_button) = button.events();
    let flags = flags(modifiers);
    for click in 1..=count.clamp(1, 3) as i64 {
        for kind in [down, up] {
            let event = mouse_event(&source, kind, point, cg_button, Some(window), click)?;
            CGEvent::set_flags(Some(&event), flags);
            CGEvent::post_to_pid(pid, Some(&event));
            sleep(Duration::from_millis(if kind == down { 30 } else { 40 }));
        }
    }
    Ok(())
}

/// Drags through `path` (global points) in window `window` of process `pid`.
pub fn drag_to_pid(pid: i32, window: u32, path: &[CGPoint]) -> Result<(), String> {
    let (Some(first), Some(last)) = (path.first(), path.last()) else { return Err("A drag needs at least two points.".into()) };
    let source = source()?;
    let (down, up, dragged, button) = Button::Left.events();
    let event = mouse_event(&source, down, *first, button, Some(window), 1)?;
    CGEvent::post_to_pid(pid, Some(&event));
    sleep(Duration::from_millis(40));
    for pair in path.windows(2) {
        for step in 1..=8 {
            let t = step as f64 / 8.0;
            let point = CGPoint::new(pair[0].x + (pair[1].x - pair[0].x) * t, pair[0].y + (pair[1].y - pair[0].y) * t);
            let event = mouse_event(&source, dragged, point, button, Some(window), 1)?;
            CGEvent::post_to_pid(pid, Some(&event));
            sleep(Duration::from_millis(12));
        }
    }
    let event = mouse_event(&source, up, *last, button, Some(window), 1)?;
    CGEvent::post_to_pid(pid, Some(&event));
    Ok(())
}

fn wheel_lines(steps: f64) -> i32 {
    (-steps * 3.0).round().clamp(-500.0, 500.0) as i32
}

/// Sends a scroll-wheel event to process `pid` at `point` (global points). It lands only in that
/// pid and never moves the real pointer.
pub fn scroll_to_pid(pid: i32, point: CGPoint, dx: f64, dy: f64) -> Result<(), String> {
    let source = source()?;
    let event = CGEvent::new_scroll_wheel_event2(Some(&source), CGScrollEventUnit::Line, 2, wheel_lines(dy), wheel_lines(dx), 0)
        .ok_or("macOS refused to create a scroll event.")?;
    CGEvent::set_location(Some(&event), point);
    CGEvent::post_to_pid(pid, Some(&event));
    Ok(())
}

/// Full-screen control only: one mouse event at `point` through the HID tap (the real pointer).
pub fn post_hid_mouse(kind: CGEventType, point: CGPoint, button: Button, click_state: i64, modifiers: &keys::Modifiers) -> Result<(), String> {
    let source = source()?;
    let (_, _, _, cg_button) = button.events();
    let event = mouse_event(&source, kind, point, cg_button, None, click_state)?;
    CGEvent::set_flags(Some(&event), flags(modifiers));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

pub fn mouse_kinds(button: Button) -> (CGEventType, CGEventType, CGEventType) {
    let (down, up, dragged, _) = button.events();
    (down, up, dragged)
}

/// Full-screen control only: a key chord through the HID tap, to whatever app holds focus.
pub fn post_hid_key(chord: &Chord) -> Result<(), String> {
    let source = source()?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(Some(&source), chord.key_code, down).ok_or("macOS refused to create a key event.")?;
        CGEvent::set_flags(Some(&event), flags(&chord.modifiers));
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        sleep(Duration::from_millis(12));
    }
    Ok(())
}

/// Full-screen control only: one character (a grapheme's UTF-16) through the HID tap.
pub fn post_hid_text(units: &[u16]) -> Result<(), String> {
    let source = source()?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(Some(&source), 0, down).ok_or("macOS refused to create a key event.")?;
        unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr()) };
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    }
    Ok(())
}

/// Full-screen control only: a scroll at the pointer through the HID tap.
pub fn post_hid_scroll(dx: f64, dy: f64) -> Result<(), String> {
    let source = source()?;
    let event = CGEvent::new_scroll_wheel_event2(Some(&source), CGScrollEventUnit::Line, 2, wheel_lines(dy), wheel_lines(dx), 0)
        .ok_or("macOS refused to create a scroll event.")?;
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    Ok(())
}

pub fn secure_input_on() -> bool {
    unsafe { IsSecureEventInputEnabled() != 0 }
}

fn number_value(dictionary: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<f64> {
    dictionary.get(key).and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64))
}

/// On-screen windows, front to back, without desktop elements. Needs no permission: titles (the
/// part Screen Recording guards) aren't read.
pub fn window_list() -> Vec<WindowEntry> {
    window_list_with(CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements)
}

/// Every window, on screen or not (minimized, other Spaces), in no useful order.
pub fn all_windows() -> Vec<WindowEntry> {
    window_list_with(CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements)
}

fn window_list_with(option: CGWindowListOption) -> Vec<WindowEntry> {
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
                bounds: Rect::new(rect.origin.x, rect.origin.y, rect.size.width, rect.size.height),
                layer: number_value(&window, unsafe { kCGWindowLayer }).unwrap_or(0.0) as i32,
                alpha: number_value(&window, unsafe { kCGWindowAlpha }).unwrap_or(1.0),
            })
        })
        .collect()
}

/// Whether the login session's screen is locked.
pub fn screen_locked() -> bool {
    let Some(session) = CGSessionCopyCurrentDictionary() else { return false };
    // SAFETY: the session dictionary is keyed by strings.
    let session: &CFDictionary<CFString, CFType> = unsafe { session.cast_unchecked() };
    session.get(&CFString::from_static_str("CGSSessionScreenIsLocked")).is_some_and(|value| {
        value.downcast_ref::<CFBoolean>().is_some_and(CFBoolean::as_bool) || value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64).is_some_and(|number| number != 0)
    })
}

pub fn pointer() -> CGPoint {
    CGEvent::new(None).map(|event| CGEvent::location(Some(&event))).unwrap_or_default()
}

/// Seconds since the user last pressed a key (any key, from the hardware).
pub fn seconds_since_typing() -> f64 {
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, CGEventType::KeyDown)
}

/// Seconds since any input from the hardware: keys, the pointer, the wheel.
pub fn seconds_since_input() -> f64 {
    // `kCGAnyInputEventType`.
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, CGEventType(u32::MAX))
}
