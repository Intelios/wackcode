//! A visual account of dispatched computer actions, never an input source. Only one marker
//! exists: the latest action owns it, even when another chat has a marker to leave behind.
//! Run generations and action sequences are checked again on the main thread, so queued work
//! cannot resurrect a stopped run or a superseded action. No AX object crosses this boundary.

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pid: i32,
    pub window: u32,
    pub frame: Rect,
    pub point: (f64, f64),
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

    fn begin(&mut self, session: &Session) -> Option<u64> {
        if !self.enabled
            || session.token.is_stopped()
            || self.runs.get(&session.task) != Some(&session.generation)
        {
            return None;
        }
        self.sequence += 1;
        self.marker = None;
        Some(self.sequence)
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
            .and_then(|mut state| state.begin(self));
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
    pub fn at(&self, target: Option<Target>) {
        let Some(sequence) = self.sequence else {
            return;
        };
        if let Ok(mut state) = self.session.cursor.0.state.lock() {
            if state.valid(&self.session, sequence) {
                let started = state
                    .marker
                    .as_ref()
                    .map(|marker| marker.started)
                    .unwrap_or_else(Instant::now);
                state.marker = target.map(|target| Marker {
                    task: self.session.task.clone(),
                    generation: self.session.generation,
                    sequence,
                    token: self.session.token.clone(),
                    target,
                    kind: self.kind,
                    started,
                    completed: None,
                });
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
enum Visibility {
    Visible,
    Hidden,
    Invalid,
}

fn visibility(target: Target, windows: &[WindowEntry], skip: &[u32]) -> Visibility {
    let Some(window) = windows
        .iter()
        .find(|window| window.number == target.window && window.pid == target.pid)
    else {
        return Visibility::Hidden;
    };
    if !window.bounds.approx_eq(&target.frame) {
        return Visibility::Invalid;
    }
    match windows.iter().find(|window| {
        !skip.contains(&window.number) && window.bounds.contains(target.point.0, target.point.1)
    }) {
        Some(window) if window.pid == target.pid && window.number == target.window => {
            Visibility::Visible
        }
        _ => Visibility::Hidden,
    }
}

/// Quartz and AppKit use the same global point units; only the primary screen's Y origin
/// differs. Never multiply by a Retina scale or flip against each monitor separately.
fn appkit_point(point: (f64, f64), primary_top: f64) -> (f64, f64) {
    (point.0, primary_top - point.1)
}

const WIDTH: f64 = 224.0;
const HEIGHT: f64 = 64.0;

/// View uses a flipped coordinate system. Leave room for the click halo around the tip,
/// flip the arrow near the bottom/right edge, and keep the pill inside the display.
fn layout(point: (f64, f64), screen: Rect) -> (Rect, (f64, f64), bool, bool) {
    // Choose the label's side before clamping the panel. Clamping a right-facing panel
    // alone can put a long label over the pointer as it approaches a display edge.
    let mirror_x = point.0 + WIDTH - 16.0 > screen.x + screen.width;
    let desired_x = if mirror_x {
        point.0 - WIDTH + 16.0
    } else {
        point.0 - 16.0
    };
    let x = desired_x.clamp(screen.x, (screen.x + screen.width - WIDTH).max(screen.x));
    let y = (point.1 - HEIGHT + 16.0)
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
        tip.1 > HEIGHT - 34.0,
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
    fn target() -> Target {
        Target {
            pid: 42,
            window: 10,
            frame: Rect {
                x: 100.0,
                y: 50.0,
                width: 500.0,
                height: 400.0,
            },
            point: (200.0, 200.0),
        }
    }
    fn session(cursor: &Cursor, task: &str) -> Session {
        cursor.0.state.lock().unwrap().start_run(task);
        cursor.session(task, StopToken::default()).unwrap()
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
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
        new.begin(Kind::Typing).at(Some(target()));
        old.begin(Kind::Clicking).at(Some(target()));
        assert_eq!(
            cursor.0.state.lock().unwrap().marker.as_ref().unwrap().kind,
            Kind::Typing
        );
    }
    #[test]
    fn latest_chat_owns_marker_and_old_completion_cannot_replace_it() {
        let cursor = Cursor::default();
        let a = session(&cursor, "a");
        let b = session(&cursor, "b");
        let first = a.begin(Kind::Clicking);
        first.at(Some(target()));
        let second = b.begin(Kind::Dragging);
        second.at(Some(target()));
        first.at(Some(target()));
        first.finish(false);
        cursor.end_run("a");
        assert_eq!(
            cursor.0.state.lock().unwrap().marker.as_ref().unwrap().task,
            "b"
        );
        cursor.end_run("b");
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
    }
    #[test]
    fn failure_unpositioned_actions_and_app_switch_clear_marker() {
        let cursor = Cursor::default();
        let s = session(&cursor, "a");
        let f = s.begin(Kind::Clicking);
        f.at(Some(target()));
        f.finish(false);
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
        s.begin(Kind::Typing).at(Some(target()));
        s.target_app(7);
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
        s.begin(Kind::Typing).at(Some(target()));
        s.begin(Kind::Pressing);
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
    }
    #[test]
    fn cancellation_and_toggle_reject_in_flight_updates() {
        let cursor = Cursor::default();
        let s = session(&cursor, "a");
        let f = s.begin(Kind::Clicking);
        s.token.stop();
        f.at(Some(target()));
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
        let s = session(&cursor, "a");
        let f = s.begin(Kind::Dragging);
        f.at(Some(target()));
        {
            let mut state = cursor.0.state.lock().unwrap();
            state.enabled(false);
            state.enabled(true);
        }
        f.at(Some(target()));
        assert!(cursor.0.state.lock().unwrap().marker.is_none());
        s.begin(Kind::Clicking).at(Some(target()));
        assert!(cursor.0.state.lock().unwrap().marker.is_some());
    }
    #[test]
    fn covered_windows_hide_but_movement_invalidates() {
        let t = target();
        let window = WindowEntry {
            number: t.window,
            pid: t.pid,
            bounds: t.frame,
        };
        let overlay = WindowEntry {
            number: 99,
            pid: 1,
            bounds: t.frame,
        };
        assert_eq!(
            visibility(t, &[overlay.clone(), window.clone()], &[99]),
            Visibility::Visible
        );
        assert_eq!(
            visibility(t, &[overlay, window.clone()], &[]),
            Visibility::Hidden
        );
        assert_eq!(visibility(t, &[], &[]), Visibility::Hidden);
        assert_eq!(visibility(t, &[window.clone()], &[]), Visibility::Visible);
        let mut moved = window;
        moved.bounds.x += 20.0;
        assert_eq!(visibility(t, &[moved], &[]), Visibility::Invalid);
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
            assert!(tip.0 >= WIDTH - 16.0);
        }
        let (_, tip, mirror_x, _) = layout((-1439.0, 400.0), screen);
        assert!(!mirror_x);
        assert!(tip.0 <= 16.0);
    }
}
