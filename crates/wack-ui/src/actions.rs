//! Everything the user can ask the app to do, as GPUI actions.
//!
//! Views dispatch these from clicks and key bindings; the shell handles them by calling
//! [`crate::app::AppState`]. A view therefore needs no handle on the app, and in a gallery scene,
//! where nothing handles them, clicks simply do nothing.

use gpui::{Action, App, KeyBinding, actions};
use wack_core::{Effort, WireFormat};

use crate::panes::{Dock, PaneId, Target, dock::PANE_CONTEXT, terminal::TERMINAL_CONTEXT};

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
        ToggleSidebar,
        /// Shows the chat's Changes pane, or hides its dock when it is in front.
        ToggleChanges,
        /// Shows the chat's terminal, or hides its dock when it is in front.
        ToggleTerminal,
        NewTerminal,
        /// Closes the pane that has focus.
        ClosePane,
        /// Closes the chat tab in front; the chat itself stays in the sidebar.
        CloseTab,
        NextTab,
        PreviousTab,
        /// Esc typed into a terminal, which goes to the shell rather than stepping back.
        TerminalEscape,
        /// Picks a picture for the backdrop.
        ChooseBackdrop,
        RemoveBackdrop,
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

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct CloseChatTab {
    pub id: String,
}

/// Brings a pane to the front of its stack.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ActivatePane {
    pub pane: PaneId,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ClosePaneId {
    pub pane: PaneId,
}

/// A dragged tab was dropped.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct MovePane {
    pub pane: PaneId,
    pub target: Target,
}

/// A new terminal as a tab beside this pane.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct NewTerminalWith {
    pub pane: PaneId,
}

/// Back to the dock's size before the user dragged it.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ResetDockSize {
    pub dock: Dock,
}

/// Gives the two stacks either side of a divider equal shares again.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct EqualizeSplit {
    pub dock: Dock,
    pub path: Vec<usize>,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenSettingsSection {
    pub section: crate::settings::Section,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseMode {
    pub mode: wack_core::Mode,
}

/// `None` goes back to the default.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseThemeColour {
    pub hex: Option<String>,
}

/// `None` goes back to the default.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseAccent {
    pub hex: Option<String>,
}

/// How far the backdrop is toned towards the window's colour, in percent.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseVeil {
    pub veil: u8,
}

/// The part of the backdrop kept in view: 0, 1 or 2 across and down.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseFocus {
    pub across: u8,
    pub down: u8,
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
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-g", ToggleChanges, Some(CONTEXT)),
        KeyBinding::new("ctrl-`", ToggleTerminal, Some(CONTEXT)),
        // macOS may report ⌃⇧` as either.
        KeyBinding::new("ctrl-shift-`", NewTerminal, Some(CONTEXT)),
        KeyBinding::new("ctrl-~", NewTerminal, Some(CONTEXT)),
        KeyBinding::new("cmd-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("cmd-w", ClosePane, Some(PANE_CONTEXT)),
        KeyBinding::new("cmd-}", NextTab, Some(CONTEXT)),
        KeyBinding::new("cmd-{", PreviousTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(CONTEXT)),
        KeyBinding::new("escape", TerminalEscape, Some(TERMINAL_CONTEXT)),
    ]);
}

/// The key context the shell's root sets, so the bindings above apply anywhere in the window.
pub fn context() -> &'static str {
    CONTEXT
}
