//! Full-screen control: what the agent does with the user's real pointer and keyboard, once the
//! user lets it take the screen (`ChatComputer::screen_access`).
//!
//! Invariants:
//! - One chat at a time holds the screen, for one run; the user gets it back when the run ends
//!   or is stopped, when they move the mouse or press a key, or with ⌃⌥⌘. (which also stops
//!   the run). Taken back, it isn't asked for again before the user's next message.
//! - Every event is gated just before it is posted ([`Go`]): the screen is still held, no
//!   password field has the keyboard, and the point is on an allowed app's window, or the app
//!   is in front for keys. A failed gate stops the input and lets go of anything held down.
//! - Typing checks before every character, so text never lands in an app the user switched to.

use std::{collections::HashMap, sync::Arc};

use crate::{
    backend::Button,
    geometry::{self, HitOwner, WindowEntry},
    keys::{Chord, Modifiers},
};

/// Why the agent let go of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The user moved the mouse, clicked, scrolled or pressed a key.
    UserInput,
    /// The user pressed ⌃⌥⌘.: the run stops too.
    StopChord,
    /// The run ended or was stopped.
    RunEnded,
    /// Computer use was turned off, or the app is quitting.
    TurnedOff,
}

impl Ended {
    /// What the model is told when it next reaches for the screen.
    pub fn words(self) -> &'static str {
        match self {
            Ended::UserInput => "The user took the screen back by using their mouse or keyboard.",
            Ended::StopChord => "The user pressed ⌃⌥⌘. to take the screen back and stop.",
            Ended::RunEnded => "The screen was given back when the last run ended.",
            Ended::TurnedOff => "Computer use was turned off.",
        }
    }
}

/// Asked before every event full-screen control posts: `Err` says why it must stop.
pub type Go = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Input through the HID tap: the real pointer and keyboard. Points are global, in points.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenInput {
    Click { point: (f64, f64), button: Button, count: u32, modifiers: Modifiers },
    Type { text: String },
    Key { chord: Chord, times: u32 },
    Scroll { point: (f64, f64), dx: f64, dy: f64 },
    Drag { path: Vec<(f64, f64)> },
}

impl ScreenInput {
    /// Where the pointer must land on the app's window; `None` for keys and typing, which go to
    /// the app in front.
    pub fn point(&self) -> Option<(f64, f64)> {
        match self {
            ScreenInput::Click { point, .. } | ScreenInput::Scroll { point, .. } => Some(*point),
            ScreenInput::Drag { path } => path.first().copied(),
            ScreenInput::Type { .. } | ScreenInput::Key { .. } => None,
        }
    }
}

/// Whether input at `point` would land on `pid`'s own window: anything else on top (another
/// app, WackCode itself, nothing at all) refuses it, saying what is in the way.
pub fn gate_point(windows: &[WindowEntry], point: (f64, f64), pid: i32, own_pid: i32, app: &str, names: &HashMap<i32, String>) -> Result<(), String> {
    let describe = |other: i32| (names.get(&other).cloned().unwrap_or_else(|| "another app".to_string()), false);
    match geometry::topmost_owner(windows, point, pid, own_pid, &[], describe) {
        HitOwner::Target => Ok(()),
        HitOwner::Own => Err(format!("WackCode's own window is over that point, not {app}'s.")),
        HitOwner::Blocked(name) | HitOwner::Other(name) => Err(format!("{name}'s window is over that point, not {app}'s.")),
        HitOwner::Nothing => Err(format!("No window is at that point: it is outside {app}'s windows.")),
    }
}

/// Whether a key event is the user's stop chord, ⌃⌥⌘ and the key that types `.`.
pub fn is_stop_chord(key_code: u16, command: bool, control: bool, option: bool, period: u16) -> bool {
    key_code == period && command && control && option
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Rect;

    #[test]
    fn input_lands_only_on_the_apps_own_window() {
        let app = WindowEntry::new(10, 42, Rect::new(0.0, 0.0, 500.0, 500.0));
        let notes = WindowEntry::new(20, 7, Rect::new(300.0, 300.0, 400.0, 400.0));
        let wackcode = WindowEntry::new(30, 1, Rect::new(0.0, 0.0, 100.0, 100.0));
        // WackCode's glow covers every display, and is looked through.
        let glow = WindowEntry { layer: 101, overlay: true, ..WindowEntry::new(40, 1, Rect::new(0.0, 0.0, 2000.0, 2000.0)) };
        let windows = [glow, wackcode, notes, app];
        let names = HashMap::from([(7, "Notes".to_string())]);
        assert!(gate_point(&windows, (200.0, 200.0), 42, 1, "TextEdit", &names).is_ok());
        assert!(gate_point(&windows, (350.0, 350.0), 42, 1, "TextEdit", &names).unwrap_err().starts_with("Notes's window"));
        assert!(gate_point(&windows, (50.0, 50.0), 42, 1, "TextEdit", &names).unwrap_err().starts_with("WackCode's own window"));
        assert!(gate_point(&windows, (900.0, 50.0), 42, 1, "TextEdit", &names).unwrap_err().starts_with("No window"));
    }

    #[test]
    fn the_stop_chord_needs_all_three_modifiers() {
        assert!(is_stop_chord(47, true, true, true, 47));
        assert!(!is_stop_chord(47, true, true, false, 47));
        assert!(!is_stop_chord(46, true, true, true, 47));
    }
}
