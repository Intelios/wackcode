//! Full-screen control on the Mac: input through the HID tap with a gate before every event,
//! the watch that gives the screen back the moment the user moves, and the glow that shows the
//! screen is taken.
//!
//! Invariants:
//! - `go` is asked before every event; a refusal stops the input, and anything held down (a
//!   mouse button mid-drag) is let go by the guard, however the input ends.
//! - The watch is an event tap that tells the user's input from ours by the marker on our
//!   events (`input::EVENT_MARKER`): anything else, or ⌃⌥⌘., ends control, once. Without the
//!   tap there is no control: `watch` fails and the screen isn't taken.
//! - The glow takes no input, is never captured, and is left out of the window list hit tests
//!   read (`overlay_windows`).

use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::sleep,
    time::Duration,
};

use objc2_core_foundation::{CFMachPort, CFRunLoop, CGPoint, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
use objc2_core_graphics::{
    CGDirectDisplayID, CGEvent, CGEventField, CGEventFlags, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType, CGGetDisplaysWithPoint,
};

use super::input::{self, Button};
use crate::{
    keys::{Layout, Modifiers},
    screen::{Ended, Go, ScreenInput},
};

const MOVE_SETTLE: Duration = Duration::from_millis(40);
const CLICK_GAP: Duration = Duration::from_millis(35);
const TYPE_GAP: Duration = Duration::from_millis(6);
const DRAG_STEP: Duration = Duration::from_millis(10);

/// A mouse button held down: let go of it whatever happens next.
struct Held {
    button: Button,
    point: CGPoint,
    down: bool,
}

impl Drop for Held {
    fn drop(&mut self) {
        if self.down {
            let (_, up, _) = input::mouse_kinds(self.button);
            let _ = input::post_hid_mouse(up, self.point, self.button, 1, &Modifiers::default());
        }
    }
}

fn point(at: (f64, f64)) -> CGPoint {
    CGPoint::new(at.0, at.1)
}

fn move_to(at: CGPoint) -> Result<(), String> {
    input::post_hid_mouse(CGEventType::MouseMoved, at, Button::Left, 0, &Modifiers::default())?;
    sleep(MOVE_SETTLE);
    Ok(())
}

/// Posts `input`, asking `go` before every event. Blocking: run it off the async threads.
pub fn perform(input: ScreenInput, go: &Go, layout: &Layout) -> Result<(), String> {
    match input {
        ScreenInput::Click { point: at, button, count, modifiers } => {
            let at = point(at);
            let button = if button == crate::backend::Button::Right { Button::Right } else { Button::Left };
            go()?;
            move_to(at)?;
            let (down, up, _) = input::mouse_kinds(button);
            let mut held = Held { button, point: at, down: false };
            for click in 1..=count.clamp(1, 3) as i64 {
                go()?;
                input::post_hid_mouse(down, at, button, click, &modifiers)?;
                held.down = true;
                sleep(Duration::from_millis(20));
                input::post_hid_mouse(up, at, button, click, &modifiers)?;
                held.down = false;
                sleep(CLICK_GAP);
            }
            Ok(())
        }
        ScreenInput::Drag { path } => {
            let points: Vec<CGPoint> = path.iter().copied().map(point).collect();
            let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else { return Err("A drag needs at least two points.".into()) };
            go()?;
            move_to(first)?;
            let (down, up, dragged) = input::mouse_kinds(Button::Left);
            input::post_hid_mouse(down, first, Button::Left, 1, &Modifiers::default())?;
            let mut held = Held { button: Button::Left, point: first, down: true };
            for pair in points.windows(2) {
                let steps = ((pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y) / 12.0).ceil().clamp(4.0, 60.0) as usize;
                for step in 1..=steps {
                    go()?;
                    let t = step as f64 / steps as f64;
                    let at = CGPoint::new(pair[0].x + (pair[1].x - pair[0].x) * t, pair[0].y + (pair[1].y - pair[0].y) * t);
                    input::post_hid_mouse(dragged, at, Button::Left, 1, &Modifiers::default())?;
                    held.point = at;
                    sleep(DRAG_STEP);
                }
            }
            input::post_hid_mouse(up, last, Button::Left, 1, &Modifiers::default())?;
            held.down = false;
            Ok(())
        }
        ScreenInput::Scroll { point: at, dx, dy } => {
            go()?;
            move_to(point(at))?;
            go()?;
            input::post_hid_scroll(dx, dy)
        }
        ScreenInput::Key { chord, times } => {
            for _ in 0..times.clamp(1, 50) {
                go()?;
                input::post_hid_key(&chord)?;
                sleep(Duration::from_millis(20));
            }
            Ok(())
        }
        ScreenInput::Type { text } => {
            let total = text.chars().count();
            for (done, ch) in text.chars().enumerate() {
                if let Err(why) = go() {
                    return Err(format!("{why} Typing stopped after {done} of {total} characters."));
                }
                input::post_hid_char(ch, layout)?;
                sleep(TYPE_GAP);
            }
            Ok(())
        }
    }
}

/// The display holding `near`, or the main display (0).
pub fn display_near(near: Option<(f64, f64)>) -> u32 {
    let Some(near) = near else { return 0 };
    let mut display: CGDirectDisplayID = 0;
    let mut count = 0u32;
    let error = unsafe { CGGetDisplaysWithPoint(point(near), 1, &mut display, &mut count) };
    if error.0 == 0 && count > 0 { display } else { 0 }
}

/// What the tap callback reaches: told once, from a thread of its own.
struct TapContext {
    ended: Arc<dyn Fn(Ended) + Send + Sync>,
    fired: AtomicBool,
    /// The key that types `.`, for the stop chord.
    period: u16,
    tap: Mutex<Option<objc2_core_foundation::CFRetained<CFMachPort>>>,
}

impl TapContext {
    fn fire(&self, ended: Ended) {
        if !self.fired.swap(true, Ordering::SeqCst) {
            let tell = self.ended.clone();
            // Never from the tap's own thread: ending control stops this watch.
            std::thread::spawn(move || tell(ended));
        }
    }
}

unsafe extern "C-unwind" fn tap_callback(_proxy: CGEventTapProxy, kind: CGEventType, event: NonNull<CGEvent>, user_info: *mut c_void) -> *mut CGEvent {
    // SAFETY: `user_info` is the `TapContext` the watch thread owns for as long as the tap runs.
    let context = unsafe { &*(user_info as *const TapContext) };
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(tap) = context.tap.lock().ok().and_then(|tap| tap.clone()) {
            CGEvent::tap_enable(&tap, true);
        }
        return event.as_ptr();
    }
    let event_ref = unsafe { event.as_ref() };
    if CGEvent::integer_value_field(Some(event_ref), CGEventField::EventSourceUserData) == input::EVENT_MARKER {
        return event.as_ptr();
    }
    if kind == CGEventType::KeyDown {
        let code = CGEvent::integer_value_field(Some(event_ref), CGEventField::KeyboardEventKeycode) as u16;
        let flags = CGEvent::flags(Some(event_ref));
        let held = |mask: CGEventFlags| flags.contains(mask);
        if crate::screen::is_stop_chord(code, held(CGEventFlags::MaskCommand), held(CGEventFlags::MaskControl), held(CGEventFlags::MaskAlternate), context.period) {
            context.fire(Ended::StopChord);
            return event.as_ptr();
        }
    }
    context.fire(Ended::UserInput);
    event.as_ptr()
}

/// The running watch: set to stop it.
static WATCH: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn mask(kinds: &[CGEventType]) -> CGEventMask {
    kinds.iter().fold(0, |mask, kind| mask | (1u64 << kind.0))
}

/// Starts watching for the user's input (and ⌃⌥⌘.), replacing any earlier watch. Fails when
/// macOS won't give WackCode an event tap.
pub fn watch(ended: Arc<dyn Fn(Ended) + Send + Sync>, period: u16) -> Result<(), String> {
    unwatch();
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, started) = std::sync::mpsc::channel::<Result<(), String>>();
    let stopping = stop.clone();
    std::thread::Builder::new()
        .name("wackcode-screen-watch".into())
        .spawn(move || {
            let context = Box::new(TapContext { ended, fired: AtomicBool::new(false), period, tap: Mutex::new(None) });
            let interest = mask(&[
                CGEventType::MouseMoved,
                CGEventType::LeftMouseDown,
                CGEventType::RightMouseDown,
                CGEventType::OtherMouseDown,
                CGEventType::LeftMouseDragged,
                CGEventType::RightMouseDragged,
                CGEventType::ScrollWheel,
                CGEventType::KeyDown,
            ]);
            let user_info = &*context as *const TapContext as *mut c_void;
            // An active tap (it passes every event on untouched) needs only Accessibility; a
            // listen-only one would need Input Monitoring too.
            let tap = unsafe { CGEvent::tap_create(CGEventTapLocation::SessionEventTap, CGEventTapPlacement::HeadInsertEventTap, CGEventTapOptions::Default, interest, Some(tap_callback), user_info) };
            let Some(tap) = tap else {
                let _ = ready.send(Err("macOS wouldn't let WackCode watch for your input, so it won't take the screen. Check Settings › Computer use › Accessibility.".into()));
                return;
            };
            let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
                let _ = ready.send(Err("The input watch couldn't start.".into()));
                return;
            };
            let Some(run_loop) = CFRunLoop::current() else {
                let _ = ready.send(Err("The input watch couldn't start.".into()));
                return;
            };
            run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
            CGEvent::tap_enable(&tap, true);
            *context.tap.lock().unwrap_or_else(|e| e.into_inner()) = Some(tap.clone());
            let _ = ready.send(Ok(()));
            while !stopping.load(Ordering::SeqCst) && !context.fired.load(Ordering::SeqCst) {
                CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.2, true);
            }
            CGEvent::tap_enable(&tap, false);
            tap.invalidate();
            *context.tap.lock().unwrap_or_else(|e| e.into_inner()) = None;
            drop(context);
        })
        .map_err(|_| "The input watch couldn't start.".to_string())?;
    started.recv_timeout(Duration::from_secs(3)).map_err(|_| "The input watch didn't start in time.".to_string())??;
    *WATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(stop);
    Ok(())
}

/// Stops the watch, if one runs. It ends within a fifth of a second.
pub fn unwatch() {
    if let Some(stop) = WATCH.lock().unwrap_or_else(|e| e.into_inner()).take() {
        stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs Accessibility for the process running it, so it is run by hand:
    /// `cargo test -p wack-computer -- --ignored the_input_watch`.
    #[test]
    #[ignore = "needs the Accessibility permission"]
    fn the_input_watch_starts_and_stops() {
        watch(Arc::new(|_| {}), 47).unwrap();
        assert!(WATCH.lock().unwrap().is_some());
        unwatch();
        assert!(WATCH.lock().unwrap().is_none());
    }
}
