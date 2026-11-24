//! The ladder every background action climbs: Accessibility first, its write read back, then
//! raw input to the app's process only where this table allows it. Pure, so it is unit-tested.
//!
//! Invariants:
//! - A reason not in the table stops the action: [`Reason::Other`] never falls through.
//! - Password fields, a covering panel or sheet, a resized window and controls that open a menu
//!   always stop; the answer says what to do instead.
//! - Raw mouse input (clicks, drags, wheel) is tried only in Chromium and Electron apps: the
//!   spike (lib.rs) found AppKit and SwiftUI apps ignore it in the background, so posting it there
//!   would only cost the model a picture to find nothing happened.
//! - Raw keys and text work in every app, so they may follow any AX failure that isn't a stop.

/// What the agent asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Click,
    Type,
    Key,
    Scroll,
    Drag,
}

/// How an app takes input in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    /// AppKit, SwiftUI, Catalyst: mouse events posted to the process are ignored.
    Native,
    /// Chromium and Electron: they act on mouse events posted to the process.
    Chromium,
}

/// Why the Accessibility route didn't do it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Nothing Accessibility can act on is under the point (a canvas, a custom view).
    NoElement,
    /// The element has no press action.
    NotPressable,
    /// The element takes no text through Accessibility.
    WriteUnsupported,
    /// The write was accepted but the value read back unchanged.
    WriteIgnored,
    /// Nothing Accessibility can scroll there.
    NoScrollBar,
    /// A drag has no Accessibility route.
    NoAxDrag,
    /// The target is a password field, or secure input is on.
    Secure,
    /// The element is disabled.
    Disabled,
    /// The window changed size since the picture the coordinates came from.
    Resized,
    /// The window is gone, minimized or in another Space.
    Unreachable,
    /// Another process's panel or a sheet covers the window.
    Covered,
    /// The control would open a menu, which would take the keyboard from the user's app.
    OpensMenu,
    /// Typing would replace text that is already there.
    WouldReplace,
    /// Anything else: stops.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Post raw input to the app's process.
    Raw,
    /// Stop and say why.
    Stop,
}

pub fn next(step: Step, reason: &Reason, kind: AppKind) -> Next {
    use Reason::*;
    let mouse = matches!(step, Step::Click | Step::Drag | Step::Scroll);
    match reason {
        Secure | Disabled | Resized | Unreachable | Covered | OpensMenu | WouldReplace | Other(_) => Next::Stop,
        NoElement | NotPressable | NoScrollBar | NoAxDrag if mouse => {
            if kind == AppKind::Chromium { Next::Raw } else { Next::Stop }
        }
        WriteUnsupported | WriteIgnored if step == Step::Type => Next::Raw,
        _ if step == Step::Key => Next::Raw,
        _ => Next::Stop,
    }
}

/// What to do about something that can't be done in the background. Full-screen control isn't
/// built yet, so this points at the other ways in; it is the one place to change once it is.
pub const ESCALATE: &str = "Try another way in: a menu command (`menu`), a keyboard shortcut (`key`), typing into a field by ref, or opening a file with computer_open. If none works, tell the user what you couldn't do in the background.";

/// The sentence for an action that stopped, saying what to do instead.
pub fn advice(step: Step, reason: &Reason, app: &str) -> String {
    use Reason::*;
    let screen = ESCALATE;
    match reason {
        NoElement | NotPressable | NoScrollBar | NoAxDrag => format!("unsupported in the background: {app} doesn't take that {} without being in front. {screen}", step_word(step)),
        WriteUnsupported | WriteIgnored => format!("unsupported in the background: {app} didn't take the text. {screen}"),
        Secure => "refused: that is a password field (or a password is being typed somewhere). Ask the user to type it.".into(),
        Disabled => "refused: that element is disabled.".into(),
        Resized => "refused: the window changed size since your picture. Take a new computer_screenshot.".into(),
        Unreachable => format!("refused: {app}'s window is closed, minimized or in another Space."),
        Covered => format!("refused: a sheet or another app's panel covers {app}'s window. Look again with computer_screenshot."),
        OpensMenu => "refused: that control opens a menu, which would take the keyboard from the user's app. Use `menu` for the menu bar, or keys, instead.".into(),
        WouldReplace => "refused: typing there would replace the text already in the field. Pass `replace: true` if that is what you want.".into(),
        Other(text) => text.clone(),
    }
}

fn step_word(step: Step) -> &'static str {
    match step {
        Step::Click => "click",
        Step::Type => "text",
        Step::Key => "key",
        Step::Scroll => "scroll",
        Step::Drag => "drag",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_always_stop() {
        for reason in [Reason::Secure, Reason::Disabled, Reason::Resized, Reason::Unreachable, Reason::Covered, Reason::OpensMenu, Reason::WouldReplace, Reason::Other("x".into())] {
            for step in [Step::Click, Step::Type, Step::Key, Step::Scroll, Step::Drag] {
                for kind in [AppKind::Native, AppKind::Chromium] {
                    assert_eq!(next(step, &reason, kind), Next::Stop, "{step:?} {reason:?} {kind:?}");
                }
            }
        }
    }

    #[test]
    fn raw_mouse_input_only_in_chromium_apps() {
        for step in [Step::Click, Step::Drag, Step::Scroll] {
            assert_eq!(next(step, &Reason::NoElement, AppKind::Native), Next::Stop);
            assert_eq!(next(step, &Reason::NoElement, AppKind::Chromium), Next::Raw);
        }
        assert_eq!(next(Step::Click, &Reason::NotPressable, AppKind::Native), Next::Stop);
        assert_eq!(next(Step::Scroll, &Reason::NoScrollBar, AppKind::Chromium), Next::Raw);
    }

    #[test]
    fn text_and_keys_fall_through_to_the_process_anywhere() {
        for kind in [AppKind::Native, AppKind::Chromium] {
            assert_eq!(next(Step::Type, &Reason::WriteUnsupported, kind), Next::Raw);
            assert_eq!(next(Step::Type, &Reason::WriteIgnored, kind), Next::Raw);
            assert_eq!(next(Step::Key, &Reason::NoElement, kind), Next::Raw);
        }
        // But never over a password field or existing text.
        assert_eq!(next(Step::Type, &Reason::Secure, AppKind::Native), Next::Stop);
        assert_eq!(next(Step::Type, &Reason::WouldReplace, AppKind::Native), Next::Stop);
    }

    #[test]
    fn advice_points_at_the_way_forward() {
        assert!(advice(Step::Click, &Reason::NoElement, "Freeform").contains("another way"));
        assert!(advice(Step::Type, &Reason::WouldReplace, "Notes").contains("replace: true"));
        assert!(advice(Step::Click, &Reason::Resized, "Notes").contains("computer_screenshot"));
        assert!(advice(Step::Type, &Reason::Secure, "Notes").contains("Ask the user"));
    }
}
