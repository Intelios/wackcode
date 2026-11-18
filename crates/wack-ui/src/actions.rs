//! Everything the user can ask the app to do, as GPUI actions.
//!
//! Views dispatch these from clicks and key bindings; the shell handles them by calling
//! [`crate::app::AppState`]. A view therefore needs no handle on the app, and in a gallery scene,
//! where nothing handles them, clicks simply do nothing.

use gpui::{Action, App, KeyBinding, actions};
use wack_core::{AgentMode, CommandKind, Reasoning, WireFormat};

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
        /// Move the command the agent is waiting on to the background (⌘B).
        MoveToBackground,
        /// Open or fold the list of background commands above the composer.
        ToggleTasks,
        /// Show the Tasks pane.
        ToggleTasksPane,
        /// Show the Agents pane: the chat's sub-agents and their work.
        ToggleAgentsPane,
        /// Show the Browser pane: the chat's browser tabs.
        ToggleBrowserPane,
        /// Summarise the current chat's older messages now.
        Compact,
        /// Step back: closes a menu or Settings, or stops the run.
        Back,
        /// Settings › Providers: save the connection the editor describes.
        SaveConnection,
        /// Settings › Providers: puts a saved connection's fields back as they are stored.
        DiscardConnection,
        /// Settings › Providers: show the editor for a new connection.
        OpenConnectionForm,
        /// Settings › Providers: back to the connections list, forgetting the editor's changes.
        CloseConnectionForm,
        /// Settings › Providers: adds a blank model row to the connection editor.
        AddModelRow,
        /// Settings › Providers: shows the key in the editor's key field, or hides it again.
        ToggleConnectionKey,
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
        /// ⇧Tab typed into a terminal, which goes to the shell rather than switching mode.
        TerminalBackTab,
        /// ⇧Tab: Build → Plan → Ultra Plan → Build, for the chat on screen or the new chat.
        CycleAgentMode,
        /// The question card: the next question, or Submit on the last.
        NextQuestion,
        PreviousQuestion,
        SubmitAnswers,
        /// Dismisses the question; the agent carries on without an answer.
        CancelQuestion,
        /// Ultra Plan's "Write the plan now".
        WrapUpQuestion,
        /// The plan awaiting review: switch to Build and implement it.
        ApprovePlan,
        /// Writes the plan awaiting review to PLAN.md in the project.
        SavePlan,
        CopyPlan,
        /// Asks to discard the plan awaiting review; `ConfirmDiscardPlan` does it.
        DiscardPlan,
        ConfirmDiscardPlan,
        /// Keeps the plan after all.
        KeepPlan,
        /// Shows the todo list's folded tasks.
        ShowDoneTodos,
        /// Opens or folds the todo list from its status pill.
        ToggleTodos,
        /// Opens or folds the waiting messages from their status pill.
        ToggleWaiting,
        /// Settings › Tools: turns unrestricted planning off, or asks before turning it on.
        ToggleUnrestrictedPlanning,
        ConfirmUnrestrictedPlanning,
        CancelUnrestrictedPlanning,
        /// Settings › Tools and Settings › Web: turns Web Fetch on or off.
        ToggleWebFetch,
        /// Settings › Web: the Web Search switch.
        ToggleWebSearch,
        /// Settings › Web: reading pages through the active search provider.
        ToggleRouteFetch,
        /// Settings › Web: back to the overview from a provider's editor.
        CloseSearchEditor,
        SaveSearchProvider,
        /// Settings › Web: runs the editor's draft as it stands.
        TestSearchProvider,
        AskDeleteSearchProvider,
        ConfirmDeleteSearchProvider,
        CancelDeleteSearchProvider,
        /// Settings › Web: clears the usage meter's counts.
        ResetWebUsage,
        /// Settings › MCP servers: back to the overview from the editor.
        CloseMcpEditor,
        SaveMcpServer,
        /// Settings › MCP servers: connects to the editor's draft as it stands and lists its tools.
        TestMcpDraft,
        /// Settings › MCP servers: a new variable (command) or header (URL) in the editor.
        AddMcpEntry,
        ConfirmDeleteMcpServer,
        CancelDeleteMcpServer,
        /// Settings › Integrations: turns the usage ledger's recording on or off.
        ToggleUsageRecording,
        /// Settings › Git: turns commit co-author attribution on or off.
        ToggleCommitAttribution,
        /// Settings › Browser: whether the Browser pane opens when the agent opens a tab.
        ToggleAgentOpensPane,
        /// Settings › Browser: asks to clear the browsing data (a confirm follows).
        AskClearBrowsingData,
        CancelClearBrowsingData,
        /// Settings › Browser: clears it.
        ConfirmClearBrowsingData,
        /// Adds a picture to the backdrop's library, and shows it.
        ChooseBackdrop,
        /// Settings › Memory: the master switch.
        ToggleMemory,
        /// Settings › Memory: saves the note the editor describes.
        SaveMemory,
        /// Settings › Memory: back to the list, forgetting the editor's changes.
        CloseMemoryEditor,
        /// Settings › Memory: moves the note or folder waiting for confirmation to the Trash.
        ConfirmMemoryDelete,
        CancelMemoryDelete,
        /// Settings › Commands: a new command of the user's own.
        NewSlashCommand,
        /// Settings › Commands: saves the command the editor describes.
        SaveSlashCommand,
        /// Settings › Commands: back to the list.
        CloseSlashCommand,
        /// Settings › Commands: asks before the command in the editor moves to the Trash.
        AskDeleteSlashCommand,
        ConfirmSlashCommandDelete,
        CancelSlashCommandDelete,
        /// Settings › Skills: a new skill in Your skills.
        NewSkill,
        /// Settings › Skills: saves the skill the editor describes.
        SaveSkill,
        /// Settings › Skills: back to the list from the editor or a skill on view.
        CloseSkill,
        /// Settings › Skills: asks before the skill in the editor moves to the Trash.
        AskDeleteSkill,
        ConfirmSkillDelete,
        CancelSkillDelete,
        /// Settings › Skills: adds a folder to load skills from.
        AddSkillFolder,
        /// Settings › Skills: imports skills from a folder, a .md file or a .zip.
        ImportSkills,
        /// Dismisses the Build/Run command configuration popover.
        CloseCommandEditor,
        /// Settings › Sub-agents: opens or closes the helper model list.
        ToggleHelperModels,
        /// Settings › Sub-agents: a new agent of the user's own.
        NewAgent,
        /// Settings › Sub-agents: a new agent starting as a copy of the one in the editor.
        DuplicateAgent,
        /// Settings › Sub-agents: saves the agent the editor describes.
        SaveAgent,
        /// Settings › Sub-agents: back to the list.
        CloseAgent,
        /// Settings › Sub-agents: asks before the agent in the editor moves to the Trash (for a
        /// customised built-in, before the built-in is restored).
        AskDeleteAgent,
        ConfirmAgentDelete,
        CancelAgentDelete,
        /// Settings › Sub-agents › Automatic titles: the switch.
        ToggleAutoTitles,
        /// Settings › Sub-agents › Automatic titles: opens or closes the model list.
        ToggleTitleModels,
        /// Renames the chat on screen, on its tab (File › Rename Chat…).
        RenameChat,
        /// Esc in a rename field: the chat keeps its name.
        CancelRename,
    ]
);

/// A suggested-task card's start: in this chat (queued behind a run) or in a chat of its own.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct StartSuggestion {
    /// The `suggest_task` call id.
    pub id: String,
    pub target: SuggestionTarget,
    /// The mode the chat or new chat starts the task in.
    pub mode: AgentMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionTarget {
    Here,
    New,
}

/// Folds a suggested task away (or brings it back) in the chat on screen.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct DismissSuggestion {
    pub id: String,
    pub dismissed: bool,
}

/// Copies a suggested task's message to the clipboard.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct CopySuggestion {
    pub id: String,
}

/// Selects a chat by id — a suggested-task card's "Opened in" link.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenChat {
    pub id: String,
}

/// Settings › Providers › Automatic titles: the model that writes titles.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseTitleModel {
    pub connection: String,
    pub model: String,
}

/// Opens a rename field over a chat's title where it was double-clicked.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RenameChatAt {
    pub chat: String,
    /// On its sidebar row rather than its tab.
    pub sidebar: bool,
}

/// The key context of a rename field, where Esc cancels instead of stepping back.
pub const RENAME_CONTEXT: &str = "RenameField";

/// Starts, stops, or asks to configure a project Build/Run command.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct TriggerProjectCommand {
    pub kind: CommandKind,
}

/// Opens or toggles the command configuration popover for Build or Run.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenCommandEditor {
    pub kind: CommandKind,
}

/// Saves the command editor's text, optionally starting the command immediately.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SaveCommandEditor {
    pub run_after: bool,
}

/// Opens the output pane for Build or Run and brings it in front.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ShowCommandOutput {
    pub kind: CommandKind,
}

/// Clears a Build or Run pane's screen without stopping the process.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ClearCommandOutput {
    pub kind: CommandKind,
}

/// Requests an escalating stop on a running Build or Run command.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct StopCommand {
    pub kind: CommandKind,
}

/// Settings › Memory: switches one project's memory on or off.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchProjectMemory {
    pub key: String,
    pub on: bool,
}

/// Settings › Memory: the editor, for a new note in this memory folder.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct NewMemory {
    pub dir: std::path::PathBuf,
}

/// Settings › Memory: the editor, showing a saved note.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct EditMemory {
    pub dir: std::path::PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseMemoryKind {
    pub kind: wack_core::memory::Kind,
}

/// Asks before a note moves to the Trash.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AskDeleteMemory {
    pub dir: std::path::PathBuf,
    pub name: String,
}

/// Asks before a project's whole memory folder moves to the Trash.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AskRemoveMemoryFolder {
    pub key: String,
}

/// Shows a memory file or folder in Finder.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RevealMemory {
    pub path: std::path::PathBuf,
}

/// Shows the usage ledger's folder in Finder (Settings › Integrations).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RevealUsageFolder {
    pub path: std::path::PathBuf,
}

/// Shows a picture from the library behind the window, or none, bringing back the theme it
/// remembers.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ShowPicture {
    pub file: Option<String>,
}

/// Takes a picture out of the library.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemovePicture {
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseFormat {
    pub format: WireFormat,
}

/// Shows a saved connection in the Settings editor.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct EditConnection {
    pub id: String,
}

/// Takes a model row out of the connection editor.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveModelRow {
    pub index: usize,
}

/// Settings › Providers: one model's Reasoning switch in the connection editor.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleModelReasoning {
    pub index: usize,
}

/// Opens the lightbox on `pictures[index]`, with the others a step away.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenPicture {
    pub pictures: std::sync::Arc<Vec<crate::pictures::Picture>>,
    pub index: usize,
}

/// Closes the lightbox.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ClosePicture;

/// The lightbox's previous (-1) or next (1) picture.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct StepPicture {
    pub by: i32,
}

/// The lightbox between fitting the window and the picture's own size.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ZoomPicture;

/// The composer's attach button: choose pictures to send with the message.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AttachPictures;

/// Files dropped on the composer, to attach as pictures.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AttachFiles {
    pub paths: Vec<std::path::PathBuf>,
}

/// Takes one picture back off the message being written.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveAttachment {
    pub index: usize,
}

/// Settings › Providers: one model's Vision switch (whether it is sent pictures).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleModelVision {
    pub index: usize,
}

/// Settings › Providers: ticks or unticks an effort one model's endpoint accepts.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleModelEffort {
    pub index: usize,
    pub effort: Reasoning,
}

/// Switches a connection on or off. Off keeps its key and models, out of the model menu.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchConnection {
    pub id: String,
    pub on: bool,
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
    pub effort: Option<Reasoning>,
}

/// The effort slider moved: like [`ChooseEffort`], but the slider stays open.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SlideEffort {
    pub effort: Option<Reasoning>,
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

/// Settings › Skills: one skill's switch, by its key.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchSkill {
    pub key: String,
    pub on: bool,
}

/// Settings › Skills: another tool's folder switched on or off.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchSkillFolder {
    pub id: String,
    pub on: bool,
}

/// Settings › Skills: stops loading skills from a folder the user added.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveSkillFolder {
    pub id: String,
}

/// Settings › Skills: opens a skill, in the editor when it is one of the user's, else read-only.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenSkill {
    pub file: std::path::PathBuf,
}

/// Settings › Skills: copies a skill from another folder into Your skills.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct CopySkill {
    pub file: std::path::PathBuf,
}

/// Settings › Skills' editor: whether the skill runs only as /name.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseSkillUse {
    pub manual: bool,
}

/// Settings › Commands: one command's switch, by its key.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchSlashCommand {
    pub key: String,
    pub on: bool,
}

/// Settings › Commands: opens one of the user's commands in the editor.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenSlashCommand {
    pub name: String,
}

/// Picks a row of the `/` menu.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct PickCommand {
    pub index: usize,
}

/// Stops a background command for the user.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct StopTask {
    pub id: gpui::SharedString,
}

/// Shows a background command's output in the Tasks pane.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ShowTask {
    pub id: gpui::SharedString,
}

/// Shows a sub-agent's work in the Agents pane.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ShowAgent {
    pub id: gpui::SharedString,
}

/// Stops one sub-agent for the user.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct StopAgent {
    pub id: gpui::SharedString,
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

/// Settings › Web: makes a search provider the active one.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseSearchProvider {
    pub id: String,
}

/// Settings › Web: the editor for a new provider of this kind.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AddSearchProvider {
    pub kind: wack_core::web::search::SearchKind,
}

/// Settings › Web: the editor for a saved provider.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct EditSearchProvider {
    pub id: String,
}

/// Settings › Web: where the edited provider's key comes from: its own, or a connection's.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseSearchKey {
    pub connection: Option<String>,
}

/// Settings › Browser: the size new tabs open at.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseDefaultViewport {
    pub viewport: wack_browser::Viewport,
}

/// Settings › Web: a Custom provider's request method.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseSearchMethod {
    pub method: wack_core::web::search::HttpMethod,
}

/// Settings › MCP servers: the editor for a new server of this type.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AddMcpServer {
    pub transport: wack_core::mcp::McpTransport,
}

/// Settings › MCP servers: the editor for a saved server.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct EditMcpServer {
    pub id: String,
}

/// Settings › MCP servers: a server's switch.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleMcpServer {
    pub id: String,
}

/// Settings › MCP servers: one tool's switch, by the server's own name for it.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleMcpTool {
    pub id: String,
    pub tool: String,
}

/// Settings › MCP servers: opens or folds a server's row.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ExpandMcpServer {
    pub id: String,
}

/// Settings › MCP servers: connects to a saved server and lists its tools.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct TestMcpServer {
    pub id: String,
}

/// Settings › MCP servers: asks before deleting a server.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct AskDeleteMcpServer {
    pub id: String,
}

/// Settings › MCP servers: removes the editor's variable or header at `index`.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct RemoveMcpEntry {
    pub index: usize,
}

/// Settings › MCP servers: the edited server's type.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseMcpTransport {
    pub transport: wack_core::mcp::McpTransport,
}

/// Settings › MCP servers: all projects, or only chosen ones.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseMcpScope {
    pub only: bool,
}

/// Settings › MCP servers: adds or removes a project from the edited server's chosen ones.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ToggleMcpProject {
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenSettingsSection {
    pub section: crate::settings::Section,
}

/// Picks the question card's option at `index` (0-based; keys 1–4).
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct PickOption {
    pub index: usize,
}

/// Shows the question card's question at `index`.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ShowQuestion {
    pub index: usize,
}

/// Build, Plan or Ultra Plan, for the chat on screen or the new chat. Ignored while it works.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SetAgentMode {
    pub mode: wack_core::AgentMode,
}

/// Light or dark appearance.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseMode {
    pub mode: wack_core::Mode,
}

/// One of the fixed themes (Settings › Appearance), remembered on the picture showing.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseTheme {
    pub theme: wack_core::ThemeName,
}

/// How much the backdrop is dimmed, in percent.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseDim {
    pub dim: u8,
}

/// How much the backdrop is blurred, in points.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseBlur {
    pub blur: u8,
}

/// How the backdrop is drawn on the new-chat screen.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseTexture {
    pub texture: wack_core::Texture,
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
        KeyBinding::new("escape", CancelRename, Some(RENAME_CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(FORM_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(FORM_CONTEXT)),
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-g", ToggleChanges, Some(CONTEXT)),
        KeyBinding::new("cmd-b", MoveToBackground, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-a", ToggleAgentsPane, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-b", ToggleBrowserPane, Some(CONTEXT)),
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
        KeyBinding::new("shift-tab", CycleAgentMode, Some(CONTEXT)),
        KeyBinding::new("shift-tab", TerminalBackTab, Some(TERMINAL_CONTEXT)),
    ]);
}

/// The key context the shell's root sets, so the bindings above apply anywhere in the window.
pub fn context() -> &'static str {
    CONTEXT
}

/// Settings › Sub-agents: how freely the agent delegates.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseDelegation {
    pub delegation: wack_core::agents::Delegation,
}

/// Settings › Sub-agents: the helper model, or `None` for the chat's own.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseHelperModel {
    pub connection: Option<String>,
    pub model: Option<String>,
}

/// Settings › Sub-agents: one agent's switch.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchAgent {
    pub name: String,
    pub on: bool,
}

/// Settings › Sub-agents: opens an agent in the editor.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct OpenAgent {
    pub name: String,
}

/// Settings › Sub-agents' editor: the model the agent runs on.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct ChooseAgentModel {
    pub helper: bool,
}

/// Settings › Sub-agents: another tool's agent folder, on or off.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = wackcode, no_json)]
pub struct SwitchAgentFolder {
    pub id: String,
    pub on: bool,
}
