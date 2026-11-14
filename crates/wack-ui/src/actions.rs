//! Everything the user can ask the app to do, as GPUI actions.
//!
//! Views dispatch these from clicks and key bindings; the shell handles them by calling
//! [`crate::app::AppState`]. A view therefore needs no handle on the app, and in a gallery scene,
//! where nothing handles them, clicks simply do nothing.

use gpui::{Action, App, KeyBinding, actions};
use wack_core::{Effort, WireFormat};

actions!(
    wackcode,
    [
        /// A new chat in the current project.
        NewChat,
        /// Pick a folder to add as a project.
        AddFolder,
        OpenSettings,
        CloseSettings,
        /// Send the composer's text, or queue it behind the run.
        Send,
        /// Stop the run in the current chat.
        Stop,
        /// Summarise the current chat's older messages now.
        Compact,
        /// Step back: closes a menu or Settings, or stops the run.
        Back,
        /// Adds the connection the Settings form describes.
        AddConnection,
        OpenModelMenu,
        OpenEffortMenu,
        CloseMenu,
        /// Moves focus to the next or previous field of a form.
        FocusNext,
        FocusPrevious,
    ]
);

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseFormat {
    pub format: WireFormat,
}

/// Asks first.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveConnection {
    pub id: String,
}

/// The model for the current chat, or for new chats on the new-chat screen.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseModel {
    pub connection: String,
    pub model: String,
}

/// `None` leaves the effort to the provider.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseEffort {
    pub effort: Option<Effort>,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SelectChat {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct NewChatIn {
    pub project: String,
}

/// Lists or hides a project's chats in the sidebar.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleProject {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct PinProject {
    pub id: String,
    pub pinned: bool,
}

/// Asks first; the folder itself is never touched.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveProject {
    pub id: String,
}

/// Asks first.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct DeleteChat {
    pub id: String,
}

/// Takes a queued message back into the composer.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct Unqueue {
    pub index: usize,
}

const CONTEXT: &str = "Shell";
/// The key context of a form whose fields Tab moves between.
pub const FORM_CONTEXT: &str = "Form";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-n", NewChat, Some(CONTEXT)),
        KeyBinding::new("cmd-o", AddFolder, Some(CONTEXT)),
        KeyBinding::new("cmd-,", OpenSettings, Some(CONTEXT)),
        KeyBinding::new("escape", Back, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(FORM_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(FORM_CONTEXT)),
    ]);
}

/// The key context the shell's root sets, so the bindings above apply anywhere in the window.
pub fn context() -> &'static str {
    CONTEXT
}
