//! A visual account of dispatched computer actions, never an input source. Only one marker
//! exists: the latest action owns it, even when another chat has a marker to leave behind.
//! Run generations and action sequences are checked again on the main thread, so queued work
//! cannot resurrect a stopped run or a superseded action. No AX object crosses this boundary.
//!
//! The marker is anchored to the app window under its point and drawn directly above that
//! window, so whatever covers the window covers the cursor too. It keeps its place across
//! actions that have no position (keys, waits), and follows its window when it moves.

mod native;

use super::engine::StopToken;
use super::geometry::{Rect, WindowEntry};
pub use crate::models::ComputerCursorAppearance as CursorAppearance;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::AppHandle;

impl CursorAppearance {
    pub fn validate(&self) -> Result<(), String> {
        if self.agent_name.trim().is_empty() || self.agent_name.chars().count() > 20_000 {
            return Err("The agent cursor needs a name of 1–20,000 characters.".into());
        }
        for colour in [
            &self.accent,
            &self.outline,
            &self.surface,
            &self.text,
            &self.muted_text,
        ] {
            if crate::glass::parse_hex(colour).is_none() {
                return Err("The agent cursor needs valid theme colours.".into());
            }
        }
        Ok(())
    }

    /// The default theme, for a run that starts before the renderer has sent its own.
    fn fallback() -> Self {
        Self {
            agent_name: "WackCode".into(),
            accent: "#c2ee4a".into(),
            outline: "#111310".into(),
            surface: "#1c1f1a".into(),
            text: "#ecefe6".into(),
            muted_text: "#9ba394".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Clicking,
    Dragging,
    Scrolling,
    Typing,
    Pressing,
    Menu,
}

impl Kind {
    fn label(self, completed: bool) -> &'static str {
        match (self, completed) {
            (Self::Clicking, false) => "Clicking",
            (Self::Dragging, false) => "Dragging",
            (Self::Scrolling, false) => "Scrolling",
            (Self::Typing, false) => "Typing",
            (Self::Pressing, false) => "Pressing",
            (Self::Menu, false) => "Opening menu",
            (Self::Clicking, true) => "Clicked",
            (Self::Dragging, true) => "Dragged",
            (Self::Scrolling, true) => "Scrolled",
            (Self::Typing, true) => "Typed",
            (Self::Pressing, true) => "Pressed",
            (Self::Menu, true) => "Opened menu",
        }
    }
}

/// The app window a marker sits above, as the window list reported it when the point was set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub window: u32,
    pub frame: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pid: i32,
    /// `None` for the menu bar, which belongs to no app window and is drawn at status level.
    pub anchor: Option<Anchor>,
    pub point: (f64, f64),
}

impl Target {
    /// Anchors `point` to the front-most visible window of `pid` containing it: the main
    /// window, or a menu, popover, sheet or panel of the same app in front of it.
    pub fn in_window(pid: i32, point: (f64, f64), windows: &[WindowEntry], skip: &[u32]) -> Option<Self> {
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        let window = windows.iter().find(|window| {
            window.pid == pid && drawn(window, skip) && window.bounds.contains(point.0, point.1)
        })?;
        Some(Self {
            pid,
            anchor: Some(Anchor { window: window.number, frame: window.bounds }),
            point,
        })
    }

    pub fn on_screen(pid: i32, point: (f64, f64)) -> Option<Self> {
        (point.0.is_finite() && point.1.is_finite()).then_some(Self { pid, anchor: None, point })
    }
}

/// A window that actually paints: on-screen lists include transparent and empty ones.
fn drawn(window: &WindowEntry, skip: &[u32]) -> bool {
    !skip.contains(&window.number) && window.alpha > 0.0 && window.bounds.has_area()
}

/// Seconds the tip takes to glide to a new point.
const GLIDE: f64 = 0.22;

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The tip `elapsed` seconds into a glide. Reduced motion jumps straight to the target.
fn glide(from: Option<(f64, f64)>, to: (f64, f64), elapsed: f64, reduced: bool) -> (f64, f64) {
    match from {
        Some(from) if !reduced && elapsed < GLIDE => {
            let t = ease_out(elapsed / GLIDE);
            (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        }
        _ => to,
    }
}

#[derive(Clone)]
struct Marker {
    task: String,
    generation: u64,
    sequence: u64,
    token: StopToken,
    target: Target,
    kind: Kind,
    started: Instant,
    completed: Option<Instant>,
    /// Where the current glide started; `None` on first appearance.
    from: Option<(f64, f64)>,
    /// When the target point last changed.
    moved: Instant,
    /// When the marker first appeared (its fade-in).
    shown: Instant,
}

impl Marker {
    fn tip(&self, now: Instant, reduced: bool) -> (f64, f64) {
        glide(self.from, self.target.point, now.duration_since(self.moved).as_secs_f64(), reduced)
    }

    /// Seconds since the tip arrived at its point (negative while still gliding).
    fn since_landing(&self, now: Instant, reduced: bool) -> f64 {
        let elapsed = now.duration_since(self.moved).as_secs_f64();
        if self.from.is_some() && !reduced {
            elapsed - GLIDE
        } else {
            elapsed
        }
    }

    /// The window moved by `offset`: carry the point and any glide in progress with it.
    fn shift(&mut self, offset: (f64, f64), frame: Rect) {
        self.target.point = (self.target.point.0 + offset.0, self.target.point.1 + offset.1);
        self.from = self.from.map(|from| (from.0 + offset.0, from.1 + offset.1));
        if let Some(anchor) = self.target.anchor.as_mut() {
            anchor.frame = frame;
        }
    }
}

#[derive(Default)]
struct State {
    enabled: bool,
    generation: u64,
    sequence: u64,
    runs: HashMap<String, u64>,
    marker: Option<Marker>,
    appearance: Option<CursorAppearance>,
}

impl State {
    fn start_run(&mut self, task: &str) {
        if !self.runs.contains_key(task) {
            self.generation += 1;
            self.runs.insert(task.into(), self.generation);
        }
    }

    fn end_run(&mut self, task: &str) {
        self.runs.remove(task);
        if self
            .marker
            .as_ref()
            .is_some_and(|marker| marker.task == task)
        {
            self.marker = None;
        }
    }

    fn valid(&self, session: &Session, sequence: u64) -> bool {
        self.enabled
            && !session.token.is_stopped()
            && self.runs.get(&session.task) == Some(&session.generation)
            && self.sequence == sequence
    }

    /// A new action takes the marker over. The chat's own marker stays where it is (an action
    /// with no position, like a key press, keeps showing there); another chat's is retired.
    fn begin(&mut self, session: &Session, kind: Kind) -> Option<u64> {
        if !self.enabled
            || session.token.is_stopped()
            || self.runs.get(&session.task) != Some(&session.generation)
        {
            return None;
        }
        self.sequence += 1;
        let sequence = self.sequence;
        match self.marker.as_mut() {
            Some(marker) if marker.task == session.task && marker.generation == session.generation => {
                marker.sequence = sequence;
                marker.token = session.token.clone();
                marker.kind = kind;
                marker.started = Instant::now();
                marker.completed = None;
            }
            _ => self.marker = None,
        }
        Some(sequence)
    }

    fn enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.sequence += 1; // Even an action in flight when the toggle flips is retired.
            self.marker = None;
            self.enabled = enabled;
        }
    }
}

struct Shared {
    state: Mutex<State>,
    app: OnceLock<AppHandle>,
}

#[derive(Clone)]
pub struct Cursor(Arc<Shared>);

impl Default for Cursor {
    fn default() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                enabled: true,
                ..State::default()
            }),
            app: OnceLock::new(),
        }))
    }
}

impl Cursor {
    pub fn start_run(&self, app: &AppHandle, task: &str) {
        let _ = self.0.app.set(app.clone());
        if let Ok(mut state) = self.0.state.lock() {
            state.start_run(task);
        }
    }

    pub fn session(&self, task: &str, token: StopToken) -> Option<Session> {
        let generation = *self.0.state.lock().ok()?.runs.get(task)?;
        Some(Session {
            cursor: self.clone(),
            task: task.into(),
            generation,
            token,
        })
    }

    pub fn end_run(&self, task: &str) {
        if let Ok(mut state) = self.0.state.lock() {
            state.end_run(task);
        }
        self.wake();
    }

    pub fn set_enabled(&self, app: &AppHandle, enabled: bool) {
        let _ = self.0.app.set(app.clone());
        if let Ok(mut state) = self.0.state.lock() {
            state.enabled(enabled);
        }
        self.wake();
    }

    pub fn appearance(&self, app: &AppHandle, appearance: CursorAppearance) {
        let _ = self.0.app.set(app.clone());
        #[cfg(debug_assertions)]
        {
            let appearance = appearance.clone();
            let _ = app.run_on_main_thread(move || native::render_frames(appearance));
        }
        if let Ok(mut state) = self.0.state.lock() {
            state.appearance = Some(appearance);
        }
        self.wake();
    }

    pub fn shutdown(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.runs.clear();
            state.marker = None;
            state.enabled(false);
        }
        self.wake();
    }

    pub fn wake(&self) {
        if let Some(app) = self.0.app.get() {
            let cursor = self.clone();
            // Main never waits for the engine. The callback reads current state, not an old
            // event payload, so a stop wins even when main-thread work was already queued.
            let _ = app.run_on_main_thread(move || native::refresh(&cursor));
        }
    }
}

#[derive(Clone)]
pub struct Session {
    cursor: Cursor,
    task: String,
    generation: u64,
    token: StopToken,
}

impl Session {
    pub fn target_app(&self, pid: i32) {
        if let Ok(mut state) = self.cursor.0.state.lock() {
            if state.runs.get(&self.task) == Some(&self.generation)
                && state
                    .marker
                    .as_ref()
                    .is_some_and(|marker| marker.task == self.task && marker.target.pid != pid)
            {
                state.marker = None;
            }
        }
        self.cursor.wake();
    }

    pub fn begin(&self, kind: Kind) -> Feedback {
        let sequence = self
            .cursor
            .0
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.begin(self, kind));
        self.cursor.wake();
        Feedback {
            session: self.clone(),
            sequence,
            kind,
        }
    }
}

#[derive(Clone)]
pub struct Feedback {
    session: Session,
    sequence: Option<u64>,
    kind: Kind,
}

impl Feedback {
    /// Moves the marker to `target`, gliding from wherever it is now. `None` (the action has
    /// no position on screen) leaves it where it is.
    pub fn at(&self, target: Option<Target>) {
        let (Some(sequence), Some(target)) = (self.sequence, target) else {
            return;
        };
        if let Ok(mut state) = self.session.cursor.0.state.lock() {
            if state.valid(&self.session, sequence) {
                let now = Instant::now();
                match state.marker.as_mut().filter(|marker| marker.sequence == sequence) {
                    Some(marker) => {
                        let tip = marker.tip(now, false);
                        if (tip.0 - target.point.0).hypot(tip.1 - target.point.1) >= 0.5 {
                            marker.from = Some(tip);
                            marker.moved = now;
                        }
                        marker.target = target;
                    }
                    None => {
                        state.marker = Some(Marker {
                            task: self.session.task.clone(),
                            generation: self.session.generation,
                            sequence,
                            token: self.session.token.clone(),
                            target,
                            kind: self.kind,
                            started: now,
                            completed: None,
                            from: None,
                            moved: now,
                            shown: now,
                        });
                    }
                }
            }
        }
        self.session.cursor.wake();
    }

    pub fn finish(&self, success: bool) {
        let Some(sequence) = self.sequence else {
            return;
        };
        if let Ok(mut state) = self.session.cursor.0.state.lock() {
            if state.valid(&self.session, sequence) {
                if success {
                    if let Some(marker) = state.marker.as_mut() {
                        marker.completed = Some(Instant::now());
                    }
                } else {
                    state.marker = None;
                }
            }
        }
        self.session.cursor.wake();
    }
}

pub fn overlay_windows() -> Vec<u32> {
    native::window_number().into_iter().collect()
}

#[derive(Debug, PartialEq)]
enum Placement {
    /// Draw directly above this window, on its layer.
    Window { number: u32, layer: i32 },
    /// The window moved by this much since the point was set; carry the marker along.
    Moved { offset: (f64, f64), frame: Rect },
    /// Not anchored to a window (the menu bar): draw at status level.
    Screen,
    /// Its window isn't on screen right now (minimized, another Space).
    Hidden,
    /// The window shrank away from the point: the marker no longer means anything.
    Gone,
}

fn placement(target: &Target, windows: &[WindowEntry]) -> Placement {
    let Some(anchor) = target.anchor else {
        return Placement::Screen;
    };
    let Some(window) = windows
        .iter()
        .find(|window| window.number == anchor.window && window.pid == target.pid)
    else {
        return Placement::Hidden;
    };
    if window.bounds.approx_eq(&anchor.frame) {
        return Placement::Window { number: window.number, layer: window.layer };
    }
    let offset = (window.bounds.x - anchor.frame.x, window.bounds.y - anchor.frame.y);
    if window.bounds.contains(target.point.0 + offset.0, target.point.1 + offset.1) {
        Placement::Moved { offset, frame: window.bounds }
    } else {
        Placement::Gone
    }
}

/// Whether the panel sits just above `window` in a front-to-back list: anything between them
/// must be the target app's own (its sheets and popovers ride above their parent).
fn stacked_above(windows: &[WindowEntry], panel: u32, window: u32, pid: i32) -> bool {
    let Some(start) = windows.iter().position(|entry| entry.number == panel) else {
        return false;
    };
    for entry in &windows[start + 1..] {
        if entry.number == window {
            return true;
        }
        if entry.pid != pid && drawn(entry, &[]) {
            return false;
        }
    }
    false
}

/// For a panel that couldn't be stacked above its window: whether nothing else covers the point.
fn uncovered(target: &Target, windows: &[WindowEntry], skip: &[u32]) -> bool {
    windows
        .iter()
        .find(|window| drawn(window, skip) && window.bounds.contains(target.point.0, target.point.1))
        .is_some_and(|window| window.pid == target.pid)
}

/// Quartz and AppKit use the same global point units; only the primary screen's Y origin
/// differs. Never multiply by a Retina scale or flip against each monitor separately.
fn appkit_point(point: (f64, f64), primary_top: f64) -> (f64, f64) {
    (point.0, primary_top - point.1)
}

/// Room around the tip for the bead trail and the click ping.
const MARGIN: f64 = 36.0;
const WIDTH: f64 = 260.0;
const HEIGHT: f64 = 88.0;

/// View uses a flipped coordinate system. Leave room for the trail and ping around the tip,
/// flip the arrow near the bottom/right edge, and keep the pill inside the display.
fn layout(point: (f64, f64), screen: Rect) -> (Rect, (f64, f64), bool, bool) {
    // Choose the label's side before clamping the panel. Clamping a right-facing panel
    // alone can put a long label over the pointer as it approaches a display edge.
    let mirror_x = point.0 + WIDTH - MARGIN > screen.x + screen.width;
    let desired_x = if mirror_x {
        point.0 - WIDTH + MARGIN
    } else {
        point.0 - MARGIN
    };
    let x = desired_x.clamp(screen.x, (screen.x + screen.width - WIDTH).max(screen.x));
    let y = (point.1 - HEIGHT + MARGIN)
        .clamp(screen.y, (screen.y + screen.height - HEIGHT).max(screen.y));
    let tip = (point.0 - x, y + HEIGHT - point.1);
    (
        Rect {
            x,
            y,
            width: WIDTH,
            height: HEIGHT,
        },
        tip,
        mirror_x,
        tip.1 > HEIGHT - 42.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settled_labels_keep_the_completed_action_without_its_contents() {
        for (kind, active, completed) in [
            (Kind::Clicking, "Clicking", "Clicked"),
            (Kind::Dragging, "Dragging", "Dragged"),
            (Kind::Scrolling, "Scrolling", "Scrolled"),
            (Kind::Typing, "Typing", "Typed"),
            (Kind::Pressing, "Pressing", "Pressed"),
            (Kind::Menu, "Opening menu", "Opened menu"),
        ] {
            assert_eq!(kind.label(false), active);
            assert_eq!(kind.label(true), completed);
        }
    }
    fn frame() -> Rect {
        Rect {
            x: 100.0,
            y: 50.0,
            width: 500.0,
            height: 400.0,
        }
    }
    fn target() -> Target {
        Target {
            pid: 42,
            anchor: Some(Anchor {
                window: 10,
                frame: frame(),
            }),
            point: (200.0, 200.0),
        }
    }
    fn session(cursor: &Cursor, task: &str) -> Session {
        cursor.0.state.lock().unwrap().start_run(task);
        cursor.session(task, StopToken::default()).unwrap()
    }
    fn marker(cursor: &Cursor) -> Option<Marker> {
        cursor.0.state.lock().unwrap().marker.clone()
    }
    #[test]
    fn stopped_and_restarted_runs_reject_old_actions() {
        let cursor = Cursor::default();
        let old = session(&cursor, "a");
        let feedback = old.begin(Kind::Clicking);
        feedback.at(Some(target()));
        cursor.end_run("a");
        let new = session(&cursor, "a");
        feedback.at(Some(target()));
        assert!(marker(&cursor).is_none());
        new.begin(Kind::Typing).at(Some(target()));
        old.begin(Kind::Clicking).at(Some(target()));
        assert_eq!(marker(&cursor).unwrap().kind, Kind::Typing);
    }
    #[test]
    fn latest_chat_owns_marker_and_old_completion_cannot_replace_it() {
        let cursor = Cursor::default();
        let a = session(&cursor, "a");
        let b = session(&cursor, "b");
        let first = a.begin(Kind::Clicking);
        first.at(Some(target()));
        let second = b.begin(Kind::Dragging);
        assert!(marker(&cursor).is_none(), "another chat's marker is retired");
        second.at(Some(target()));
        first.at(Some(target()));
        first.finish(false);
        cursor.end_run("a");
        assert_eq!(marker(&cursor).unwrap().task, "b");
        cursor.end_run("b");
        assert!(marker(&cursor).is_none());
    }
    #[test]
    fn unpositioned_actions_keep_the_place_but_failure_and_app_switch_clear_it() {
        let cursor = Cursor::default();
        let s = session(&cursor, "a");
        s.begin(Kind::Clicking).at(Some(target()));
        let keys = s.begin(Kind::Pressing);
        keys.at(None);
        let kept = marker(&cursor).unwrap();
        assert_eq!((kept.kind, kept.target.point), (Kind::Pressing, (200.0, 200.0)));
        keys.finish(true);
        assert!(marker(&cursor).unwrap().completed.is_some());
        let f = s.begin(Kind::Clicking);
        assert!(marker(&cursor).unwrap().completed.is_none());
        f.finish(false);
        assert!(marker(&cursor).is_none());
        s.begin(Kind::Typing).at(Some(target()));
        s.target_app(7);
        assert!(marker(&cursor).is_none());
    }
    #[test]
    fn a_new_point_glides_from_where_the_tip_is() {
        let cursor = Cursor::default();
        let s = session(&cursor, "a");
        s.begin(Kind::Clicking).at(Some(target()));
        assert_eq!(marker(&cursor).unwrap().from, None, "the first appearance doesn't glide");
        s.begin(Kind::Clicking).at(Some(Target { point: (300.0, 260.0), ..target() }));
        let m = marker(&cursor).unwrap();
        assert_eq!(m.from, Some((200.0, 200.0)));
        assert_eq!(m.tip(m.moved, false), (200.0, 200.0));
        assert_eq!(m.tip(m.moved, true), (300.0, 260.0));
        let later = m.moved + std::time::Duration::from_secs(1);
        assert_eq!(m.tip(later, false), (300.0, 260.0));
        assert!(m.since_landing(m.moved, false) < 0.0);
        assert!(m.since_landing(m.moved, true) >= 0.0);
    }
    #[test]
    fn glides_ease_out_and_reduced_motion_jumps() {
        let from = Some((0.0, 0.0));
        let half = glide(from, (100.0, 0.0), GLIDE / 2.0, false);
        assert!(half.0 > 50.0 && half.0 < 100.0, "ease-out is past halfway at half time");
        assert_eq!(glide(from, (100.0, 0.0), GLIDE / 2.0, true), (100.0, 0.0));
        assert_eq!(glide(from, (100.0, 0.0), GLIDE, false), (100.0, 0.0));
        assert_eq!(glide(None, (100.0, 0.0), 0.0, false), (100.0, 0.0));
    }
    #[test]
    fn cancellation_and_toggle_reject_in_flight_updates() {
        let cursor = Cursor::default();
        let s = session(&cursor, "a");
        let f = s.begin(Kind::Clicking);
        s.token.stop();
        f.at(Some(target()));
        assert!(marker(&cursor).is_none());
        let s = session(&cursor, "a");
        let f = s.begin(Kind::Dragging);
        f.at(Some(target()));
        {
            let mut state = cursor.0.state.lock().unwrap();
            state.enabled(false);
            state.enabled(true);
        }
        f.at(Some(target()));
        assert!(marker(&cursor).is_none());
        s.begin(Kind::Clicking).at(Some(target()));
        assert!(marker(&cursor).is_some());
    }
    fn entry(number: u32, pid: i32, bounds: Rect) -> WindowEntry {
        WindowEntry::new(number, pid, bounds)
    }
    #[test]
    fn targets_anchor_to_the_apps_front_window_under_the_point() {
        let main = entry(10, 42, frame());
        let popover = entry(11, 42, Rect { x: 150.0, y: 150.0, width: 100.0, height: 100.0 });
        let menu = entry(12, 42, Rect { x: 600.0, y: 60.0, width: 120.0, height: 200.0 });
        let other = entry(20, 7, Rect { x: 0.0, y: 0.0, width: 2000.0, height: 2000.0 });
        let mut invisible = entry(30, 42, Rect { x: 0.0, y: 0.0, width: 2000.0, height: 2000.0 });
        invisible.alpha = 0.0;
        let windows = [invisible, other, menu, popover, main];
        let window = |point| Target::in_window(42, point, &windows, &[]).and_then(|t| t.anchor).map(|a| a.window);
        assert_eq!(window((200.0, 200.0)), Some(11), "the popover in front of the main window");
        assert_eq!(window((400.0, 400.0)), Some(10));
        assert_eq!(window((650.0, 100.0)), Some(12), "a menu outside the main window");
        assert_eq!(window((1500.0, 1500.0)), None);
        assert_eq!(Target::in_window(42, (f64::NAN, 1.0), &windows, &[]), None);
    }
    #[test]
    fn placement_follows_moves_and_retires_when_the_point_is_lost() {
        let t = target();
        let mut window = entry(10, 42, frame());
        window.layer = 3;
        assert_eq!(placement(&t, &[window.clone()]), Placement::Window { number: 10, layer: 3 });
        assert_eq!(placement(&t, &[]), Placement::Hidden);
        assert_eq!(placement(&Target { anchor: None, ..t }, &[]), Placement::Screen);
        let mut moved = window.clone();
        moved.bounds.x += 20.0;
        moved.bounds.y -= 5.0;
        assert_eq!(placement(&t, &[moved.clone()]), Placement::Moved { offset: (20.0, -5.0), frame: moved.bounds });
        let mut shrunk = window;
        shrunk.bounds.width = 50.0;
        assert_eq!(placement(&t, &[shrunk]), Placement::Gone);

        let mut m = Marker {
            task: "a".into(),
            generation: 1,
            sequence: 1,
            token: StopToken::default(),
            target: t,
            kind: Kind::Clicking,
            started: Instant::now(),
            completed: None,
            from: Some((150.0, 150.0)),
            moved: Instant::now(),
            shown: Instant::now(),
        };
        m.shift((20.0, -5.0), moved.bounds);
        assert_eq!((m.target.point, m.from), ((220.0, 195.0), Some((170.0, 145.0))));
        assert_eq!(placement(&m.target, &[moved]), Placement::Window { number: 10, layer: 3 });
    }
    #[test]
    fn stacking_allows_only_the_apps_own_windows_between_panel_and_target() {
        let r = frame();
        let panel = entry(99, 1, r);
        let sheet = entry(11, 42, r);
        let target = entry(10, 42, r);
        let wackcode = entry(5, 1, r);
        let mut ghost = entry(6, 7, r);
        ghost.alpha = 0.0;
        assert!(stacked_above(&[panel.clone(), sheet.clone(), target.clone()], 99, 10, 42));
        assert!(stacked_above(&[wackcode.clone(), panel.clone(), ghost, target.clone()], 99, 10, 42));
        assert!(!stacked_above(&[panel.clone(), wackcode, target.clone()], 99, 10, 42));
        assert!(!stacked_above(&[target.clone(), panel.clone()], 99, 10, 42));
        assert!(!stacked_above(&[target], 99, 10, 42));
    }
    #[test]
    fn fallback_occlusion_ignores_invisible_windows_and_the_overlay() {
        let t = target();
        let window = entry(10, 42, frame());
        let overlay = entry(99, 1, frame());
        let mut ghost = entry(6, 7, frame());
        ghost.alpha = 0.0;
        let empty = entry(8, 7, Rect { x: 200.0, y: 200.0, width: 0.0, height: 0.0 });
        let popover = entry(11, 42, frame());
        let cover = entry(5, 7, frame());
        assert!(uncovered(&t, &[overlay.clone(), ghost, empty, window.clone()], &[99]));
        assert!(uncovered(&t, &[popover, window.clone()], &[]));
        assert!(!uncovered(&t, &[overlay, window.clone()], &[]));
        assert!(!uncovered(&t, &[cover, window], &[]));
        assert!(!uncovered(&t, &[], &[]));
    }
    #[test]
    fn point_conversion_and_edge_layout_preserve_the_exact_tip() {
        for screen in [
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
            },
            Rect {
                x: -1920.0,
                y: -200.0,
                width: 1920.0,
                height: 1080.0,
            },
        ] {
            for point in [
                (screen.x + 1.0, screen.y + 1.0),
                (screen.x + 700.0, screen.y + 400.0),
                (
                    screen.x + screen.width - 1.0,
                    screen.y + screen.height - 1.0,
                ),
            ] {
                let (frame, tip, _, _) = layout(point, screen);
                assert_eq!((frame.x + tip.0, frame.y + HEIGHT - tip.1), point);
                assert!(frame.x >= screen.x && frame.y >= screen.y);
                assert!(
                    frame.x + WIDTH <= screen.x + screen.width
                        && frame.y + HEIGHT <= screen.y + screen.height
                );
            }
        }
        let (_, tip, _, _) = layout((700.0, 400.0), Rect { x: 0.0, y: 0.0, width: 1440.0, height: 900.0 });
        assert_eq!(tip, (MARGIN, MARGIN), "room for the trail on every side of the tip");
        assert_eq!(appkit_point((-700.0, -200.0), 900.0), (-700.0, 1100.0));
        assert_eq!(appkit_point((1600.0, 1000.0), 900.0), (1600.0, -100.0));
    }
    #[test]
    fn labels_switch_sides_before_the_panel_is_clamped_at_a_display_edge() {
        let screen = Rect {
            x: -1440.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        };
        for point in [(-160.0, 400.0), (-40.0, 400.0), (-1.0, 400.0)] {
            let (_, tip, mirror_x, _) = layout(point, screen);
            assert!(mirror_x);
            assert!(tip.0 >= WIDTH - MARGIN);
        }
        let (_, tip, mirror_x, _) = layout((-1439.0, 400.0), screen);
        assert!(!mirror_x);
        assert!(tip.0 <= MARGIN);
    }
}
