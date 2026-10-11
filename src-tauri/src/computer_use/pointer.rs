//! Pointer delivery through the real cursor (the only input `wackdev::input` can't do, since
//! keys and text go straight to the target process). `Foreground` waits for the user to be
//! idle, brings the target forward, hit-tests every point so it lands on the target's own
//! window (never WackCode or a blocked app), and restores the previous front app and cursor
//! when dropped — also on errors and stops.

use super::cursor;
use super::geometry::HitOwner;
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{CGEventSource, CGMouseButton, CGWarpMouseCursorPosition};
use objc2_core_graphics::CGEventType;
use std::thread::sleep;
use std::time::{Duration, Instant};
use wackdev::{apps, ax::{self, Element}, geometry, input};

const IDLE_REQUIRED: f64 = 0.5;
const IDLE_WAIT: Duration = Duration::from_secs(2);
const ACTIVATE_WAIT: Duration = Duration::from_millis(800);

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
    held: Option<input::Button>,
    source: objc2_core_foundation::CFRetained<CGEventSource>,
    blocked: BlockCheck,
    feedback: Option<(cursor::Feedback, cursor::Target)>,
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
        if input::screen_locked() {
            return Err("The screen is locked, so computer use can't click anything.".into());
        }
        if input::secure_input_on() {
            return Err("Secure keyboard entry is on (a password field has focus somewhere), so computer use won't take over the pointer. Try again later or use element refs.".into());
        }
        let started = Instant::now();
        while input::seconds_since_input() < IDLE_REQUIRED {
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
            previous_cursor: input::cursor(),
            last_posted: None,
            held: None,
            source: input::source()?,
            blocked,
            window,
            feedback: None,
        };
        if foreground.previous_front != Some(target_pid) {
            input::set_frontmost(target_pid);
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
        let windows = input::window_list();
        let blocked = &self.blocked;
        let owner = geometry::topmost_owner(&windows, point, self.target_pid, self.own_pid, &cursor::overlay_windows(), |pid| {
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
                let hit = unsafe { objc2_core_foundation::CFRetained::from_raw(hit) };
                if ax::pid(&hit).is_some_and(|pid| pid != self.target_pid) {
                    return Err("Another app's element is at that point, so the click was refused.".into());
                }
            }
        }
        Ok(())
    }

    fn post(&mut self, kind: CGEventType, point: CGPoint, button: CGMouseButton, click_state: i64) -> Result<(), String> {
        input::post_mouse(&self.source, kind, point, button, click_state)?;
        self.last_posted = Some(point);
        Ok(())
    }

    pub fn set_feedback(&mut self, feedback: Option<(cursor::Feedback, cursor::Target)>) { self.feedback = feedback; }

    fn show_point(&self, point: (f64, f64)) {
        if let Some((feedback, target)) = &self.feedback { feedback.at(Some(cursor::Target { point, ..*target })); }
    }

    pub fn click(&mut self, point: (f64, f64), button: input::Button, count: u8) -> Result<(), String> {
        self.check(point)?;
        self.show_point(point);
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
        self.show_point(from);
        let (down, up, dragged, cg_button) = input::Button::Left.events();
        let start = CGPoint::new(from.0, from.1);
        self.post(CGEventType::MouseMoved, start, cg_button, 0)?;
        sleep(Duration::from_millis(20));
        self.held = Some(input::Button::Left);
        self.post(down, start, cg_button, 1)?;
        const STEPS: u32 = 16;
        for step in 1..=STEPS {
            if stopped() {
                return Err("The drag was stopped.".into());
            }
            let t = f64::from(step) / f64::from(STEPS);
            let point = CGPoint::new(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            self.post(dragged, point, cg_button, 1)?;
            self.show_point((point.x, point.y));
            sleep(Duration::from_millis(14));
        }
        self.post(up, CGPoint::new(to.0, to.1), cg_button, 1)?;
        self.held = None;
        Ok(())
    }

    /// Scrolls by `dx`/`dy` steps (positive dy scrolls down) with the pointer at `point`.
    pub fn scroll(&mut self, point: (f64, f64), dx: f64, dy: f64) -> Result<(), String> {
        self.check(point)?;
        self.show_point(point);
        let at = CGPoint::new(point.0, point.1);
        self.post(CGEventType::MouseMoved, at, CGMouseButton::Left, 0)?;
        sleep(Duration::from_millis(20));
        input::post_scroll(&self.source, dx, dy)?;
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
            input::set_frontmost(previous);
        }
        // Put the cursor back only if the user hasn't moved it since the last event we posted.
        if let Some(last) = self.last_posted {
            let now = input::cursor();
            if (now.x - last.x).abs() < 1.0 && (now.y - last.y).abs() < 1.0 {
                let _ = CGWarpMouseCursorPosition(self.previous_cursor);
            }
        }
    }
}
