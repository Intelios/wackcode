//! The agent's cursor: where the agent is acting, drawn over the app's window. It shows what
//! happened, and is never a source of input. Ported from version 2's `computer_use/cursor.rs`.
//!
//! Invariants:
//! - Only one marker exists, app-wide: the latest action owns it, and another chat's marker
//!   is retired.
//! - Run generations and action sequences are checked again on every update, so a stopped run,
//!   or an action that has been replaced, can't bring its marker back.
//! - The marker sits directly above the app window under its point (`native::cursor`), so
//!   whatever covers the window covers the cursor too. It keeps its place across actions
//!   that have no position (keys, menus), and follows its window when it moves.
//! - Its label is the action's `summary`, which the model writes for the user. Text that was
//!   typed never appears on it.
//! - Colours come from the app's theme (`Computer::set_cursor_appearance`); Reduce Motion
//!   jumps instead of gliding.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use tokio_util::sync::CancellationToken;

use crate::geometry::{Rect, WindowEntry};

/// An sRGB colour, each channel 0 to 1.
pub type Rgb = [f64; 3];

/// The theme's colours for the cursor, as `wack-ui` resolves them.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub accent: Rgb,
    /// The arrow's contrasting edge.
    pub outline: Rgb,
    /// The label's ground.
    pub surface: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
}

impl Default for Appearance {
    /// The default theme, for an action before the app has sent its own.
    fn default() -> Self {
        let hex = |value: u32| [((value >> 16) & 0xff) as f64 / 255.0, ((value >> 8) & 0xff) as f64 / 255.0, (value & 0xff) as f64 / 255.0];
        Self { accent: hex(0xc2ee4a), outline: hex(0x111310), surface: hex(0x1c1f1a), text: hex(0xecefe6), muted: hex(0x9ba394) }
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
    /// What the label says when the model gave no summary.
    pub fn label(self, completed: bool) -> &'static str {
        match (self, completed) {
            (Self::Clicking, false) => "Clicking",
            (Self::Dragging, false) => "Dragging",
            (Self::Scrolling, false) => "Scrolling",
            (Self::Typing, false) => "Typing",
            (Self::Pressing, false) => "Pressing",
            (Self::Menu, false) => "Choosing",
            (Self::Clicking, true) => "Clicked",
            (Self::Dragging, true) => "Dragged",
            (Self::Scrolling, true) => "Scrolled",
            (Self::Typing, true) => "Typed",
            (Self::Pressing, true) => "Pressed",
            (Self::Menu, true) => "Chose",
        }
    }
}

/// The longest summary the label shows; the panel trims it further to fit.
const SUMMARY_LIMIT: usize = 48;

/// The app window a marker sits above, as the window list reported it when the point was set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub window: u32,
    pub frame: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub pid: i32,
    pub anchor: Anchor,
    pub point: (f64, f64),
}

impl Target {
    /// Anchors `point` to the front-most drawn window of `pid` containing it: the main window,
    /// or a menu, popover, sheet or panel of the same app in front of it.
    pub fn in_window(pid: i32, point: (f64, f64), windows: &[WindowEntry], skip: &[u32]) -> Option<Self> {
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        let window = windows.iter().find(|window| window.pid == pid && drawn(window, skip) && window.bounds.contains(point.0, point.1))?;
        Some(Self { pid, anchor: Anchor { window: window.number, frame: window.bounds }, point })
    }
}

/// A window that actually paints: on-screen lists include transparent and empty ones.
pub(crate) fn drawn(window: &WindowEntry, skip: &[u32]) -> bool {
    !skip.contains(&window.number) && window.alpha > 0.0 && window.bounds.has_area()
}

/// Seconds the tip takes to glide to a new point.
pub(crate) const GLIDE: f64 = 0.22;

pub(crate) fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The tip `elapsed` seconds into a glide. Reduced motion jumps straight to the target.
pub(crate) fn glide(from: Option<(f64, f64)>, to: (f64, f64), elapsed: f64, reduced: bool) -> (f64, f64) {
    match from {
        Some(from) if !reduced && elapsed < GLIDE => {
            let t = ease_out(elapsed / GLIDE);
            (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        }
        _ => to,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Marker {
    pub chat: u64,
    pub generation: u64,
    pub sequence: u64,
    pub token: CancellationToken,
    pub target: Target,
    pub kind: Kind,
    pub summary: Option<String>,
    pub started: Instant,
    pub completed: Option<Instant>,
    /// Where the current glide started; `None` on first appearance.
    pub from: Option<(f64, f64)>,
    /// When the target point last changed.
    pub moved: Instant,
    /// When the marker first appeared (its fade-in).
    pub shown: Instant,
}

impl Marker {
    pub fn tip(&self, now: Instant, reduced: bool) -> (f64, f64) {
        glide(self.from, self.target.point, now.duration_since(self.moved).as_secs_f64(), reduced)
    }

    /// Seconds since the tip arrived at its point (negative while still gliding).
    pub fn since_landing(&self, now: Instant, reduced: bool) -> f64 {
        let elapsed = now.duration_since(self.moved).as_secs_f64();
        if self.from.is_some() && !reduced { elapsed - GLIDE } else { elapsed }
    }

    /// The window moved by `offset`: carry the point and any glide in progress with it.
    pub fn shift(&mut self, offset: (f64, f64), frame: Rect) {
        self.target.point = (self.target.point.0 + offset.0, self.target.point.1 + offset.1);
        self.from = self.from.map(|from| (from.0 + offset.0, from.1 + offset.1));
        self.target.anchor.frame = frame;
    }

    /// What the label says.
    pub fn label(&self, resting: bool) -> String {
        self.summary.clone().unwrap_or_else(|| self.kind.label(resting).to_string())
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub sequence: u64,
    generation: u64,
    /// The live run of each chat, by the chat's id: its generation.
    runs: HashMap<u64, u64>,
    pub marker: Option<Marker>,
    pub appearance: Appearance,
}

impl State {
    fn current(&self, chat: u64, generation: u64, token: &CancellationToken) -> bool {
        !token.is_cancelled() && self.runs.get(&chat) == Some(&generation)
    }

    /// Whether `marker` may still be drawn.
    pub fn live(&self, marker: &Marker) -> bool {
        self.current(marker.chat, marker.generation, &marker.token) && self.sequence == marker.sequence
    }
}

/// Asks the native panel to redraw (on the main thread).
type Wake = Box<dyn Fn(&Cursor) + Send + Sync>;

struct Shared {
    state: Mutex<State>,
    /// `None` in tests: nothing is drawn.
    wake: Option<Wake>,
}

/// The app's one agent cursor.
#[derive(Clone)]
pub struct Cursor(Arc<Shared>);

impl Cursor {
    /// A cursor that draws nothing: its state is all there is (tests, the fake desktop).
    pub fn headless() -> Self {
        Self(Arc::new(Shared { state: Mutex::new(State::default()), wake: None }))
    }

    /// A cursor whose every change calls `wake`.
    pub fn with_wake(wake: impl Fn(&Cursor) + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared { state: Mutex::new(State::default()), wake: Some(Box::new(wake)) }))
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wake(&self) {
        if let Some(wake) = &self.0.wake {
            wake(self);
        }
    }

    /// A run starts in `chat`: its actions may draw until [`Cursor::end_run`] with the returned
    /// generation.
    pub fn start_run(&self, chat: u64) -> u64 {
        let mut state = self.state();
        state.generation += 1;
        let generation = state.generation;
        state.runs.insert(chat, generation);
        generation
    }

    /// The generation of `chat`'s live run.
    pub fn run_of(&self, chat: u64) -> Option<u64> {
        self.state().runs.get(&chat).copied()
    }

    /// Whether `chat` has a run the cursor knows of.
    pub fn has_run(&self, chat: u64) -> bool {
        self.state().runs.contains_key(&chat)
    }

    /// The run ends: its marker goes. A later run of the same chat is left alone.
    pub fn end_run(&self, chat: u64, generation: u64) {
        {
            let mut state = self.state();
            if state.runs.get(&chat) == Some(&generation) {
                state.runs.remove(&chat);
            }
            if state.marker.as_ref().is_some_and(|marker| marker.chat == chat && marker.generation == generation) {
                state.marker = None;
            }
        }
        self.wake();
    }

    pub fn set_appearance(&self, appearance: Appearance) {
        let changed = {
            let mut state = self.state();
            let changed = state.appearance != appearance;
            state.appearance = appearance;
            changed
        };
        if changed {
            self.wake();
        }
    }

    pub fn appearance(&self) -> Appearance {
        self.state().appearance.clone()
    }

    /// One tool call's use of the cursor, while `chat` has a live run.
    pub fn session(&self, chat: u64, token: CancellationToken) -> Option<Session> {
        let generation = *self.state().runs.get(&chat)?;
        Some(Session { cursor: self.clone(), chat, generation, token })
    }

    /// Takes every marker down (the app is quitting, or computer use was turned off).
    pub fn clear(&self) {
        {
            let mut state = self.state();
            state.marker = None;
            state.sequence += 1;
        }
        self.wake();
    }
}

#[derive(Clone)]
pub struct Session {
    cursor: Cursor,
    chat: u64,
    generation: u64,
    token: CancellationToken,
}

impl Session {
    /// The run moved on to another app: a marker over the last one means nothing now.
    pub fn target_app(&self, pid: i32) {
        {
            let mut state = self.cursor.state();
            if state.runs.get(&self.chat) == Some(&self.generation) && state.marker.as_ref().is_some_and(|marker| marker.chat == self.chat && marker.target.pid != pid) {
                state.marker = None;
            }
        }
        self.cursor.wake();
    }

    /// An action starts: it takes the marker over. The chat's own marker stays where it is (an
    /// action with no position, like a key press, shows there); another chat's is retired.
    pub fn begin(&self, kind: Kind, summary: Option<&str>) -> Feedback {
        let summary = summary.map(str::trim).filter(|summary| !summary.is_empty()).map(|summary| {
            let mut clipped: String = summary.chars().take(SUMMARY_LIMIT).collect();
            if summary.chars().count() > SUMMARY_LIMIT {
                clipped.push('…');
            }
            clipped
        });
        let sequence = {
            let mut state = self.cursor.state();
            if state.current(self.chat, self.generation, &self.token) {
                state.sequence += 1;
                let sequence = state.sequence;
                match state.marker.as_mut() {
                    Some(marker) if marker.chat == self.chat && marker.generation == self.generation => {
                        marker.sequence = sequence;
                        marker.token = self.token.clone();
                        marker.kind = kind;
                        marker.summary = summary.clone();
                        marker.started = Instant::now();
                        marker.completed = None;
                    }
                    _ => state.marker = None,
                }
                Some(sequence)
            } else {
                None
            }
        };
        self.cursor.wake();
        Feedback { session: self.clone(), sequence, kind, summary }
    }
}

/// One action's marker.
#[derive(Clone)]
pub struct Feedback {
    session: Session,
    sequence: Option<u64>,
    kind: Kind,
    summary: Option<String>,
}

impl Feedback {
    /// Moves the marker to `target`, gliding from wherever it is now. `None` (the action has
    /// no place on screen) leaves it where it is.
    pub fn at(&self, target: Option<Target>) {
        let (Some(sequence), Some(target)) = (self.sequence, target) else { return };
        {
            let session = &self.session;
            let mut state = session.cursor.state();
            if state.current(session.chat, session.generation, &session.token) && state.sequence == sequence {
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
                            chat: session.chat,
                            generation: session.generation,
                            sequence,
                            token: session.token.clone(),
                            target,
                            kind: self.kind,
                            summary: self.summary.clone(),
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

    /// The action ended: the marker rests where it is, or goes if the action failed.
    pub fn finish(&self, success: bool) {
        let Some(sequence) = self.sequence else { return };
        {
            let session = &self.session;
            let mut state = session.cursor.state();
            if state.current(session.chat, session.generation, &session.token) && state.sequence == sequence {
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

#[derive(Debug, PartialEq)]
pub(crate) enum Placement {
    /// Draw directly above this window, on its layer.
    Window { number: u32, layer: i32 },
    /// The window moved by this much since the point was set; carry the marker along.
    Moved { offset: (f64, f64), frame: Rect },
    /// Its window isn't on screen right now (minimized, another Space).
    Hidden,
    /// The window shrank away from the point: the marker no longer means anything.
    Gone,
}

pub(crate) fn placement(target: &Target, windows: &[WindowEntry]) -> Placement {
    let anchor = target.anchor;
    let Some(window) = windows.iter().find(|window| window.number == anchor.window && window.pid == target.pid) else {
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
pub(crate) fn stacked_above(windows: &[WindowEntry], panel: u32, window: u32, pid: i32) -> bool {
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
pub(crate) fn uncovered(target: &Target, windows: &[WindowEntry], skip: &[u32]) -> bool {
    windows.iter().find(|window| drawn(window, skip) && window.bounds.contains(target.point.0, target.point.1)).is_some_and(|window| window.pid == target.pid)
}

/// Quartz and AppKit use the same global point units; only the primary screen's Y origin
/// differs. Never multiply by a Retina scale or flip against each display separately.
pub(crate) fn appkit_point(point: (f64, f64), primary_top: f64) -> (f64, f64) {
    (point.0, primary_top - point.1)
}

/// Room around the tip for the bead trail and the click ping.
pub(crate) const MARGIN: f64 = 36.0;
pub(crate) const WIDTH: f64 = 300.0;
pub(crate) const HEIGHT: f64 = 88.0;

/// The panel's frame for a tip at `point` (AppKit coordinates) on `screen`, the tip inside the
/// flipped view, and whether the arrow flips left or up: room for the trail and ping around the
/// tip, the label kept inside the display.
pub(crate) fn layout(point: (f64, f64), screen: Rect) -> (Rect, (f64, f64), bool, bool) {
    // Choose the label's side before clamping the panel. Clamping a right-facing panel alone
    // can put a long label over the pointer as it nears a display edge.
    let mirror_x = point.0 + WIDTH - MARGIN > screen.x + screen.width;
    let desired_x = if mirror_x { point.0 - WIDTH + MARGIN } else { point.0 - MARGIN };
    let x = desired_x.clamp(screen.x, (screen.x + screen.width - WIDTH).max(screen.x));
    let y = (point.1 - HEIGHT + MARGIN).clamp(screen.y, (screen.y + screen.height - HEIGHT).max(screen.y));
    let tip = (point.0 - x, y + HEIGHT - point.1);
    (Rect { x, y, width: WIDTH, height: HEIGHT }, tip, mirror_x, tip.1 > HEIGHT - 42.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_say_the_summary_or_the_kind() {
        for (kind, active, completed) in [
            (Kind::Clicking, "Clicking", "Clicked"),
            (Kind::Dragging, "Dragging", "Dragged"),
            (Kind::Scrolling, "Scrolling", "Scrolled"),
            (Kind::Typing, "Typing", "Typed"),
            (Kind::Pressing, "Pressing", "Pressed"),
            (Kind::Menu, "Choosing", "Chose"),
        ] {
            assert_eq!(kind.label(false), active);
            assert_eq!(kind.label(true), completed);
        }
        let cursor = Cursor::headless();
        let s = session(&cursor, 1);
        s.begin(Kind::Clicking, Some("  Press 7 ")).at(Some(target()));
        assert_eq!(marker(&cursor).unwrap().label(false), "Press 7");
        s.begin(Kind::Typing, Some(&"long ".repeat(40))).at(None);
        assert!(marker(&cursor).unwrap().label(false).ends_with('…'));
        s.begin(Kind::Pressing, Some(" ")).at(None);
        assert_eq!(marker(&cursor).unwrap().label(true), "Pressed");
    }

    fn frame() -> Rect {
        Rect { x: 100.0, y: 50.0, width: 500.0, height: 400.0 }
    }

    fn target() -> Target {
        Target { pid: 42, anchor: Anchor { window: 10, frame: frame() }, point: (200.0, 200.0) }
    }

    fn session(cursor: &Cursor, chat: u64) -> Session {
        if !cursor.has_run(chat) {
            cursor.start_run(chat);
        }
        cursor.session(chat, CancellationToken::new()).unwrap()
    }

    fn marker(cursor: &Cursor) -> Option<Marker> {
        cursor.state().marker.clone()
    }

    #[test]
    fn stopped_and_restarted_runs_reject_old_actions() {
        let cursor = Cursor::headless();
        let first = cursor.start_run(1);
        let old = cursor.session(1, CancellationToken::new()).unwrap();
        let feedback = old.begin(Kind::Clicking, None);
        feedback.at(Some(target()));
        cursor.end_run(1, first);
        assert!(marker(&cursor).is_none());
        cursor.start_run(1);
        let new = cursor.session(1, CancellationToken::new()).unwrap();
        feedback.at(Some(target()));
        assert!(marker(&cursor).is_none());
        new.begin(Kind::Typing, None).at(Some(target()));
        old.begin(Kind::Clicking, None).at(Some(target()));
        assert_eq!(marker(&cursor).unwrap().kind, Kind::Typing);
        // Ending the old run again leaves the new one's marker.
        cursor.end_run(1, first);
        assert!(marker(&cursor).is_some());
    }

    #[test]
    fn the_latest_chat_owns_the_marker_and_an_old_completion_cannot_replace_it() {
        let cursor = Cursor::headless();
        let a_run = cursor.start_run(1);
        let b_run = cursor.start_run(2);
        let a = cursor.session(1, CancellationToken::new()).unwrap();
        let b = cursor.session(2, CancellationToken::new()).unwrap();
        let first = a.begin(Kind::Clicking, None);
        first.at(Some(target()));
        let second = b.begin(Kind::Dragging, None);
        assert!(marker(&cursor).is_none(), "another chat's marker is retired");
        second.at(Some(target()));
        first.at(Some(target()));
        first.finish(false);
        cursor.end_run(1, a_run);
        assert_eq!(marker(&cursor).unwrap().chat, 2);
        cursor.end_run(2, b_run);
        assert!(marker(&cursor).is_none());
    }

    #[test]
    fn unpositioned_actions_keep_the_place_but_failure_and_an_app_switch_clear_it() {
        let cursor = Cursor::headless();
        let s = session(&cursor, 1);
        s.begin(Kind::Clicking, None).at(Some(target()));
        let keys = s.begin(Kind::Pressing, None);
        keys.at(None);
        let kept = marker(&cursor).unwrap();
        assert_eq!((kept.kind, kept.target.point), (Kind::Pressing, (200.0, 200.0)));
        keys.finish(true);
        assert!(marker(&cursor).unwrap().completed.is_some());
        let failed = s.begin(Kind::Clicking, None);
        assert!(marker(&cursor).unwrap().completed.is_none());
        failed.finish(false);
        assert!(marker(&cursor).is_none());
        s.begin(Kind::Typing, None).at(Some(target()));
        s.target_app(7);
        assert!(marker(&cursor).is_none());
    }

    #[test]
    fn a_new_point_glides_from_where_the_tip_is() {
        let cursor = Cursor::headless();
        let s = session(&cursor, 1);
        s.begin(Kind::Clicking, None).at(Some(target()));
        assert_eq!(marker(&cursor).unwrap().from, None, "the first appearance doesn't glide");
        s.begin(Kind::Clicking, None).at(Some(Target { point: (300.0, 260.0), ..target() }));
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
    fn stopping_and_clearing_reject_updates_in_flight() {
        let cursor = Cursor::headless();
        cursor.start_run(1);
        let token = CancellationToken::new();
        let s = cursor.session(1, token.clone()).unwrap();
        let feedback = s.begin(Kind::Clicking, None);
        token.cancel();
        feedback.at(Some(target()));
        assert!(marker(&cursor).is_none());
        let s = session(&cursor, 1);
        let feedback = s.begin(Kind::Dragging, None);
        feedback.at(Some(target()));
        cursor.clear();
        feedback.at(Some(target()));
        assert!(marker(&cursor).is_none());
        s.begin(Kind::Clicking, None).at(Some(target()));
        assert!(marker(&cursor).is_some());
        assert!(cursor.session(9, CancellationToken::new()).is_none(), "a chat with no run draws nothing");
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
        let window = |point| Target::in_window(42, point, &windows, &[]).map(|t| t.anchor.window);
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
        assert_eq!(placement(&t, std::slice::from_ref(&window)), Placement::Window { number: 10, layer: 3 });
        assert_eq!(placement(&t, &[]), Placement::Hidden);
        let mut moved = window.clone();
        moved.bounds.x += 20.0;
        moved.bounds.y -= 5.0;
        assert_eq!(placement(&t, std::slice::from_ref(&moved)), Placement::Moved { offset: (20.0, -5.0), frame: moved.bounds });
        let mut shrunk = window;
        shrunk.bounds.width = 50.0;
        assert_eq!(placement(&t, &[shrunk]), Placement::Gone);

        let now = Instant::now();
        let mut m = Marker {
            chat: 1,
            generation: 1,
            sequence: 1,
            token: CancellationToken::new(),
            target: t,
            kind: Kind::Clicking,
            summary: None,
            started: now,
            completed: None,
            from: Some((150.0, 150.0)),
            moved: now,
            shown: now,
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
        assert!(stacked_above(&[panel.clone(), sheet, target.clone()], 99, 10, 42));
        assert!(stacked_above(&[wackcode.clone(), panel.clone(), ghost, target.clone()], 99, 10, 42));
        assert!(!stacked_above(&[panel.clone(), wackcode, target.clone()], 99, 10, 42));
        assert!(!stacked_above(&[target.clone(), panel], 99, 10, 42));
        assert!(!stacked_above(&[target], 99, 10, 42));
    }

    #[test]
    fn the_fallback_looks_through_invisible_windows_and_the_overlay() {
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
    fn point_conversion_and_edge_layout_keep_the_exact_tip() {
        for screen in [Rect { x: 0.0, y: 0.0, width: 1440.0, height: 900.0 }, Rect { x: -1920.0, y: -200.0, width: 1920.0, height: 1080.0 }] {
            for point in [(screen.x + 1.0, screen.y + 1.0), (screen.x + 700.0, screen.y + 400.0), (screen.x + screen.width - 1.0, screen.y + screen.height - 1.0)] {
                let (frame, tip, _, _) = layout(point, screen);
                assert_eq!((frame.x + tip.0, frame.y + HEIGHT - tip.1), point);
                assert!(frame.x >= screen.x && frame.y >= screen.y);
                assert!(frame.x + WIDTH <= screen.x + screen.width && frame.y + HEIGHT <= screen.y + screen.height);
            }
        }
        let (_, tip, _, _) = layout((700.0, 400.0), Rect { x: 0.0, y: 0.0, width: 1440.0, height: 900.0 });
        assert_eq!(tip, (MARGIN, MARGIN), "room for the trail on every side of the tip");
        assert_eq!(appkit_point((-700.0, -200.0), 900.0), (-700.0, 1100.0));
        assert_eq!(appkit_point((1600.0, 1000.0), 900.0), (1600.0, -100.0));
    }

    #[test]
    fn labels_switch_sides_before_the_panel_is_clamped_at_a_display_edge() {
        let screen = Rect { x: -1440.0, y: 0.0, width: 1440.0, height: 900.0 };
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
