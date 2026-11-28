//! The app's state: the engine's store, which screen is showing, and a live [`ChatSession`] for
//! every chat opened since launch. Views read it and dispatch actions; every change goes through
//! a method here.
//!
//! Invariants:
//! - `store` is the only copy of projects, chats and connections. Nothing here caches them.
//! - A chat's session, once made, lives until the chat or its project is removed, so a run keeps
//!   going while the user looks elsewhere. Removing it drops its run.
//! - Nothing the user wrote is lost: a message that cannot be sent stays in its composer, and
//!   queued or steering messages a run did not deliver go back into the composer.
//! - `tabs` lists chats that exist, each once; a chat on screen is always one of them. They live
//!   for this launch only: the app opens on a new chat, as version 2 did, rather than where the
//!   last one was left. The sidebar's side and the project last worked in are saved, and a
//!   chat's pane layout shortly after it changes (a drag changes it on every frame).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};

use futures::{StreamExt, channel::mpsc};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, Task, Window};
use gpui_tokio::Tokio;
use wack_core::{
    AgentMode, Appearance, Chat, ChatKind, CommandEvent, CommandKind, CommandStatus, Job, ModelChoice, Outcome,
    AutoTitleConfig, Project, Reasoning, Runner, Store, Ui, WindowState, WireFormat,
    chat_refs::{self, ChatRef},
    files::{self, WorkspaceFiles},
    skills::{Command, CommandUse, Outgoing, active_slash, find, matching, parse_line, pick, resolve_message},
    step_summary, title, title_from,
};

use crate::{
    appearance_page::Thumbnail,
    backdrop::Picture,
    chat::{ChatEvent, ChatSession, now_millis},
    memory_page::{MemoryConfirm, MemoryEditor, MemoryState},
    web_page::{EditorData as WebEditorData, TestOutcome, WebData, WebEditor, WebState},
    mcp_page::{EditorData as McpEditorData, Live, McpData, McpEditor, McpState, ProbeOutcome, RowTest, ServerRow},
    skills_page::{SkillEditor, SkillsState, Viewing},
    commands_page::{CommandEditor, CommandsState},
    motion::TitleSwap,
    panes::{ChatPanes, Layout, PaneKind},
    run_button::{CommandButtonData, CommandEditorData},
    settings::{ProviderForm, Section},
    sidebar::RowConfirm,
    slash_menu::{Matches, Slash, SlashData},
    text_field::{MenuKey, TextField, TextFieldEvent},
    theme,
    typography::Type,
};

/// Why a `/command` can't go with attached text files: the files would fold into its expansion.
const FILES_WITH_COMMAND: &str = "A command can't take attached files. Remove them first.";

/// Rows a composer grows to before it scrolls.
pub const COMPOSER_ROWS: usize = 10;
pub const NEW_CHAT_PLACEHOLDER: &str = "Describe a task, or ask about the code";
/// The Chat area's composers: the hero's, and a chat's.
pub const CHAT_AREA_PLACEHOLDER: &str = "Ask anything…";
pub const CHAT_FOLLOW_UP_PLACEHOLDER: &str = "Message WackCode…";
/// The Chat area's key among the new-chat screens' panes (the others are project ids).
const CHAT_AREA_PANES: &str = "chat-area";
/// How long a control clicked once keeps asking.
const CONFIRM_FOR: Duration = Duration::from_secs(4);
/// How many chats the Chat area's `@` picker offers at once.
const MENTION_LIMIT: usize = 8;
/// How many files and folders Code's `@` picker offers at once.
const FILE_MENTION_LIMIT: usize = 20;
/// The only slash commands a Chat chat has: the app's own. Anything else goes as written.
const CHAT_AREA_COMMANDS: [&str; 4] = ["new", "name", "copy", "compact"];
pub const PLAN_PLACEHOLDER: &str = "Describe the work — in Plan mode WackCode inspects and proposes a plan for your approval…";
pub const ULTRA_PLAN_PLACEHOLDER: &str = "Describe the work — in Ultra Plan WackCode interviews you one question at a time…";

/// What a new chat's empty composer says in Build: the project it will work in.
pub fn new_chat_placeholder(project: &str) -> String {
    format!("Ask about {project}, or describe a task…")
}

/// What an empty composer says: the planning modes explain themselves.
pub fn placeholder(mode: AgentMode, new_chat: bool) -> &'static str {
    match mode {
        AgentMode::Plan => PLAN_PLACEHOLDER,
        AgentMode::UltraPlan => ULTRA_PLAN_PLACEHOLDER,
        AgentMode::Build if new_chat => NEW_CHAT_PLACEHOLDER,
        AgentMode::Build => crate::chat::FOLLOW_UP_PLACEHOLDER,
    }
}

/// What a Chat chat's message sends: one of the app's own commands (`/new`, `/name`, `/copy`,
/// `/compact`), or the words as written. A Chat chat has no skills or templates, so any other
/// `/word` is just text.
pub fn chat_area_outgoing(text: &str) -> Outgoing {
    match parse_line(text) {
        Some((name, args)) => match CHAT_AREA_COMMANDS.iter().find(|builtin| **builtin == name) {
            Some(&name) => Outgoing::Builtin { name, args: args.to_string() },
            None => Outgoing::Message(text.to_string()),
        },
        None => Outgoing::Message(text.to_string()),
    }
}

/// A menu open over the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Model,
    Effort,
    /// The crew of the chat's Team: its model and effort.
    Crew,
}

/// A new chat being written, kept as a tab while another is on screen.
#[derive(Clone)]
pub struct DraftTab {
    pub key: u64,
    /// Its project; `None` in the Chat area.
    pub project: Option<String>,
    text: String,
    attachments: Vec<crate::attachments::Attachment>,
    files: Vec<crate::attachments::FileAttachment>,
}

impl DraftTab {
    fn blank(&self) -> bool {
        self.text.trim().is_empty() && self.attachments.is_empty() && self.files.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// No project yet.
    Welcome,
    /// Writing the first message of a chat in this project.
    NewChat(String),
    /// Writing the first message of a chat in the Chat area: the hero.
    NewChatArea,
    Chat(String),
    Settings,
}

/// The command configuration popover currently open on screen.
pub struct CommandEditorState {
    pub project_id: String,
    pub kind: CommandKind,
    pub input: Entity<TextField>,
    pub error: Option<SharedString>,
    _subscription: Subscription,
}

fn command_placeholder(kind: CommandKind) -> &'static str {
    match kind {
        CommandKind::Build => "cargo build",
        CommandKind::Run => "cargo run",
    }
}

fn command_pane_kind(kind: CommandKind) -> PaneKind {
    match kind {
        CommandKind::Build => PaneKind::Build,
        CommandKind::Run => PaneKind::Run,
    }
}

pub struct AppState {
    store: Store,
    selection: Selection,
    /// Where closing Settings goes back to.
    before_settings: Selection,
    sessions: HashMap<String, Entity<ChatSession>>,
    /// The browser engine, shared by every chat's tabs. It starts on first use (browser.rs).
    browser: std::sync::Arc<dyn wack_browser::Backend>,
    /// Each open chat's shell tasks, so quitting can stop them without a window context.
    chat_tasks: HashMap<String, wack_core::tasks::Tasks>,
    /// Each open chat's sub-agents, likewise.
    chat_agents: HashMap<String, wack_core::agents::Agents>,
    session_subscriptions: HashMap<String, Vec<Subscription>>,
    /// Projects whose chats the sidebar lists.
    open_projects: HashSet<String>,
    /// When a message last left a composer, for the composer's send moment (`motion::send_moment`).
    pub sent_at: Option<std::time::Instant>,
    /// Each project's checked-out branch, looked up off the UI thread.
    branches: HashMap<String, Option<String>>,
    /// The new-chat composer, shared by every project.
    pub draft: Entity<TextField>,
    /// Pictures attached to the new chat's first message.
    pub draft_attachments: Vec<crate::attachments::Attachment>,
    /// Text files attached to the new chat's first message (picked, dropped, pasted, terminal).
    pub draft_files: Vec<crate::attachments::FileAttachment>,
    /// Where a drag in Settings › Appearance has the backdrop's focus, shown everywhere but saved
    /// only when the drag ends.
    pub crop_preview: Option<(f32, f32)>,
    /// The picture open over the window.
    pub lightbox: Option<crate::pictures::Lightbox>,
    /// The mode the new chat starts in.
    pub new_chat_mode: AgentMode,
    /// The new chat's Team crew, carried into the chat its first message makes.
    pub new_chat_team: Option<ModelChoice>,
    /// Team chats whose Agents pane has opened by itself for their first worker since launch.
    team_pane_opened: HashSet<String>,
    /// Why the last chat could not start, shown under the new-chat composer.
    pub notice: Option<SharedString>,
    /// Settings › Providers' connection editor.
    pub form: ProviderForm,
    /// The model catalogue that fills the editor's model rows.
    pub catalog: crate::model_catalog::CatalogState,
    /// Fetch models' filter field, watched while the list is open.
    fetch_subscription: Option<Subscription>,
    pub menu: Option<Menu>,
    /// The section Settings shows.
    pub settings_section: Section,
    /// Why the last appearance change could not be made.
    pub appearance_notice: Option<SharedString>,
    /// A backdrop picture is being read.
    pub importing: bool,
    /// Settings › Tools is asking before planning loses its restrictions.
    pub confirming_unrestricted: bool,
    /// Settings › Memory: the overview as last read, the note editor and a pending delete.
    pub memory: MemoryState,
    memory_subscriptions: Vec<Subscription>,
    /// Settings › Web: the provider editor and the overview's last failure.
    pub web: WebState,
    /// Every MCP connection, shared by the chats of each project; ended on quit.
    mcp: wack_core::mcp_client::Hub,
    /// Settings › MCP servers: the editor, the open row, and each server's last test.
    pub mcp_settings: McpState,
    mcp_subscriptions: Vec<Subscription>,
    web_subscriptions: Vec<Subscription>,
    /// Settings › Skills: the folders as last read, the editor, a skill on view.
    pub skills: SkillsState,
    skill_subscriptions: Vec<Subscription>,
    /// Settings › Commands: the commands as last read and the editor.
    pub templates: CommandsState,
    template_subscriptions: Vec<Subscription>,
    pub subagents: crate::subagents_page::SubagentsState,
    subagent_subscriptions: Vec<Subscription>,
    /// Settings › Prompts: the open prompt editor and the restore-all confirmation.
    pub prompts: crate::prompts_page::PromptsState,
    prompt_subscriptions: Vec<Subscription>,
    /// Settings › Git's last save failure, if any.
    pub git_error: Option<SharedString>,
    /// Settings › Browser.
    pub browser_error: Option<SharedString>,
    /// The app's one link to the Mac for computer use; each Code chat gets its own part of it.
    pub computer: wack_computer::computer::Computer,
    /// Settings › Computer use's last failure, if any.
    pub computer_error: Option<SharedString>,
    pub browser_clearing: crate::browser_page::Clearing,
    /// Long-lived Build and Run PTY commands keyed by `(canonical_cwd, kind)`.
    runs: Runner,
    command_event_tx: mpsc::UnboundedSender<CommandEvent>,
    /// Draft panes (for example Build / Run output) open on `Screen::NewChat` before a chat starts.
    new_chat_panes: HashMap<String, ChatPanes>,
    /// Browser tabs opened on `Screen::NewChat`, handed to the chat its first message made, by
    /// chat id, so its pages carry on rather than load again.
    handed_tabs: HashMap<String, wack_browser::ChatTabs>,
    /// The open Build/Run command editor popover, if any.
    pub command_editor: Option<CommandEditorState>,
    /// Chats open as tabs, in order. The strip shows the area's own.
    tabs: Vec<String>,
    /// Where each area was left, so switching areas comes back to it.
    parked: HashMap<ChatKind, Selection>,
    /// What each area's new-chat composer held when the other area was entered.
    parked_drafts: HashMap<ChatKind, String>,
    /// New chats being written, each a tab of its own as in version 2: "+" on a new chat opens
    /// another beside it. The one on screen is `active_draft`; its words live in `draft` while
    /// it shows and in its tab while it does not.
    drafts: Vec<DraftTab>,
    active_draft: Option<u64>,
    next_draft: u64,
    /// Counts the Chat area's new-chat screens, so each visit gets its own idea chips.
    pub hero_visit: u64,
    sidebar_hidden: bool,
    /// Chats whose pane layout changed since it was last saved.
    unsaved_layouts: HashSet<String>,
    saving_layouts: Option<Task<()>>,
    /// Normal-mode window geometry waiting for a move/resize drag to settle (or exit) before
    /// writing `state.json`.
    pending_window: Option<WindowState>,
    saving_window: Option<Task<()>>,
    /// The connection editor's fields, re-subscribed whenever its model rows change.
    form_subscriptions: Vec<Subscription>,
    /// Automatic titles being asked for, by chat: the attempt and the request. Only the current
    /// attempt's answer is used, and dropping the task cancels the request, so a rename, a stop or
    /// a delete simply removes the entry.
    titling: HashMap<String, (u64, Task<()>)>,
    title_attempts: u64,
    /// Chats whose title the model just changed, for the swap animation. Old entries are left to
    /// be replaced: [`TitleSwap::live`] tells the views when one is over.
    pub title_swaps: HashMap<String, TitleSwap>,
    /// Settings › Providers › Automatic titles: the model list is open, and why the last change
    /// could not be made.
    pub title_models_open: bool,
    pub title_error: Option<SharedString>,
    /// Settings › Sub-agents › Step summaries: the model list is open, and what went wrong.
    pub summary_models_open: bool,
    pub summary_error: Option<SharedString>,
    /// Chats already told this session that step summaries' model can't be used: told once.
    summary_warned: HashSet<String>,
    /// The chat being renamed in place, if any.
    pub renaming: Option<Renaming>,
    /// The New Chat screen's list of projects is open.
    pub project_picker: bool,
    /// A chat's right-click menu, while open.
    pub chat_menu: Option<ChatMenu>,
    /// A message's right-click menu, in the chat on screen.
    pub prompt_menu: Option<PromptMenu>,
    /// The `/` menu, open over one composer.
    pub slash: Option<Slash>,
    /// Where ⎋ closed the menu (whose composer, the word's start): it stays shut for that word.
    slash_dismissed: Option<(Option<String>, usize)>,
    /// The chats picked with `@` in each Chat-area composer (a chat's id, or `None` for the new
    /// chat's), sent as the message's referenced-chats section while their `@"Name"` is still in
    /// the text (`wack_core::chat_refs::surviving`).
    mention_refs: HashMap<Option<String>, Vec<ChatRef>>,
    /// Workspace file lists cached per project folder for Code's `@` file/folder picker.
    workspace_files: HashMap<PathBuf, WorkspaceFilesCache>,
    /// The sidebar's search field: what is typed filters the area on screen. Emptied when the
    /// area changes.
    pub search: Entity<TextField>,
    /// The sidebar shows the area's Archived view instead of its list.
    pub archived_open: bool,
    /// A sidebar control clicked once that now asks (the chat, and which): a second click does
    /// it, and it stops asking after a few seconds or when anything else is clicked.
    pub confirming: Option<(String, RowConfirm)>,
    /// The Archived view's Delete all, clicked once.
    pub confirming_all: bool,
    /// The commands as last read, for the menu and for lighting a typed `/command`. Read again
    /// each time the menu opens.
    commands: Vec<Command>,
    /// A message refused as an unknown command (whose composer, its text): ↩ again with the text
    /// unchanged sends it as written.
    literal: Option<(Option<String>, String)>,
    _subscriptions: Vec<Subscription>,
}

/// Cached workspace file listing for Code's `@` picker, per project folder.
#[derive(Default)]
struct WorkspaceFilesCache {
    /// The last completed listing, kept visible while a refresh runs.
    files: Option<WorkspaceFiles>,
    /// Why the last listing failed, when no listing is held.
    error: Option<String>,
    loading: bool,
    /// Monotonic token so a slower earlier scan never overwrites a newer one.
    request: u64,
}

/// A chat's right-click menu: whose, where the pointer was, and whether its sidebar row opened it.
#[derive(Clone, Debug)]
pub struct ChatMenu {
    pub chat: String,
    pub position: gpui::Point<gpui::Pixels>,
    pub sidebar: bool,
}

/// A message's right-click menu: whose chat, which of the user's messages (from 0), and where
/// the pointer was.
#[derive(Clone, Debug)]
pub struct PromptMenu {
    pub chat: String,
    pub number: usize,
    pub position: gpui::Point<gpui::Pixels>,
}

/// A chat's title being edited where it is shown: in its header or on its sidebar row.
pub struct Renaming {
    pub chat: String,
    /// Where the field is drawn: the sidebar row or the chat's header.
    pub place: RenamePlace,
    pub field: Entity<TextField>,
    /// What had focus before, to give it back.
    back: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenamePlace {
    /// The chat's header, at the title's own size: from the title, its tab or File › Rename
    /// Chat…. (Once it was the 28-point tab, which the owner found cramped.) The tab of the chat
    /// being renamed carries the field (`TitleLook::Editing`) for the header to draw.
    Header,
    Sidebar,
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.flush_window();
        self.runs.terminate_all();
    }
}

/// Times task news may wake a chat before the user writes again.
const MAX_WAKES: u32 = 3;

/// How long a layout must stay still before it is saved.
const LAYOUT_SAVE_DELAY: Duration = Duration::from_millis(400);

/// How long a connection's "Saved" confirmation stays in the editor's bar.
const SAVE_NOTICE_TIME: Duration = Duration::from_secs(3);

impl AppState {
    /// Opens the store in the data folder: the dev app's own for a debug build.
    pub fn open_store() -> Result<Store, String> {
        Store::open(&Store::default_folder()?)
    }

    pub fn new(mut store: Store, cx: &mut Context<Self>) -> Self {
        // Shuffle-on-open: rotate the backdrop before applying the theme so the window's first
        // frame already wears the picture and theme this launch picked.
        let appearance_notice = store.shuffle_backdrop_on_open().err().map(Into::into);
        let draft = cx.new(|cx| TextField::new(Type::Body, NEW_CHAT_PLACEHOLDER, cx).multiline(COMPOSER_ROWS).takes_pictures());
        let draft_subscription = cx.subscribe(&draft, |app, _, event, cx| match event {
            TextFieldEvent::Enter | TextFieldEvent::SendNow => app.start_chat(cx),
            TextFieldEvent::Changed => {
                if app.notice.take().is_some() {
                    cx.notify();
                }
                app.composer_changed(None, cx);
            }
            TextFieldEvent::Moved => app.composer_changed(None, cx),
            TextFieldEvent::Menu(key) => app.slash_key(None, *key, cx),
            TextFieldEvent::PastedPictures => app.paste_pictures(None, cx),
        });
        let form = ProviderForm::new(cx);
        let mcp = wack_core::mcp_client::Hub::new(Some(store.mcp_cache_folder()));
        let search = cx.new(|cx| TextField::new(Type::Ui, "Search chats…", cx));
        let search_subscription = cx.subscribe(&search, |app, _, event, cx| {
            if matches!(event, TextFieldEvent::Changed) {
                app.confirming = None;
                app.confirming_all = false;
                cx.notify();
            }
        });
        let quit_subscription = cx.on_app_quit(|app, _| {
            app.flush_window();
            async {}
        });
        let subscriptions = vec![draft_subscription, search_subscription, quit_subscription];
        let ui = store.ui().clone();
        // A fresh app: no tabs, no chat in front, and a new chat in the project last worked in (as
        // version 2 opened), rather than wherever the last one was left.
        let selection = match store.projects() {
            _ if ui.area == ChatKind::Chat => Selection::NewChatArea,
            [] => Selection::Welcome,
            projects => {
                let id = ui
                    .last_project
                    .as_deref()
                    .and_then(|id| store.project(id))
                    .or_else(|| store.last_used_project())
                    .or_else(|| projects.first())
                    .unwrap()
                    .id
                    .clone();
                Selection::NewChat(id)
            }
        };
        let tabs: Vec<String> = Vec::new();
        let runs = Runner::new();
        let (command_event_tx, mut command_event_rx) = mpsc::unbounded::<CommandEvent>();
        cx.spawn(async move |app, cx| {
            const MAX_BATCH: usize = 4_096;
            while let Some(first) = command_event_rx.next().await {
                let mut batch = vec![first];
                while batch.len() < MAX_BATCH {
                    match command_event_rx.try_recv() {
                        Ok(next) => batch.push(next),
                        Err(_) => break,
                    }
                }
                if app.update(cx, |app, cx| app.handle_command_events(batch, cx)).is_err() {
                    break;
                }
                // Yield back to the main queue (~60 Hz cap) so noisy Build/Run output never
                // starves AppKit or GPUI.
                cx.background_executor().timer(Duration::from_millis(16)).await;
            }
        })
        .detach();
        // Before anything can start the browser: a clear the running browser couldn't finish
        // (Settings › Browser), and the size new tabs open at.
        wack_browser::cef::finish_clearing(&store.browser_folder());
        wack_browser::set_default_viewport(store.browser_config().default_viewport);
        let browser = crate::browser::engine(&store);
        // On the main thread: it reads the keyboard layout.
        let computer = wack_computer::computer::Computer::native(store.computer_config().settings());
        let mut app = Self {
            store,
            before_settings: selection.clone(),
            selection: Selection::Welcome,
            sent_at: None,
            sessions: HashMap::new(),
            browser,
            chat_tasks: HashMap::new(),
            chat_agents: HashMap::new(),
            session_subscriptions: HashMap::new(),
            open_projects: HashSet::new(),
            branches: HashMap::new(),
            draft,
            draft_attachments: Vec::new(),
            draft_files: Vec::new(),
            crop_preview: None,
            lightbox: None,
            slash: None,
            slash_dismissed: None,
            mention_refs: HashMap::new(),
            workspace_files: HashMap::new(),
            search,
            archived_open: false,
            confirming: None,
            confirming_all: false,
            commands: Vec::new(),
            literal: None,
            new_chat_mode: AgentMode::Build,
            new_chat_team: None,
            team_pane_opened: HashSet::new(),
            notice: None,
            form,
            menu: None,
            settings_section: Section::default(),
            confirming_unrestricted: false,
            memory: MemoryState::default(),
            memory_subscriptions: Vec::new(),
            web: WebState::default(),
            web_subscriptions: Vec::new(),
            mcp,
            mcp_settings: McpState::default(),
            mcp_subscriptions: Vec::new(),
            skills: SkillsState::default(),
            skill_subscriptions: Vec::new(),
            templates: CommandsState::default(),
            template_subscriptions: Vec::new(),
            subagents: Default::default(),
            subagent_subscriptions: Vec::new(),
            prompts: Default::default(),
            prompt_subscriptions: Vec::new(),
            git_error: None,
            browser_error: None,
            computer,
            computer_error: None,
            browser_clearing: Default::default(),
            runs,
            command_event_tx,
            new_chat_panes: HashMap::new(),
            handed_tabs: HashMap::new(),
            command_editor: None,
            appearance_notice,
            importing: false,
            tabs,
            parked: HashMap::new(),
            parked_drafts: HashMap::new(),
            drafts: Vec::new(),
            active_draft: None,
            next_draft: 1,
            hero_visit: now_millis() / 1000,
            sidebar_hidden: ui.sidebar_hidden,
            unsaved_layouts: HashSet::new(),
            saving_layouts: None,
            pending_window: None,
            saving_window: None,
            form_subscriptions: Vec::new(),
            catalog: Default::default(),
            fetch_subscription: None,
            titling: HashMap::new(),
            title_attempts: 0,
            title_swaps: HashMap::new(),
            title_models_open: false,
            title_error: None,
            summary_models_open: false,
            summary_error: None,
            summary_warned: HashSet::new(),
            renaming: None,
            chat_menu: None,
            prompt_menu: None,
            project_picker: false,
            _subscriptions: subscriptions,
        };
        app.watch_form(cx);
        theme::apply(app.store.appearance(), cx);
        // The agent's cursor, drawn over other apps' windows, wears the theme and follows it.
        app.computer.set_cursor_appearance(crate::theme::Theme::get(cx).cursor_appearance());
        cx.observe_global::<crate::theme::Theme>(|app, cx| app.computer.set_cursor_appearance(crate::theme::Theme::get(cx).cursor_appearance())).detach();
        app.select(selection, cx);
        app.start_usage_tasks(cx);
        app
    }

    /// The ledger's two background loops: a write that failed is retried every few seconds,
    /// and Settings › Integrations, while it is the section shown, refreshes to show the queue
    /// draining and failures clearing on their own.
    fn start_usage_tasks(&self, cx: &mut Context<Self>) {
        let writer = self.store.usage().clone();
        Tokio::spawn(cx, async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                writer.flush();
            }
        })
        .detach();
        cx.spawn(async move |app, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if app
                    .update(cx, |app, cx| {
                        // The ledger's health and the MCP servers' connections change on their own.
                        if app.selection == Selection::Settings && matches!(app.settings_section, Section::Integrations | Section::Mcp) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }

    /// Settings › Tools: whether the agent may read public web pages.
    pub fn toggle_web_fetch(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = self.store.set_web_fetch(!self.store.web_fetch_enabled()) {
            self.notice = Some(message.into());
        }
        cx.notify();
    }

    fn web_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.web.error = result.err().map(Into::into);
        cx.notify();
    }

    pub fn toggle_web_search(&mut self, cx: &mut Context<Self>) {
        let result = self.store.set_web_search(!self.store.web_config().search);
        self.web_result(result, cx);
    }

    pub fn toggle_route_fetch(&mut self, cx: &mut Context<Self>) {
        let result = self.store.set_route_fetch(!self.store.web_config().route_fetch);
        self.web_result(result, cx);
    }

    pub fn choose_search_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        let result = self.store.set_active_search_provider(id);
        self.web_result(result, cx);
    }

    pub fn reset_web_usage(&mut self, cx: &mut Context<Self>) {
        let result = self.store.reset_web_usage(None);
        self.web_result(result, cx);
    }

    /// Settings › Web as the page draws it.
    pub fn web_data(&self, cx: &App) -> WebData {
        let config = self.store.web_config();
        let editor = self.web.editor.as_ref().map(|editor| {
            let lenders = self
                .store
                .lendable_connections(editor.kind)
                .into_iter()
                .map(|connection| (connection.id.clone(), connection.name.clone()))
                .collect();
            let saved = editor.editing.as_deref().is_some_and(|id| self.store.search_key_saved(id));
            WebEditorData::of(editor, saved, lenders, cx)
        });
        WebData {
            fetch: config.fetch,
            search: config.search,
            route_fetch: config.route_fetch,
            providers: self.store.search_providers(),
            connection_names: self.store.connections().iter().map(|connection| (connection.id.clone(), connection.name.clone())).collect(),
            editor,
            error: self.web.error.clone(),
        }
    }

    /// The editor for a new provider of `kind`. Returns its name field.
    pub fn add_search_provider(&mut self, kind: wack_core::web::search::SearchKind, cx: &mut Context<Self>) -> Entity<TextField> {
        let draft = wack_core::web::SearchDraft::new(kind);
        self.open_search_editor(WebEditor::new(&draft, cx), cx)
    }

    pub fn edit_search_provider(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let provider = self.store.web_config().providers.iter().find(|provider| provider.id == id)?.clone();
        let draft = wack_core::web::SearchDraft::of(&provider);
        Some(self.open_search_editor(WebEditor::new(&draft, cx), cx))
    }

    fn open_search_editor(&mut self, editor: WebEditor, cx: &mut Context<Self>) -> Entity<TextField> {
        let name = editor.name.clone();
        self.web.editor = Some(editor);
        self.web.error = None;
        self.watch_search_editor(cx);
        cx.notify();
        name
    }

    pub fn close_search_editor(&mut self, cx: &mut Context<Self>) {
        self.web.editor = None;
        self.web_subscriptions.clear();
        cx.notify();
    }

    pub fn choose_search_key(&mut self, connection: Option<String>, cx: &mut Context<Self>) {
        if let Some(editor) = self.web.editor.as_mut() {
            editor.key_source = match connection {
                Some(connection) => wack_core::web::search::KeySource::Connection { connection },
                None => wack_core::web::search::KeySource::Own,
            };
            editor.error = None;
        }
        cx.notify();
    }

    pub fn choose_search_method(&mut self, method: wack_core::web::search::HttpMethod, cx: &mut Context<Self>) {
        if let Some(editor) = self.web.editor.as_mut() {
            editor.method = method;
            editor.error = None;
        }
        cx.notify();
    }

    pub fn save_search_provider(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.web.editor.as_ref() else {
            return;
        };
        let key = editor.key(cx);
        let result = editor.draft(cx).and_then(|draft| self.store.save_search_provider(&draft, key.as_deref()));
        match result {
            Ok(_) => self.close_search_editor(cx),
            Err(message) => {
                if let Some(editor) = self.web.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    /// Runs the draft as it stands, with a typed key or the saved one, and shows what came back.
    pub fn test_search_provider(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.web.editor.as_mut() else {
            return;
        };
        let key = editor.key(cx);
        let client = editor.draft(cx).and_then(|draft| self.store.draft_search_client(&draft, key.as_deref()));
        let client = match client {
            Ok(client) => client,
            Err(message) => {
                editor.test = Some(TestOutcome::Failed(message.into()));
                cx.notify();
                return;
            }
        };
        editor.testing = true;
        editor.test = None;
        editor.test_serial += 1;
        let serial = editor.test_serial;
        let work = Tokio::spawn(cx, async move {
            let started = std::time::Instant::now();
            let query = wack_core::web::search::normalize_query("WackCode", Some(3), None, None)?;
            let hits = client.search(&query).await?;
            Ok::<_, String>((started.elapsed().as_millis() as u64, hits))
        });
        cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The test search stopped.".into()));
            let _ = app.update(cx, |app, cx| {
                let Some(editor) = app.web.editor.as_mut().filter(|editor| editor.test_serial == serial) else {
                    return;
                };
                editor.testing = false;
                editor.test = Some(match result {
                    Ok((elapsed_ms, hits)) => TestOutcome::Found {
                        elapsed_ms,
                        hits: hits
                            .into_iter()
                            .map(|hit| {
                                let host = hit.url.split('/').nth(2).unwrap_or("").trim_start_matches("www.").to_string();
                                (hit.title.into(), host.into())
                            })
                            .collect(),
                    },
                    Err(message) => TestOutcome::Failed(message.into()),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn ask_delete_search_provider(&mut self, confirming: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.web.editor.as_mut() {
            editor.confirming_delete = confirming;
        }
        cx.notify();
    }

    pub fn confirm_delete_search_provider(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.web.editor.as_ref().and_then(|editor| editor.editing.clone()) else {
            return;
        };
        match self.store.delete_search_provider(&id) {
            Ok(()) => self.close_search_editor(cx),
            Err(message) => {
                if let Some(editor) = self.web.editor.as_mut() {
                    editor.confirming_delete = false;
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    fn watch_search_editor(&mut self, cx: &mut Context<Self>) {
        self.web_subscriptions.clear();
        let Some(editor) = self.web.editor.as_ref() else {
            return;
        };
        let body = editor.body_template.clone();
        for field in editor.fields() {
            let multiline = field == body;
            self.web_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter if multiline => field.update(cx, |field, cx| field.insert_line_break(cx)),
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_search_provider(cx),
                TextFieldEvent::Changed => {
                    if let Some(editor) = app.web.editor.as_mut() {
                        editor.error = None;
                    }
                    cx.notify();
                }
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Settings › MCP servers

    /// Closes connections to servers that were removed, switched off, changed, or scoped away.
    fn mcp_changed(&mut self) {
        self.mcp.retain(|id, fingerprint, folder| self.store.mcp_keeps_connection(id, fingerprint, folder));
    }

    fn project_names(&self) -> Vec<(String, SharedString)> {
        self.store.projects().iter().map(|project| (project.id.clone(), project.name.clone().into())).collect()
    }

    /// Settings › MCP servers as the page draws it, with each server's connections as they stand.
    pub fn mcp_data(&self) -> McpData {
        use wack_core::mcp_client::State;
        let names = self.project_names();
        let states = self.mcp.states();
        let servers = self
            .store
            .mcp_servers()
            .iter()
            .map(|server| {
                let mine: Vec<&State> = states.iter().filter(|state| state.server == server.id).map(|state| &state.state).collect();
                let connected: Vec<usize> = mine.iter().filter_map(|state| if let State::Connected { tools } = state { Some(*tools) } else { None }).collect();
                let live = if let Some(tools) = connected.iter().max() {
                    Live::Connected { tools: *tools, folders: connected.len() }
                } else if mine.iter().any(|state| matches!(state, State::Connecting)) {
                    Live::Connecting
                } else if let Some(error) = mine.iter().find_map(|state| if let State::Failed(error) = state { Some(error.clone()) } else { None }) {
                    Live::Failed(error.into())
                } else {
                    Live::Idle
                };
                let pending = self.mcp_settings.pending_scope.as_ref().filter(|(id, _)| id == &server.id);
                let scope = pending.map_or_else(|| server.scope.clone(), |(_, scope)| scope.clone());
                ServerRow {
                    reach: crate::mcp_page::reach(&server.scope, &names),
                    scope,
                    scope_pending: pending.is_some(),
                    server: server.clone(),
                    live,
                    test: self.mcp_settings.tests.get(&server.id).cloned(),
                }
            })
            .collect();
        let editor = self.mcp_settings.editor.as_ref().map(|editor| {
            let confirming = editor.editing.is_some() && editor.editing == self.mcp_settings.confirming_delete;
            McpEditorData::of(editor, names.clone(), confirming)
        });
        McpData {
            servers,
            all_projects: names,
            expanded: self.mcp_settings.expanded.clone(),
            confirming_delete: self.mcp_settings.confirming_delete.clone(),
            editor,
            error: self.mcp_settings.error.clone(),
        }
    }

    /// The editor for a new server. Returns its name field.
    pub fn add_mcp_server(&mut self, transport: wack_core::mcp::McpTransport, cx: &mut Context<Self>) -> Entity<TextField> {
        let editor = McpEditor::new(&wack_core::mcp::McpDraft::new(transport), &[], cx);
        self.open_mcp_editor(editor, cx)
    }

    pub fn edit_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let server = self.store.mcp_server(id)?.clone();
        let saved = self.store.mcp_saved_names(id);
        let editor = McpEditor::new(&wack_core::mcp::McpDraft::of(&server), &saved, cx);
        Some(self.open_mcp_editor(editor, cx))
    }

    fn open_mcp_editor(&mut self, editor: McpEditor, cx: &mut Context<Self>) -> Entity<TextField> {
        let name = editor.name.clone();
        self.mcp_settings.editor = Some(editor);
        self.mcp_settings.error = None;
        self.mcp_settings.confirming_delete = None;
        self.mcp_settings.pending_scope = None;
        self.watch_mcp_editor(cx);
        cx.notify();
        name
    }

    pub fn close_mcp_editor(&mut self, cx: &mut Context<Self>) {
        self.mcp_settings.editor = None;
        self.mcp_settings.confirming_delete = None;
        self.mcp_subscriptions.clear();
        cx.notify();
    }

    fn mcp_editor(&mut self, change: impl FnOnce(&mut McpEditor), cx: &mut Context<Self>) {
        if let Some(editor) = self.mcp_settings.editor.as_mut() {
            change(editor);
            editor.error = None;
        }
        cx.notify();
    }

    pub fn choose_mcp_transport(&mut self, transport: wack_core::mcp::McpTransport, cx: &mut Context<Self>) {
        self.mcp_editor(|editor| editor.transport = transport, cx);
    }

    /// A new variable or header row. Returns its name field.
    pub fn add_mcp_entry(&mut self, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let mut editor = self.mcp_settings.editor.take()?;
        let field = editor.add_entry(cx);
        self.mcp_settings.editor = Some(editor);
        self.watch_mcp_editor(cx);
        cx.notify();
        Some(field)
    }

    pub fn remove_mcp_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.mcp_editor(
            |editor| {
                let entries = editor.entries_mut();
                if index < entries.len() {
                    entries.remove(index);
                }
            },
            cx,
        );
        self.watch_mcp_editor(cx);
    }

    fn inline_mcp_scope(&self, id: &str) -> Option<wack_core::mcp::McpScope> {
        if let Some((pending_id, scope)) = &self.mcp_settings.pending_scope
            && pending_id == id
        {
            return Some(scope.clone());
        }
        self.store.mcp_server(id).map(|server| server.scope.clone())
    }

    fn apply_inline_mcp_scope(&mut self, id: &str, next: wack_core::mcp::McpScope, cx: &mut Context<Self>) {
        if next.needs_project_choice() {
            self.mcp_settings.pending_scope = Some((id.to_string(), next));
            self.mcp_settings.error = None;
            cx.notify();
            return;
        }
        self.mcp_settings.pending_scope = None;
        let result = self.store.set_mcp_server_scope(id, next);
        self.mcp_result(result, cx);
    }

    pub fn choose_mcp_area(&mut self, server: Option<&str>, area: wack_core::mcp::AreaScope, cx: &mut Context<Self>) {
        match server {
            None => self.mcp_editor(|editor| editor.scope = editor.scope.with_area(area), cx),
            Some(id) => {
                if let Some(current) = self.inline_mcp_scope(id) {
                    self.apply_inline_mcp_scope(id, current.with_area(area), cx);
                }
            }
        }
    }

    pub fn choose_mcp_project_filter(&mut self, server: Option<&str>, filter: wack_core::mcp::ProjectFilter, cx: &mut Context<Self>) {
        match server {
            None => self.mcp_editor(|editor| editor.scope = editor.scope.with_project_filter(filter), cx),
            Some(id) => {
                if let Some(current) = self.inline_mcp_scope(id) {
                    self.apply_inline_mcp_scope(id, current.with_project_filter(filter), cx);
                }
            }
        }
    }

    pub fn toggle_mcp_project(&mut self, server: Option<&str>, id: &str, cx: &mut Context<Self>) {
        match server {
            None => self.mcp_editor(|editor| editor.scope = editor.scope.toggle_project(id), cx),
            Some(server_id) => {
                if let Some(current) = self.inline_mcp_scope(server_id) {
                    self.apply_inline_mcp_scope(server_id, current.toggle_project(id), cx);
                }
            }
        }
    }

    /// Saves the editor's server, closes the editor and tests the connection in the background
    /// when it is new, reconfigured, or has no known tools yet.
    pub fn save_mcp_server(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.mcp_settings.editor.as_ref() else {
            return;
        };
        let previous = editor.editing.as_deref().and_then(|id| {
            Some((self.store.mcp_spec(id)?.fingerprint(), !self.store.mcp_server(id)?.tools.is_empty()))
        });
        let result = editor.draft(cx).and_then(|draft| self.store.save_mcp_server(&draft));
        match result {
            Ok(id) => {
                let should_test = match previous {
                    Some((old_fp, has_tools)) => !has_tools || self.store.mcp_spec(&id).is_some_and(|spec| spec.fingerprint() != old_fp),
                    None => true,
                };
                self.close_mcp_editor(cx);
                self.mcp_changed();
                self.mcp_settings.expanded = Some(id.clone());
                if should_test {
                    self.test_mcp_server(&id, cx);
                }
            }
            Err(message) => {
                if let Some(editor) = self.mcp_settings.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    /// Where Test connection starts a command: the home folder, since Settings has no project.
    fn probe_folder() -> std::path::PathBuf {
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| "/".into())
    }

    /// Connects to the draft as it stands, typed values or saved ones, and shows its tools.
    pub fn test_mcp_draft(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.mcp_settings.editor.as_mut() else {
            return;
        };
        let spec = editor.draft(cx).and_then(|draft| self.store.mcp_draft_spec(&draft));
        let spec = match spec {
            Ok(spec) => spec,
            Err(message) => {
                editor.test = Some(ProbeOutcome::Failed(message.into()));
                cx.notify();
                return;
            }
        };
        editor.testing = true;
        editor.test = None;
        editor.test_serial += 1;
        let (serial, editing) = (editor.test_serial, editor.editing.clone());
        let hub = self.mcp.clone();
        let work = Tokio::spawn(cx, async move {
            let started = std::time::Instant::now();
            let tools = hub.probe(&spec, &Self::probe_folder()).await?;
            Ok::<_, String>((started.elapsed().as_millis() as u64, spec.fingerprint(), tools))
        });
        cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The connection test stopped.".into()));
            let _ = app.update(cx, |app, cx| {
                // A saved server reached the same way keeps the tools for its row.
                if let (Ok((_, fingerprint, tools)), Some(id)) = (&result, &editing) {
                    let _ = app.store.set_mcp_tools(id, fingerprint, tools);
                }
                let Some(editor) = app.mcp_settings.editor.as_mut().filter(|editor| editor.test_serial == serial) else {
                    return;
                };
                editor.testing = false;
                editor.test = Some(match result {
                    Ok((elapsed_ms, _, tools)) => ProbeOutcome::Found { elapsed_ms, tools: tools.iter().map(|tool| (tool.name.clone().into(), tool.read_only())).collect() },
                    Err(message) => ProbeOutcome::Failed(message.into()),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Connects to a saved server in a connection of its own and keeps its tools for its row.
    pub fn test_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(spec) = self.store.mcp_spec(id) else {
            return;
        };
        if matches!(self.mcp_settings.tests.get(id), Some(RowTest::Testing)) {
            return;
        }
        self.mcp_settings.tests.insert(id.to_string(), RowTest::Testing);
        let hub = self.mcp.clone();
        let work = Tokio::spawn(cx, async move {
            let tools = hub.probe(&spec, &Self::probe_folder()).await?;
            Ok::<_, String>((spec.fingerprint(), tools))
        });
        let id = id.to_string();
        cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The connection test stopped.".into()));
            let _ = app.update(cx, |app, cx| {
                let test = match result {
                    Ok((fingerprint, tools)) => {
                        let _ = app.store.set_mcp_tools(&id, &fingerprint, &tools);
                        RowTest::Passed(tools.len())
                    }
                    Err(message) => RowTest::Failed(message.into()),
                };
                app.mcp_settings.tests.insert(id, test);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn mcp_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.mcp_settings.error = result.err().map(Into::into);
        self.mcp_changed();
        cx.notify();
    }

    pub fn toggle_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(enabled) = self.store.mcp_server(id).map(|server| server.enabled) else {
            return;
        };
        let result = self.store.set_mcp_server_enabled(id, !enabled);
        self.mcp_result(result, cx);
    }

    pub fn toggle_mcp_tool(&mut self, id: &str, tool: &str, cx: &mut Context<Self>) {
        let Some(off) = self.store.mcp_server(id).map(|server| server.disabled_tools.iter().any(|name| name == tool)) else {
            return;
        };
        let result = self.store.set_mcp_tool_enabled(id, tool, off);
        self.mcp_result(result, cx);
    }

    pub fn expand_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        let open = self.mcp_settings.expanded.as_deref() != Some(id);
        self.mcp_settings.expanded = open.then(|| id.to_string());
        self.mcp_settings.confirming_delete = None;
        self.mcp_settings.pending_scope = None;
        cx.notify();
    }

    pub fn ask_delete_mcp_server(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.mcp_settings.confirming_delete = id;
        cx.notify();
    }

    pub fn confirm_delete_mcp_server(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.mcp_settings.confirming_delete.take() else {
            return;
        };
        match self.store.delete_mcp_server(&id) {
            Ok(()) => {
                self.mcp_settings.tests.remove(&id);
                if self.mcp_settings.expanded.as_deref() == Some(id.as_str()) {
                    self.mcp_settings.expanded = None;
                }
                if self.mcp_settings.pending_scope.as_ref().is_some_and(|(pending, _)| pending == &id) {
                    self.mcp_settings.pending_scope = None;
                }
                if self.mcp_settings.editor.as_ref().is_some_and(|editor| editor.editing.as_deref() == Some(id.as_str())) {
                    self.close_mcp_editor(cx);
                }
                self.mcp_result(Ok(()), cx);
            }
            Err(message) => self.mcp_result(Err(message), cx),
        }
    }

    fn watch_mcp_editor(&mut self, cx: &mut Context<Self>) {
        self.mcp_subscriptions.clear();
        let Some(editor) = self.mcp_settings.editor.as_ref() else {
            return;
        };
        let args = editor.args.clone();
        for field in editor.fields() {
            let multiline = field == args;
            self.mcp_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter if multiline => field.update(cx, |field, cx| field.insert_line_break(cx)),
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_mcp_server(cx),
                TextFieldEvent::Changed => {
                    if let Some(editor) = app.mcp_settings.editor.as_mut() {
                        editor.error = None;
                    }
                    cx.notify();
                }
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }

    /// Settings › Integrations: the ledger's off switch.
    pub fn toggle_usage_recording(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = self.store.set_usage_recording(!self.store.usage_recording()) {
            self.notice = Some(message.into());
        }
        cx.notify();
    }

    /// Settings › Git: whether coding chats end their commits with a Co-authored-by trailer.
    pub fn toggle_commit_attribution(&mut self, cx: &mut Context<Self>) {
        let mut config = self.store.git_config().clone();
        config.commit_attribution = !config.commit_attribution;
        self.git_error = self.store.set_git_config(config).err().map(Into::into);
        cx.notify();
    }

    /// The user answered chat `chat`'s computer-use card `id`. An Always allow or Never allow is
    /// saved first, and only while the card is still up; if saving fails the card stays, saying
    /// why, so the user can answer again.
    pub fn answer_computer_card(&mut self, chat: &str, id: u64, answer: wack_computer::computer::Answer, cx: &mut Context<Self>) {
        use wack_computer::computer::{Answer, Ask, SavedApp};
        let Some(session) = self.sessions.get(chat).cloned() else { return };
        let Some(computer) = session.read(cx).computer.clone() else { return };
        let Some(card) = computer.card().filter(|card| card.id == id) else { return };
        let app = match &card.ask {
            Ask::App { app, .. } => app,
            // Clipboard and screen cards are this chat's alone: nothing is saved.
            Ask::Clipboard { .. } | Ask::Screen { .. } => {
                computer.answer(id, if matches!(answer, Answer::Allow | Answer::AlwaysAllow) { Answer::Allow } else { Answer::Deny });
                return;
            }
        };
        let saved = match answer {
            Answer::AlwaysAllow => match SavedApp::of(app) {
                Some(saved) => self.store.always_allow_app(saved),
                None => Err(format!("{} has no fixed place on disk, so it can only be allowed for this chat.", app.name)),
            },
            Answer::NeverAllow => match &app.bundle_id {
                Some(bundle_id) => self.store.never_allow_app(bundle_id),
                None => Err(format!("{} has no bundle id to remember it by: deny it instead.", app.name)),
            },
            Answer::Allow | Answer::Deny => Ok(()),
        };
        if let Err(error) = saved {
            session.update(cx, |session, cx| {
                session.computer_error = Some((id, error.into()));
                cx.notify();
            });
            return;
        }
        self.computer.set_settings(self.store.computer_config().settings());
        computer.answer(id, answer);
    }

    /// Settings › Computer use: on or off.
    pub fn toggle_computer_use(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.store.computer_config().enabled;
        self.computer_error = self.store.set_computer_enabled(enabled).err().map(Into::into);
        self.computer.set_settings(self.store.computer_config().settings());
        cx.notify();
    }

    /// Settings › Computer use: an always-allowed app asks again next time.
    pub fn forget_always_allowed(&mut self, path: &str, cx: &mut Context<Self>) {
        self.computer_error = self.store.forget_always_allowed(path).err().map(Into::into);
        self.computer.set_settings(self.store.computer_config().settings());
        cx.notify();
    }

    /// Settings › Computer use: a never-allowed app may be asked about again.
    pub fn forget_never_allowed(&mut self, bundle_id: &str, cx: &mut Context<Self>) {
        self.computer_error = self.store.forget_never_allowed(bundle_id).err().map(Into::into);
        self.computer.set_settings(self.store.computer_config().settings());
        cx.notify();
    }

    /// Settings › Computer use: asks macOS for a permission (resetting a grant that went stale
    /// when the app was rebuilt) and opens its pane in System Settings.
    pub fn request_computer_permission(&mut self, screen: bool, cx: &mut Context<Self>) {
        use wack_computer::native::permissions::{self, Pane};
        let (pane, granted) = if screen {
            (Pane::ScreenRecording, permissions::screen_recording_preflight())
        } else {
            (Pane::Accessibility, permissions::accessibility())
        };
        let identifier = wack_computer::native::apps::own_bundle_id();
        self.computer_error = permissions::request(pane, identifier.as_deref(), granted).err().map(Into::into);
        cx.notify();
    }

    /// Settings › Browser: whether the Browser pane opens when the agent opens a tab.
    pub fn toggle_agent_opens_pane(&mut self, cx: &mut Context<Self>) {
        let mut config = self.store.browser_config().clone();
        config.agent_opens_pane = !config.agent_opens_pane;
        self.browser_error = self.store.set_browser_config(config).err().map(Into::into);
        cx.notify();
    }

    /// Settings › Browser: the size new tabs open at.
    pub fn choose_default_viewport(&mut self, viewport: wack_browser::Viewport, cx: &mut Context<Self>) {
        let mut config = self.store.browser_config().clone();
        config.default_viewport = viewport;
        self.browser_error = self.store.set_browser_config(config).err().map(Into::into);
        wack_browser::set_default_viewport(self.store.browser_config().default_viewport);
        cx.notify();
    }

    /// Settings › Browser: shows or takes back the confirm for clearing the browsing data.
    pub fn ask_clear_browsing_data(&mut self, asking: bool, cx: &mut Context<Self>) {
        self.browser_clearing = if asking { crate::browser_page::Clearing::Confirming } else { crate::browser_page::Clearing::Idle };
        cx.notify();
    }

    /// Settings › Browser: clears the shared profile, as far as the running browser allows.
    pub fn clear_browsing_data(&mut self, cx: &mut Context<Self>) {
        use crate::browser_page::Clearing;
        self.browser_clearing = match wack_browser::cef::clear_browsing_data() {
            Ok(wack_browser::cef::Cleared::Everything) => Clearing::Cleared,
            Ok(wack_browser::cef::Cleared::CookiesNow) => Clearing::Pending,
            Err(message) => Clearing::Failed(message.into()),
        };
        cx.notify();
    }

    pub fn computer_data(&self) -> crate::computer_page::ComputerData {
        let config = self.store.computer_config();
        let permissions = self.computer.desktop().permissions();
        crate::computer_page::ComputerData {
            enabled: config.enabled,
            accessibility: permissions.accessibility,
            screen_recording: permissions.screen_recording,
            always: config.always.iter().map(|app| (app.name.clone().into(), app.path.clone().into())).collect(),
            never: config.never.iter().map(|id| id.clone().into()).collect(),
            error: self.computer_error.clone(),
        }
    }

    pub fn browser_data(&self) -> crate::browser_page::BrowserData {
        use crate::browser_page::Clearing;
        let config = self.store.browser_config();
        // A clear left for the next start still shows after leaving the page.
        let clearing = match &self.browser_clearing {
            Clearing::Idle if wack_browser::cef::clearing_pending(&self.store.browser_folder()) => Clearing::Pending,
            other => other.clone(),
        };
        crate::browser_page::BrowserData {
            agent_opens_pane: config.agent_opens_pane,
            default_viewport: config.default_viewport,
            clearing,
            version: wack_browser::cef::version().into(),
            error: self.browser_error.clone(),
        }
    }

    /// Changes how the app looks; the theme follows at once.
    pub fn set_appearance(&mut self, change: impl FnOnce(&mut Appearance), cx: &mut Context<Self>) {
        let mut appearance = self.store.appearance().clone();
        change(&mut appearance);
        match self.store.set_appearance(appearance) {
            Ok(()) => {
                self.appearance_notice = None;
                theme::apply(self.store.appearance(), cx);
            }
            Err(message) => self.appearance_notice = Some(message.into()),
        }
        cx.notify();
    }

    /// Reads a picture off the UI thread and makes it the backdrop.
    pub fn import_backdrop(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let folder = self.store.backgrounds_folder();
        self.importing = true;
        self.appearance_notice = None;
        cx.notify();
        let import = cx.background_spawn(async move { wack_core::appearance::import_backdrop(&path, &folder) });
        cx.spawn(async move |app, cx| {
            let result = import.await;
            let _ = app.update(cx, |app, cx| {
                app.importing = false;
                match result {
                    // It joins the library and shows, keeping the look of the picture it replaces.
                    Ok(backdrop) => app.set_appearance(|appearance| appearance.add(backdrop), cx),
                    Err(message) => {
                        app.appearance_notice = Some(message.into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// The backdrop to draw, if there is one, where a drag in the position editor has it now.
    pub fn backdrop(&self) -> Option<Picture> {
        self.store.appearance().backdrop().map(|backdrop| {
            let picture = self.picture(backdrop);
            match self.crop_preview {
                Some(focus) => Picture { focus, ..picture },
                None => picture,
            }
        })
    }

    /// Every picture in the library, for Settings › Appearance.
    pub fn pictures(&self) -> Vec<Thumbnail> {
        let appearance = self.store.appearance();
        appearance
            .pictures
            .iter()
            .map(|backdrop| Thumbnail {
                file: backdrop.file.clone().into(),
                shown: appearance.shown.as_deref() == Some(backdrop.file.as_str()),
                theme: backdrop.theme,
                picture: self.picture(backdrop),
                // A picture whose thumbnail could not be made is drawn from the picture itself.
                thumb: Some(self.store.backdrop_thumb_path(backdrop))
                    .filter(|thumb| thumb.exists())
                    .unwrap_or_else(|| self.store.backdrop_path(backdrop))
                    .to_string_lossy()
                    .to_string()
                    .into(),
            })
            .collect()
    }

    fn picture(&self, backdrop: &wack_core::Backdrop) -> Picture {
        Picture {
            path: self.store.backdrop_path(backdrop).to_string_lossy().to_string().into(),
            soft_path: self.store.backdrop_soft_path(backdrop).to_string_lossy().to_string().into(),
            width: backdrop.width,
            height: backdrop.height,
            focus: (backdrop.focus_x, backdrop.focus_y),
            zoom: backdrop.zoom as f32 / 100.,
            dim: backdrop.dim as f32 / 100.,
            blur: backdrop.blur as f32,
            texture: backdrop.texture,
            shade: backdrop.shade.unwrap_or([0, 255]),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn session(&self, chat: &str) -> Option<&Entity<ChatSession>> {
        self.sessions.get(chat)
    }

    pub fn is_open(&self, project: &str) -> bool {
        self.open_projects.contains(project)
    }

    pub fn tabs(&self) -> &[String] {
        &self.tabs
    }

    /// The open tabs of the area on screen, in order: each area keeps its own.
    pub fn area_tabs(&self) -> Vec<String> {
        let area = self.area();
        self.tabs.iter().filter(|id| self.store.chat(id).is_some_and(|chat| chat.kind == area)).cloned().collect()
    }

    /// Which area a selection is in; `None` for Settings, which belongs to neither.
    fn area_of(&self, selection: &Selection) -> Option<ChatKind> {
        match selection {
            Selection::Welcome | Selection::NewChat(_) => Some(ChatKind::Code),
            Selection::NewChatArea => Some(ChatKind::Chat),
            Selection::Chat(id) => self.store.chat(id).map(|chat| chat.kind),
            Selection::Settings => None,
        }
    }

    /// The area on screen: Settings shows over the one it was opened from.
    pub fn area(&self) -> ChatKind {
        self.area_of(&self.selection).or_else(|| self.area_of(&self.before_settings)).unwrap_or(self.store.ui().area)
    }

    /// Where an area opens when it has nowhere to come back to: a new chat (in the project last
    /// worked in, for Code), or the welcome while there is no project.
    fn area_home(&self, area: ChatKind) -> Selection {
        match area {
            ChatKind::Chat => Selection::NewChatArea,
            ChatKind::Code => {
                let ui = self.store.ui();
                match ui.last_project.as_deref().and_then(|id| self.store.project(id)).or_else(|| self.store.last_used_project()).or_else(|| self.store.projects().first()) {
                    Some(project) => Selection::NewChat(project.id.clone()),
                    None => Selection::Welcome,
                }
            }
        }
    }

    /// Whether a parked selection can still be shown.
    fn still_there(&self, selection: &Selection) -> bool {
        match selection {
            Selection::Chat(id) => self.store.chat(id).is_some_and(|chat| !chat.is_archived()),
            Selection::NewChat(project) => self.store.project(project).is_some(),
            Selection::Welcome => self.store.projects().is_empty(),
            Selection::NewChatArea | Selection::Settings => true,
        }
    }

    /// ⌥⌘1 and ⌥⌘2, and the sidebar's switch: shows `area` where it was left. Settings closes.
    pub fn enter_area(&mut self, area: ChatKind, cx: &mut Context<Self>) {
        if self.selection == Selection::Settings {
            self.form.put_key_away(cx);
        }
        if self.area() == area {
            if self.selection == Selection::Settings {
                self.close_settings(cx);
            }
            return;
        }
        let target = self.parked.get(&area).cloned().filter(|parked| self.still_there(parked)).unwrap_or_else(|| self.area_home(area));
        self.select(target, cx);
    }

    /// True while a chat of `area` waits for the user to answer its question: the area switch
    /// marks the other area with a dot.
    pub fn area_waiting(&self, area: ChatKind, cx: &App) -> bool {
        self.sessions.iter().any(|(id, session)| {
            session.read(cx).asking.is_some() && self.store.chat(id).is_some_and(|chat| chat.kind == area && !chat.is_archived())
        })
    }

    /// An idea chip on the Chat area's new chat: its words in the composer, not sent.
    pub fn use_idea(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(idea) = crate::chat_area::IDEAS.get(index) else {
            return;
        };
        if self.selection != Selection::NewChatArea {
            return;
        }
        self.draft.update(cx, |draft, cx| draft.set_text(idea.prompt, cx));
        cx.notify();
    }

    /// The Chat chat on screen's scratchpad.
    pub fn current_scratchpad(&self) -> Option<PathBuf> {
        let Selection::Chat(id) = &self.selection else { return None };
        let chat = self.store.chat(id).filter(|chat| chat.is_chat_area())?;
        Some(self.store.scratch_folder(chat))
    }

    /// What a chat's right-click menu needs to know of it.
    pub fn chat_facts(&self, menu: &ChatMenu) -> Option<crate::context_menu::ChatFacts> {
        let chat = self.store.chat(&menu.chat)?;
        Some(crate::context_menu::ChatFacts {
            chat: menu.chat.clone(),
            chat_area: chat.is_chat_area(),
            has_file: self.store.session_path(chat).exists(),
            tab_open: self.tabs.contains(&menu.chat),
            sidebar: menu.sidebar,
        })
    }

    /// The folder a chat works in: its project's, or a Chat area chat's scratchpad.
    fn chat_folder(&self, id: &str) -> Option<PathBuf> {
        let chat = self.store.chat(id)?;
        if chat.is_chat_area() {
            return Some(self.store.scratch_folder(chat));
        }
        chat.project.as_deref().and_then(|project| self.store.project(project)).map(|project| project.path.clone())
    }

    pub fn open_chat_menu(&mut self, menu: ChatMenu, cx: &mut Context<Self>) {
        self.chat_menu = Some(menu);
        cx.notify();
    }

    pub fn close_chat_menu(&mut self, cx: &mut Context<Self>) {
        if self.chat_menu.take().is_some() {
            cx.notify();
        }
    }

    /// A chat menu's row, but Rename, which the shell starts where there is a window.
    pub fn chat_menu_pick(&mut self, chat: &str, pick: crate::actions::ChatPick, cx: &mut Context<Self>) {
        use crate::actions::ChatPick;
        match pick {
            ChatPick::Rename => {}
            ChatPick::Reveal => {
                if let Some(folder) = self.chat_folder(chat) {
                    let _ = std::fs::create_dir_all(&folder);
                    crate::memory_page::reveal(&folder);
                }
            }
            ChatPick::RevealFile => {
                if let Some(listed) = self.store.chat(chat) {
                    crate::memory_page::reveal(&self.store.session_path(listed));
                }
            }
            ChatPick::CopyPath => {
                if let Some(folder) = self.chat_folder(chat) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(folder.display().to_string()));
                }
            }
            ChatPick::CloseTab => self.close_tab(chat, cx),
            ChatPick::Archive => self.archive_chat(chat, false, cx),
        }
    }

    pub fn reveal_project(&self) {
        if let Some(project) = self.current_project() {
            crate::memory_page::reveal(&project.path);
        }
    }

    pub fn reveal_scratchpad(&self) {
        if let Some(folder) = self.current_scratchpad() {
            let _ = std::fs::create_dir_all(&folder);
            crate::memory_page::reveal(&folder);
        }
    }

    /// A file chip under a Chat reply. Its path came from the model, so it is shown only when it
    /// is, on disk with links followed, inside the scratchpad of the chat on screen.
    pub fn reveal_scratchpad_file(&self, path: &Path) {
        let Some(folder) = self.current_scratchpad().and_then(|folder| folder.canonicalize().ok()) else { return };
        if let Ok(real) = path.canonicalize()
            && real.starts_with(&folder)
            && real != folder
        {
            crate::memory_page::reveal(&real);
        }
    }

    pub fn copy_scratchpad_path(&self, cx: &mut Context<Self>) {
        if let Some(folder) = self.current_scratchpad() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(folder.display().to_string()));
        }
    }

    pub fn sidebar_hidden(&self) -> bool {
        self.sidebar_hidden
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_hidden = !self.sidebar_hidden;
        self.save_ui();
        cx.notify();
    }

    /// The chat in front, if a chat is on screen.
    pub fn current_chat(&self) -> Option<&Entity<ChatSession>> {
        match &self.selection {
            Selection::Chat(id) => self.sessions.get(id),
            _ => None,
        }
    }

    /// Closes a chat's tab. The chat stays in the sidebar, and its session keeps running.
    pub fn close_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab == id) else {
            return;
        };
        if self.selection == Selection::Chat(id.to_string()) {
            // The neighbour on the right takes its place, as in a browser; else the one before.
            // Only the area's own tabs are neighbours.
            let tabs = self.area_tabs();
            let at = tabs.iter().position(|tab| tab == id).unwrap_or(0);
            let next = tabs.get(at + 1).or_else(|| at.checked_sub(1).and_then(|before| tabs.get(before))).cloned();
            self.tabs.remove(index);
            match next {
                Some(next) => self.select(Selection::Chat(next), cx),
                None => {
                    let chat = self.store.chat(id).cloned();
                    match chat {
                        Some(chat) if chat.is_chat_area() => self.select(Selection::NewChatArea, cx),
                        Some(Chat { project: Some(project), .. }) => self.select(Selection::NewChat(project), cx),
                        _ => self.select_somewhere(cx),
                    }
                }
            }
        } else {
            self.tabs.remove(index);
        }
        self.save_ui();
        cx.notify();
    }

    /// Steps through the tabs, wrapping round; from the new-chat screen, to the last tab.
    pub fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let tabs = self.area_tabs();
        if tabs.is_empty() {
            return;
        }
        let count = tabs.len();
        let next = match &self.selection {
            Selection::Chat(id) => match tabs.iter().position(|tab| tab == id) {
                Some(index) if forward => (index + 1) % count,
                Some(index) => (index + count - 1) % count,
                None => 0,
            },
            _ => count - 1,
        };
        let id = tabs[next].clone();
        self.select(Selection::Chat(id), cx);
    }

    fn save_ui(&mut self) {
        // Settings has no project of its own, so the one last worked in stays.
        let last_project = self
            .current_project()
            .map(|project| project.id.clone())
            .or_else(|| self.store.ui().last_project.clone().filter(|id| self.store.project(id).is_some()));
        let window = self.pending_window.take().or_else(|| self.store.ui().window.clone());
        self.saving_window = None;
        let ui = Ui { sidebar_hidden: self.sidebar_hidden, last_project, area: self.area(), window };
        if let Err(message) = self.store.set_ui(ui) {
            self.notice = Some(message.into());
        }
    }

    /// Records a normal-mode window move or resize, saving once the drag settles (and on exit).
    pub fn record_window(&mut self, window: WindowState, cx: &mut Context<Self>) {
        let current = self.pending_window.as_ref().or(self.store.ui().window.as_ref());
        if current == Some(&window) {
            return;
        }
        self.pending_window = Some(window);
        self.saving_window = Some(cx.spawn(async move |app, cx| {
            cx.background_executor().timer(LAYOUT_SAVE_DELAY).await;
            let _ = app.update(cx, |app, _| app.flush_window());
        }));
    }

    /// Drops an un-flushed move/resize (for example a transition frame while entering fullscreen).
    pub fn cancel_pending_window(&mut self) {
        self.pending_window = None;
        self.saving_window = None;
    }

    /// Writes any pending window geometry to `state.json`.
    pub fn flush_window(&mut self) {
        self.saving_window = None;
        let Some(window) = self.pending_window.take() else {
            return;
        };
        if let Err(message) = self.store.set_window(window) {
            self.notice = Some(message.into());
        }
    }

    /// Saves a chat's pane layout once it has stopped changing for a moment.
    fn layout_changed(&mut self, chat: &str, cx: &mut Context<Self>) {
        self.unsaved_layouts.insert(chat.to_string());
        self.saving_layouts = Some(cx.spawn(async move |app, cx| {
            cx.background_executor().timer(LAYOUT_SAVE_DELAY).await;
            let _ = app.update(cx, |app, cx| app.save_layouts(cx));
        }));
    }

    fn save_layouts(&mut self, cx: &mut Context<Self>) {
        for chat in std::mem::take(&mut self.unsaved_layouts) {
            let Some(session) = self.sessions.get(&chat) else {
                continue;
            };
            let layout = &session.read(cx).panes.layout;
            let value = (*layout != Layout::default()).then(|| serde_json::to_value(layout).ok()).flatten();
            if let Err(message) = self.store.set_layout(&chat, value) {
                self.notice = Some(message.into());
            }
        }
        self.saving_layouts = None;
    }

    pub fn branch(&self, project: &str) -> Option<&str> {
        self.branches.get(project).and_then(|branch| branch.as_deref())
    }

    /// The project the user is in: the selected chat's, or the one a new chat is for.
    pub fn current_project(&self) -> Option<&Project> {
        let id = match &self.selection {
            Selection::NewChat(project) => Some(project.as_str()),
            Selection::Chat(chat) => self.store.chat(chat).and_then(|chat| chat.project.as_deref()),
            Selection::Welcome | Selection::NewChatArea | Selection::Settings => None,
        };
        id.and_then(|id| self.store.project(id))
    }

    /// The text field that should have focus on the current screen.
    pub fn active_field(&self, cx: &App) -> Option<Entity<TextField>> {
        if let Some(editor) = &self.command_editor {
            return Some(editor.input.clone());
        }
        match &self.selection {
            Selection::NewChat(_) | Selection::NewChatArea => Some(self.draft.clone()),
            Selection::Chat(chat) => self.sessions.get(chat).map(|session| session.read(cx).composer.clone()),
            Selection::Settings => match self.settings_section {
                Section::Providers => self.form.open.then(|| self.form.name.clone()),
                Section::Memory => self.memory.editor.as_ref().map(|editor| editor.title.clone()),
                Section::Web => self.web.editor.as_ref().map(|editor| editor.name.clone()),
                Section::Mcp => self.mcp_settings.editor.as_ref().map(|editor| editor.name.clone()),
                Section::Skills => self.skills.editor.as_ref().map(|editor| editor.name.clone()),
                Section::Commands => self.templates.editor.as_ref().map(|editor| editor.name.clone()),
                Section::Subagents => self.subagents.editor.as_ref().map(|editor| editor.name.clone()),
                _ => None,
            },
            Selection::Welcome => None,
        }
    }

    pub fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.project_picker = false;
        // Leaving an area parks where it was, and its new chat's draft, so coming back finds
        // both; Settings shows over an area without leaving it.
        // Leaving a new chat keeps its words in its tab.
        if Self::draft_target(&self.selection).is_some() {
            self.park_draft(cx);
        }
        let leaving = if self.selection == Selection::Settings { self.before_settings.clone() } else { self.selection.clone() };
        let (from, to) = (self.area_of(&leaving), self.area_of(&selection));
        if let (Some(from), Some(to)) = (from, to)
            && from != to
        {
            self.parked.insert(from, leaving.clone());
            // Each area is searched and archived on its own.
            self.search.update(cx, |search, cx| search.set_text("", cx));
            self.archived_open = false;
            self.confirming = None;
            self.confirming_all = false;
            let draft = self.draft.read(cx).text().to_string();
            self.parked_drafts.insert(from, draft);
            let back = self.parked_drafts.remove(&to).unwrap_or_default();
            self.draft.update(cx, |field, cx| field.set_text(&back, cx));
            self.draft_attachments.clear();
            self.draft_files.clear();
        }
        match &selection {
            Selection::Chat(id) => {
                let Some(chat) = self.store.chat(id).cloned() else {
                    return;
                };
                let session = self.ensure_session(&chat, cx);
                session.update(cx, |session, cx| {
                    if session.unread {
                        session.unread = false;
                        cx.notify();
                    }
                    // Coming back to a question puts focus back on it.
                    if let Some(asking) = &session.asking {
                        session.focus_request = Some(asking.focus.clone());
                    }
                });
                if let Some(project) = &chat.project {
                    self.open_projects.insert(project.clone());
                    self.look_up_branch(project, cx);
                }
                if !self.tabs.contains(id) {
                    self.tabs.push(id.clone());
                }
            }
            Selection::NewChat(project) => {
                if self.store.project(project).is_none() {
                    return;
                }
                self.open_projects.insert(project.clone());
                self.look_up_branch(project, cx);
                let words = self.new_chat_words(project);
                self.draft.update(cx, |draft, cx| draft.set_placeholder(words, cx));
            }
            Selection::NewChatArea => {
                // A fresh visit gets its own idea chips.
                if self.selection != Selection::NewChatArea {
                    self.hero_visit += 1;
                }
                self.draft.update(cx, |draft, cx| draft.set_placeholder(CHAT_AREA_PLACEHOLDER, cx));
            }
            Selection::Settings => {
                if self.selection != Selection::Settings {
                    self.before_settings = self.selection.clone();
                }
                if self.settings_section == Section::Providers {
                    self.ensure_catalog(cx);
                }
                if self.settings_section == Section::Memory {
                    self.refresh_memory();
                }
                if self.settings_section == Section::Skills {
                    self.refresh_skills();
                }
                if self.settings_section == Section::Commands {
                    self.refresh_templates();
                }
                if self.settings_section == Section::Subagents {
                    self.refresh_subagents();
                }
                if self.settings_section == Section::Git {
                    self.git_error = None;
                }
                // A confirm left open is not still waiting when Settings opens again.
                if matches!(self.browser_clearing, crate::browser_page::Clearing::Confirming) {
                    self.browser_clearing = Default::default();
                }
            }
            Selection::Welcome => {}
        }
        match Self::draft_target(&selection) {
            // A new chat on screen is always one of the draft tabs: the one it was, else the
            // project's latest, else a fresh one.
            Some(project) => {
                let current = self.active_draft.filter(|key| self.drafts.iter().any(|draft| draft.key == *key && draft.project == project));
                let key = match current.or_else(|| self.drafts.iter().rev().find(|draft| draft.project == project).map(|draft| draft.key)) {
                    Some(key) => key,
                    None => self.add_draft(project),
                };
                self.load_draft(key, cx);
            }
            // A blank new chat left for a chat goes; one with words in it stays a tab.
            None if matches!(selection, Selection::Chat(_)) => {
                self.drafts.retain(|draft| !draft.blank());
                self.active_draft = None;
            }
            None => {}
        }
        if self.selection != selection {
            self.close_slash(cx);
        }
        self.notice = None;
        self.menu = None;
        self.command_editor = None;
        self.selection = selection;
        self.save_ui();
        cx.notify();
    }

    /// The project a new-chat selection writes in (`None` for the Chat area); `None` for any
    /// other selection.
    fn draft_target(selection: &Selection) -> Option<Option<String>> {
        match selection {
            Selection::NewChat(project) => Some(Some(project.clone())),
            Selection::NewChatArea => Some(None),
            _ => None,
        }
    }

    fn add_draft(&mut self, project: Option<String>) -> u64 {
        let key = self.next_draft;
        self.next_draft += 1;
        self.drafts.push(DraftTab { key, project, text: String::new(), attachments: Vec::new(), files: Vec::new() });
        key
    }

    /// Keeps what the composer holds in the draft tab on screen.
    fn park_draft(&mut self, cx: &App) {
        let text = self.draft.read(cx).text().to_string();
        let attachments = self.draft_attachments.clone();
        let files = self.draft_files.clone();
        if let Some(key) = self.active_draft
            && let Some(draft) = self.drafts.iter_mut().find(|draft| draft.key == key)
        {
            draft.text = text;
            draft.attachments = attachments;
            draft.files = files;
        }
    }

    /// Puts a draft tab's words back in the composer, and makes it the one on screen.
    fn load_draft(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(draft) = self.drafts.iter().find(|draft| draft.key == key).cloned() else { return };
        self.draft.update(cx, |field, cx| field.set_text(&draft.text, cx));
        self.draft_attachments = draft.attachments;
        self.draft_files = draft.files;
        self.active_draft = Some(key);
    }

    /// The draft tabs of the area on screen: their keys, and which is on screen.
    pub fn draft_tabs(&self) -> Vec<(u64, bool)> {
        let on_draft = Self::draft_target(&self.selection).is_some();
        let area = self.area();
        self.drafts
            .iter()
            .filter(|draft| if draft.project.is_some() { area == ChatKind::Code } else { area == ChatKind::Chat })
            .map(|draft| (draft.key, on_draft && self.active_draft == Some(draft.key)))
            .collect()
    }

    /// "+" (⌘N) on a new chat: another new chat beside it, in the same place.
    fn another_new_chat(&mut self, cx: &mut Context<Self>) {
        let Some(project) = Self::draft_target(&self.selection) else { return };
        self.park_draft(cx);
        let key = self.add_draft(project);
        self.load_draft(key, cx);
        self.select(self.selection.clone(), cx);
    }

    /// A draft tab clicked: its new chat comes back on screen with its words.
    pub fn select_draft(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(project) = self.drafts.iter().find(|draft| draft.key == key).map(|draft| draft.project.clone()) else { return };
        if Self::draft_target(&self.selection).is_some() {
            self.park_draft(cx);
        }
        self.load_draft(key, cx);
        self.select(project.map_or(Selection::NewChatArea, Selection::NewChat), cx);
    }

    /// A draft tab closed: its words go. If it was on screen, another draft tab of the area takes
    /// its place, else the area's last chat, else a fresh new chat.
    pub fn close_draft(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(at) = self.drafts.iter().position(|draft| draft.key == key) else { return };
        let closed = self.drafts.remove(at);
        let on_screen = Self::draft_target(&self.selection).is_some() && self.active_draft == Some(key);
        if !on_screen {
            cx.notify();
            return;
        }
        self.active_draft = None;
        let next = self.draft_tabs().last().map(|(key, _)| *key);
        if let Some(next) = next {
            self.load_draft(next, cx);
            let project = self.drafts.iter().find(|draft| draft.key == next).and_then(|draft| draft.project.clone());
            self.select(project.map_or(Selection::NewChatArea, Selection::NewChat), cx);
        } else if let Some(last) = self.area_tabs().last().cloned() {
            self.draft.update(cx, |field, cx| field.set_text("", cx));
            self.draft_attachments.clear();
            self.draft_files.clear();
            self.select(Selection::Chat(last), cx);
        } else {
            let key = self.add_draft(closed.project.clone());
            self.load_draft(key, cx);
            self.select(self.selection.clone(), cx);
        }
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.selection == Selection::Settings {
            self.form.put_key_away(cx);
            self.close_model_suggestions(cx);
            let back = self.before_settings.clone();
            self.selection = Selection::Welcome;
            self.select(back, cx);
            if self.selection == Selection::Welcome {
                self.select_somewhere(cx);
            }
        }
    }

    /// A new chat in `project`, or in the current project when `None`; in the Chat area, a new
    /// chat there.
    pub fn new_chat(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        // On a new chat already, "+" opens another beside it, as version 2's did.
        if let Some(here) = Self::draft_target(&self.selection)
            && (project.is_none() || project == here)
        {
            self.another_new_chat(cx);
            return;
        }
        if project.is_none() && self.area() == ChatKind::Chat {
            self.select(Selection::NewChatArea, cx);
            return;
        }
        let project = project
            .or_else(|| self.current_project().map(|project| project.id.clone()))
            .or_else(|| self.store.projects().first().map(|project| project.id.clone()));
        match project {
            Some(project) => self.select(Selection::NewChat(project), cx),
            None => self.select(Selection::Welcome, cx),
        }
    }

    pub fn toggle_project_picker(&mut self, cx: &mut Context<Self>) {
        self.project_picker = !self.project_picker && matches!(self.selection, Selection::NewChat(_));
        cx.notify();
    }

    pub fn close_project_picker(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.project_picker) {
            cx.notify();
        }
    }

    /// A project picked in the New Chat screen's list: the new chat on screen moves to it, with
    /// its words, pictures and files, and its draft tab with it.
    pub fn choose_new_chat_project(&mut self, project: String, cx: &mut Context<Self>) {
        self.project_picker = false;
        if !matches!(self.selection, Selection::NewChat(_)) || self.store.project(&project).is_none() {
            cx.notify();
            return;
        }
        if let Some(key) = self.active_draft
            && let Some(draft) = self.drafts.iter_mut().find(|draft| draft.key == key)
        {
            draft.project = Some(project.clone());
        }
        self.select(Selection::NewChat(project), cx);
        cx.notify();
    }

    pub fn add_project(&mut self, path: &Path, cx: &mut Context<Self>) {
        match self.store.add_project(path) {
            Ok(project) => self.select(Selection::NewChat(project.id), cx),
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
            }
        }
    }

    pub fn toggle_project(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.open_projects.remove(id) {
            self.open_projects.insert(id.to_string());
        }
        cx.notify();
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool, cx: &mut Context<Self>) {
        if let Err(message) = self.store.set_pinned(id, pinned) {
            self.notice = Some(message.into());
        }
        cx.notify();
    }

    /// Takes the project out of WackCode, with its chats. The folder itself is not touched.
    pub fn remove_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let project_path = self.store.project(id).map(|project| project.path.clone());
        let chats: Vec<String> = self.store.chats(id).iter().map(|chat| chat.id.clone()).collect();
        if let Err(message) = self.store.remove_project(id) {
            self.notice = Some(message.into());
            cx.notify();
            return;
        }
        self.mcp_settings.pending_scope = None;
        self.mcp_changed();
        self.new_chat_panes.remove(id);
        if self.command_editor.as_ref().is_some_and(|editor| editor.project_id == id) {
            self.command_editor = None;
        }
        if let Some(path) = project_path {
            self.runs.stop_project(&path);
            self.workspace_files.remove(&path);
        }
        for chat in chats {
            self.drop_session(&chat);
        }
        self.open_projects.remove(id);
        let gone = match &self.selection {
            Selection::NewChat(project) => project == id,
            Selection::Chat(chat) => self.store.chat(chat).is_none(),
            _ => false,
        };
        if gone {
            self.select_somewhere(cx);
        }
        self.save_ui();
        cx.notify();
    }

    pub fn delete_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        let project = self.store.chat(id).and_then(|chat| chat.project.clone());
        let chat_area = self.store.chat(id).is_some_and(Chat::is_chat_area);
        self.drop_session(id);
        if let Err(message) = self.store.delete_chat(id) {
            self.notice = Some(message.into());
        }
        // A chat a suggestion card linked to is gone: the card is actionable again.
        if let Some(project) = &project {
            let project = project.clone();
            for (chat_id, session) in self.sessions.clone() {
                if self.store.chat(&chat_id).is_some_and(|chat| chat.project.as_ref() == Some(&project)) {
                    let chat = self.store.chat(&chat_id).unwrap().clone();
                    let view = self.suggestion_view(&chat);
                    session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
                }
            }
        }
        if self.selection == Selection::Chat(id.to_string()) {
            match project {
                _ if chat_area => self.select(Selection::NewChatArea, cx),
                Some(project) => self.select(Selection::NewChat(project), cx),
                None => self.select_somewhere(cx),
            }
        }
        self.parked.retain(|_, parked| *parked != Selection::Chat(id.to_string()));
        self.save_ui();
        cx.notify();
    }

    /// The sidebar's search, as words to match; none while it is empty.
    pub fn search_terms(&self, cx: &App) -> Vec<String> {
        crate::sidebar::search_terms(self.search.read(cx).text())
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.set_text("", cx));
        cx.notify();
    }

    pub fn toggle_archived(&mut self, cx: &mut Context<Self>) {
        self.archived_open = !self.archived_open;
        self.confirming = None;
        self.confirming_all = false;
        if self.selection == Selection::Settings {
            self.close_settings(cx);
        }
        cx.notify();
    }

    /// Whether `chat`'s `which` control was already clicked once; if not, it asks now, for a
    /// few seconds.
    fn confirmed(&mut self, chat: &str, which: RowConfirm, cx: &mut Context<Self>) -> bool {
        if self.confirming.as_ref().is_some_and(|(asking, what)| asking == chat && *what == which) {
            self.confirming = None;
            return true;
        }
        self.confirming = Some((chat.to_string(), which));
        self.confirming_all = false;
        let asked = (chat.to_string(), which);
        cx.spawn(async move |app, cx| {
            cx.background_executor().timer(CONFIRM_FOR).await;
            let _ = app.update(cx, |app, cx| {
                if app.confirming.as_ref() == Some(&asked) {
                    app.confirming = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
        false
    }

    /// A row's Archive: asks once, then puts the chat in its area's Archived view. A run in it
    /// stops, and it leaves the tabs; on screen, its area's new chat takes its place.
    pub fn archive_chat(&mut self, id: &str, ask: bool, cx: &mut Context<Self>) {
        if ask && !self.confirmed(id, RowConfirm::Archive, cx) {
            return;
        }
        let Some(chat) = self.store.chat(id).cloned() else { return };
        self.stop(id, cx);
        if let Err(message) = self.store.set_archived(id, true) {
            self.notice = Some(message.into());
            cx.notify();
            return;
        }
        self.tabs.retain(|tab| tab != id);
        if self.selection == Selection::Chat(id.to_string()) {
            match &chat.project {
                Some(project) => self.select(Selection::NewChat(project.clone()), cx),
                None => self.select(Selection::NewChatArea, cx),
            }
        }
        self.parked.retain(|_, parked| *parked != Selection::Chat(id.to_string()));
        self.save_ui();
        cx.notify();
    }

    /// Back to the list, from an Archived row (asking once) or the chat's own footer.
    pub fn unarchive_chat(&mut self, id: &str, ask: bool, cx: &mut Context<Self>) {
        if ask && !self.confirmed(id, RowConfirm::Unarchive, cx) {
            return;
        }
        if let Err(message) = self.store.set_archived(id, false) {
            self.notice = Some(message.into());
        }
        if self.store.archived_chats(self.area()).is_empty() {
            self.archived_open = false;
        }
        cx.notify();
    }

    /// An Archived row's Delete: asks once, then deletes the chat and its files for good.
    pub fn delete_archived_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.confirmed(id, RowConfirm::Delete, cx) {
            return;
        }
        self.delete_chat(id, cx);
        if self.store.archived_chats(self.area()).is_empty() {
            self.archived_open = false;
        }
    }

    /// The Archived view's Delete all: asks once, then deletes every archived chat of the area.
    pub fn delete_all_archived(&mut self, cx: &mut Context<Self>) {
        if !self.confirming_all {
            self.confirming_all = true;
            self.confirming = None;
            cx.spawn(async move |app, cx| {
                cx.background_executor().timer(CONFIRM_FOR).await;
                let _ = app.update(cx, |app, cx| {
                    app.confirming_all = false;
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        self.confirming_all = false;
        let ids: Vec<String> = self.store.archived_chats(self.area()).iter().map(|chat| chat.id.clone()).collect();
        for id in ids {
            self.delete_chat(&id, cx);
        }
        self.archived_open = false;
        cx.notify();
    }

    /// After the selected thing went away: the Chat area's new chat, a new chat in the first
    /// project, or the welcome.
    fn select_somewhere(&mut self, cx: &mut Context<Self>) {
        if self.area() == ChatKind::Chat {
            self.select(Selection::NewChatArea, cx);
            return;
        }
        match self.store.projects().first().map(|project| project.id.clone()) {
            Some(project) => self.select(Selection::NewChat(project), cx),
            None => self.select(Selection::Welcome, cx),
        }
    }

    fn drop_session(&mut self, chat: &str) {
        // Its tasks are asked to stop first; dropping the chat would only kill them.
        if let Some(tasks) = self.chat_tasks.remove(chat) {
            std::thread::spawn(move || tasks.terminate_all());
        }
        if let Some(agents) = self.chat_agents.remove(chat) {
            std::thread::spawn(move || agents.terminate_all());
        }
        self.titling.remove(chat);
        self.title_swaps.remove(chat);
        if self.renaming.as_ref().is_some_and(|renaming| renaming.chat == chat) {
            self.renaming = None;
        }
        self.sessions.remove(chat);
        self.session_subscriptions.remove(chat);
        self.unsaved_layouts.remove(chat);
        self.tabs.retain(|tab| tab != chat);
    }

    /// Sends the new-chat composer's text as the first message of a new chat. A `/command` that
    /// acts on a chat has none to act on yet; one the model gets starts the chat.
    pub fn start_chat(&mut self, cx: &mut Context<Self>) {
        let project = match self.selection.clone() {
            Selection::NewChat(project) => Some(project),
            Selection::NewChatArea => None,
            _ => return,
        };
        let raw_text = self.draft.read(cx).text().trim().to_string();
        let pictures = self.draft_attachments.clone();
        let files = self.draft_files.clone();
        if raw_text.is_empty() && pictures.is_empty() && files.is_empty() {
            return;
        }
        if let Some(problem) = crate::attachments::files_problem(&files) {
            self.notice = Some(problem.into());
            cx.notify();
            return;
        }
        if !files.is_empty() && raw_text.starts_with('/') {
            self.notice = Some(FILES_WITH_COMMAND.into());
            cx.notify();
            return;
        }
        let text = crate::attachments::compose(&raw_text, &files);
        let cwd = project.as_deref().and_then(|id| self.store.project(id)).map(|project| project.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        let outgoing = match &project {
            Some(_) => self.outgoing(None, &text, &cwd),
            None => Ok(self.with_refs(None, chat_area_outgoing(&text))),
        };
        let (job, opening) = match outgoing {
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
            Ok(Outgoing::Message(message)) => (Job::Prompt(message), raw_text.clone()),
            Ok(Outgoing::Prompt { text, command }) => {
                let opening = self.command_opening(&command);
                (Job::Command { text, command }, opening)
            }
            Ok(Outgoing::Builtin { name: "new", .. }) => {
                self.draft.update(cx, |draft, cx| draft.set_text("", cx));
                return;
            }
            Ok(Outgoing::Builtin { name, .. }) => {
                self.notice = Some(format!("/{name} works in a chat. Send a message first to start one.").into());
                cx.notify();
                return;
            }
        };
        let area = if project.is_some() { ChatKind::Code } else { ChatKind::Chat };
        let Some(resolved) = self.store.resolve_default(area) else {
            self.notice = Some("Add a connection in Settings › Providers to start chatting.".into());
            cx.notify();
            return;
        };
        if let Some(refused) = self.pictures_refused(Some(&resolved), pictures.len()) {
            self.notice = Some(refused.into());
            cx.notify();
            return;
        }
        let choice = resolved.choice();
        let team = if project.is_some() { self.new_chat_team.clone() } else { None };
        if let Some(missing) = self.crew_missing(team.as_ref()) {
            self.notice = Some(missing.into());
            cx.notify();
            return;
        }
        let job = with_pictures(job, &pictures);
        self.sent_at = Some(std::time::Instant::now());
        let opening = if opening.is_empty() {
            files
                .iter()
                .map(|file| file.name.to_string())
                .chain(pictures.iter().map(|picture| picture.name.to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            opening
        };
        let title = match &job {
            Job::Command { command, .. } | Job::Attached { command: Some(command), .. } => title_from(&command.line()),
            _ => title_from(if raw_text.is_empty() { &opening } else { &raw_text }),
        };
        let created = match &project {
            Some(project) => self.store.create_chat(project, &title, choice, self.new_chat_mode, None),
            None => self.store.create_area_chat(&title, choice),
        };
        let chat = match created {
            Ok(chat) => chat,
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
        };
        if team.is_some()
            && let Err(message) = self.store.set_team(&chat.id, team)
        {
            self.notice = Some(message.into());
        }
        // Preserve any Build/Run output panes opened on NewChat into the new chat's layout so
        // live command output stays visible seamlessly after the first message sends.
        let draft_panes = self.new_chat_panes.remove(project.as_deref().unwrap_or(CHAT_AREA_PANES));
        if let Some(draft_panes) = &draft_panes
            && !draft_panes.layout.panes().is_empty()
            && let Some(value) = serde_json::to_value(&draft_panes.layout).ok()
        {
            let _ = self.store.set_layout(&chat.id, Some(value));
        }
        // Pages opened before the chat existed are the chat's now, still live.
        if let Some(tabs) = draft_panes.as_ref().and_then(|panes| panes.browser()).filter(|tabs| !tabs.saved().is_empty()) {
            let value = serde_json::to_value(tabs.saved()).ok();
            if let Err(message) = self.store.set_browser_tabs(&chat.id, value) {
                self.notice = Some(message.into());
            }
            self.handed_tabs.insert(chat.id.clone(), tabs.clone());
        }
        // The next new chat starts in Build again, without a Team: a chat's mode is its own.
        if project.is_some() {
            self.new_chat_mode = AgentMode::Build;
            self.new_chat_team = None;
        }
        self.draft.update(cx, |draft, cx| draft.set_text("", cx));
        self.draft_attachments.clear();
        self.draft_files.clear();
        // The draft tab becomes the chat's tab.
        if let Some(key) = self.active_draft.take() {
            self.drafts.retain(|draft| draft.key != key);
        }
        self.mention_refs.remove(&None);
        self.select(Selection::Chat(chat.id.clone()), cx);
        self.run(&chat.id, job, cx);
        self.start_title(&chat.id, &opening, cx);
    }

    /// What a title is asked for when a chat opens with a command: what was typed and what the
    /// command does, never the whole expanded skill.
    fn command_opening(&self, command: &CommandUse) -> String {
        let description = find(&self.commands, &command.name).map(|found| found.description.clone()).unwrap_or_default();
        format!("{}\n{}", command.line(), description).trim().to_string()
    }

    /// What sending `text` from `owner`'s composer does, against the commands on disk now. An
    /// unknown `/name` is refused once; sent again unchanged, it goes as written.
    fn outgoing(&mut self, owner: Option<&str>, text: &str, cwd: &Path) -> Result<Outgoing, String> {
        let owner = owner.map(str::to_string);
        if self.literal.take().is_some_and(|(from, literal)| from == owner && literal == text) {
            return Ok(Outgoing::Message(text.to_string()));
        }
        let Some((name, _)) = parse_line(text) else { return Ok(Outgoing::Message(text.to_string())) };
        self.commands = self.store.command_catalog();
        if find(&self.commands, name).is_none() {
            self.literal = Some((owner, text.to_string()));
            return Err(format!("There is no /{name} command. Press ↩ again to send it as a message."));
        }
        resolve_message(&self.commands, text, cwd)
    }

    /// Asks the title model for a name for a chat that has just started, alongside its first run.
    /// The opening-line title stays until the answer comes, and after a failure.
    fn start_title(&mut self, chat: &str, opening: &str, cx: &mut Context<Self>) {
        let (provider, connection, model) = match self.store.title_setup() {
            None => return,
            Some(Ok(setup)) => setup,
            Some(Err(_)) => {
                self.note(chat, title::UNUSABLE, cx);
                return;
            }
        };
        let request = title::request_with(&model, opening, self.store.prompts().title.as_deref());
        // The ledger records the request as a run's calls are: against the chat's project, or as
        // the Chat area's.
        let recorder = self.store.chat(chat).and_then(|listed| self.store.chat_recorder(&connection, listed));
        self.title_attempts += 1;
        let attempt = self.title_attempts;
        let id = chat.to_string();
        let work = Tokio::spawn(cx, async move {
            let recorder = recorder.as_deref().map(|recorder| recorder as &dyn wack_core::Recorder);
            title::generate(&provider, &request, recorder).await
        });
        let task = cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The title request stopped.".into()));
            let _ = app.update(cx, |app, cx| app.title_arrived(&id, attempt, result, cx));
        });
        self.titling.insert(chat.to_string(), (attempt, task));
    }

    fn title_arrived(
        &mut self,
        chat: &str,
        attempt: u64,
        result: Result<Option<String>, String>,
        cx: &mut Context<Self>,
    ) {
        // A rename, stop or delete since: the answer is no longer wanted.
        if self.titling.get(chat).map(|(current, _)| *current) != Some(attempt) {
            return;
        }
        self.titling.remove(chat);
        let Ok(Some(name)) = result else {
            self.note(chat, title::FAILED, cx);
            return;
        };
        let Some(old) = self.store.chat(chat).map(|listed| listed.title.clone()) else {
            return;
        };
        if old == name {
            return;
        }
        if self.store.rename_chat(chat, &name).is_ok() {
            self.title_swaps.insert(chat.to_string(), TitleSwap::new(old.into()));
            cx.notify();
        }
    }

    /// Asks the step-summary model to name groups of steps that just settled in `chat`, when it
    /// is switched on. The app's own sentence stays until each answer comes, and after a failure.
    fn summarise(&mut self, chat: &str, jobs: Vec<crate::work::SummaryJob>, cx: &mut Context<Self>) {
        let (provider, connection, model) = match self.store.summary_setup() {
            None => return,
            Some(Ok(setup)) => setup,
            Some(Err(_)) => {
                if self.summary_warned.insert(chat.to_string()) {
                    self.note(chat, step_summary::UNUSABLE, cx);
                }
                return;
            }
        };
        let system = self.store.prompts().step_summary.clone();
        let requests: Vec<(String, wack_core::llm::Request)> = jobs
            .into_iter()
            .map(|job| (job.id, step_summary::request(&model, &job.intent, &job.steps, system.as_deref())))
            .collect();
        // Recorded as a run's calls are: against the chat's project, or as the Chat area's.
        let recorder = self.store.chat(chat).and_then(|listed| self.store.chat_recorder(&connection, listed));
        let work = Tokio::spawn(cx, async move {
            let recorder = recorder.as_deref().map(|recorder| recorder as &dyn wack_core::Recorder);
            let asks = requests.iter().map(|(id, request)| async { (id.clone(), step_summary::generate(&provider, request, recorder).await) });
            futures::future::join_all(asks).await
        });
        let id = chat.to_string();
        cx.spawn(async move |app, cx| {
            let Ok(answers) = work.await else { return };
            let _ = app.update(cx, |app, cx| {
                // A chat deleted since keeps nothing.
                if app.store.chat(&id).is_none() {
                    return;
                }
                let Some(session) = app.sessions.get(&id).cloned() else { return };
                for (group, answer) in answers {
                    if let Ok(Some(summary)) = answer {
                        session.update(cx, |session, cx| session.set_summary(&group, summary, cx));
                    }
                }
            });
        })
        .detach();
    }

    /// A word to the user in a chat's transcript.
    /// The chat whose composer is on screen, or `None` for a new chat's (Code or Chat area).
    pub fn composing_in(&self) -> Option<Option<String>> {
        match &self.selection {
            Selection::Chat(id) => Some(Some(id.clone())),
            Selection::NewChat(_) | Selection::NewChatArea => Some(None),
            _ => None,
        }
    }

    /// A new chat's placeholder in `project`: its name in Build, the planning modes' own words.
    fn new_chat_words(&self, project: &str) -> String {
        match (self.new_chat_mode, self.store.project(project)) {
            (AgentMode::Build, Some(project)) => new_chat_placeholder(&project.name),
            (mode, _) => placeholder(mode, true).to_string(),
        }
    }

    /// The pictures attached to `chat`'s message, or the new chat's.
    pub fn attachments(&self, chat: Option<&str>, cx: &App) -> Vec<crate::attachments::Attachment> {
        match chat {
            Some(chat) => self.sessions.get(chat).map(|session| session.read(cx).attachments.clone()).unwrap_or_default(),
            None => self.draft_attachments.clone(),
        }
    }

    fn set_attachments(&mut self, chat: Option<&str>, attachments: Vec<crate::attachments::Attachment>, cx: &mut Context<Self>) {
        match chat {
            Some(chat) => {
                if let Some(session) = self.sessions.get(chat) {
                    session.update(cx, |session, cx| {
                        session.attachments = attachments;
                        cx.notify();
                    });
                }
            }
            None => self.draft_attachments = attachments,
        }
        cx.notify();
    }

    /// A refusal while attaching: in the chat's transcript, or the new chat's notice.
    pub(crate) fn attach_problem(&mut self, chat: Option<&str>, problem: String, cx: &mut Context<Self>) {
        match chat {
            Some(chat) => self.note(chat, &problem, cx),
            None => {
                self.notice = Some(problem.into());
                cx.notify();
            }
        }
    }

    /// Attaches the files at `paths` (pictures or UTF-8 text files) to `chat`'s message
    /// (the new chat's for `None`), reading them off the main thread.
    pub fn attach_paths(&mut self, chat: Option<String>, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let sources = paths.into_iter().map(crate::text_field::Pasted::File).collect();
        self.attach(chat, sources, cx);
    }

    /// What was pasted into `chat`'s composer (the new chat's for `None`).
    pub fn paste_pictures(&mut self, chat: Option<String>, cx: &mut Context<Self>) {
        let field = match chat.as_deref() {
            Some(chat) => self.sessions.get(chat).map(|session| session.read(cx).composer.clone()),
            None => Some(self.draft.clone()),
        };
        let Some(field) = field else { return };
        let pasted = field.update(cx, |field, _| field.take_pasted());
        let (texts, rest): (Vec<_>, Vec<_>) = pasted.into_iter().partition(|pasted| matches!(pasted, crate::text_field::Pasted::Text(_)));
        if !texts.is_empty() {
            let mut list = self.files(chat.as_deref(), cx);
            let mut problem = None;
            for text in texts {
                if let crate::text_field::Pasted::Text(text) = text {
                    match crate::attachments::FileAttachment::from_paste(&text, &list) {
                        Ok(file) => list.push(file),
                        Err(err) if problem.is_none() => problem = Some(err),
                        Err(_) => {}
                    }
                }
            }
            self.set_files(chat.as_deref(), list, cx);
            if let Some(problem) = problem {
                self.attach_problem(chat.as_deref(), problem, cx);
            }
        }
        self.attach(chat, rest, cx);
    }

    /// Tidies a terminal selection and attaches it to `chat`'s draft (the new chat's for `None`),
    /// returning the composer field to focus on success or a user-facing refusal sentence.
    pub fn attach_terminal_selection(&mut self, chat: Option<String>, raw: String, cx: &mut Context<Self>) -> Result<Entity<TextField>, String> {
        if let Some(id) = chat.as_deref()
            && self.store.chat(id).is_some_and(Chat::is_archived)
        {
            return Err("Unarchive this chat to add to its draft.".into());
        }
        let mut list = self.files(chat.as_deref(), cx);
        let file = crate::attachments::FileAttachment::from_terminal(&raw, &list)?;
        list.push(file);
        self.set_files(chat.as_deref(), list, cx);
        match chat.as_deref() {
            Some(id) => self.sessions.get(id).map(|session| session.read(cx).composer.clone()).ok_or_else(|| "That chat is not open.".into()),
            None => Ok(self.draft.clone()),
        }
    }

    /// The text files attached to `chat`'s message, or the new chat's.
    pub fn files(&self, chat: Option<&str>, cx: &App) -> Vec<crate::attachments::FileAttachment> {
        match chat {
            Some(chat) => self.sessions.get(chat).map(|session| session.read(cx).files.clone()).unwrap_or_default(),
            None => self.draft_files.clone(),
        }
    }

    fn set_files(&mut self, chat: Option<&str>, files: Vec<crate::attachments::FileAttachment>, cx: &mut Context<Self>) {
        match chat {
            Some(chat) => {
                if let Some(session) = self.sessions.get(chat) {
                    session.update(cx, |session, cx| {
                        session.files = files;
                        cx.notify();
                    });
                }
            }
            None => self.draft_files = files,
        }
        cx.notify();
    }

    /// A pasted file's text back in the field, at the caret, in place of the file.
    pub fn insert_file(&mut self, chat: Option<String>, index: usize, cx: &mut Context<Self>) {
        let mut list = self.files(chat.as_deref(), cx);
        if index >= list.len() {
            return;
        }
        let file = list.remove(index);
        self.set_files(chat.as_deref(), list, cx);
        let field = match chat.as_deref() {
            Some(chat) => self.sessions.get(chat).map(|session| session.read(cx).composer.clone()),
            None => Some(self.draft.clone()),
        };
        if let Some(field) = field {
            field.update(cx, |field, cx| field.insert_text(&file.text, cx));
        }
    }

    pub fn remove_file(&mut self, chat: Option<String>, index: usize, cx: &mut Context<Self>) {
        let mut list = self.files(chat.as_deref(), cx);
        if index < list.len() {
            list.remove(index);
            self.set_files(chat.as_deref(), list, cx);
        }
    }

    fn attach(&mut self, chat: Option<String>, sources: Vec<crate::text_field::Pasted>, cx: &mut Context<Self>) {
        if sources.is_empty() {
            return;
        }
        let allow_mentions = !self.is_chat_area_owner(chat.as_deref());
        let read = cx.background_spawn(async move {
            sources
                .into_iter()
                .map(|source| match source {
                    crate::text_field::Pasted::File(path) => crate::attachments::Loaded::from_path(&path, allow_mentions),
                    crate::text_field::Pasted::Picture(bytes) => {
                        crate::attachments::Attachment::from_bytes(bytes, "Pasted picture").map(crate::attachments::Loaded::Picture)
                    }
                    crate::text_field::Pasted::Text(_) => Err("Pasted text is attached as a file, not a picture.".into()),
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |app, cx| {
            let read = read.await;
            let _ = app.update(cx, |app, cx| {
                let mut pictures = app.attachments(chat.as_deref(), cx);
                let mut files = app.files(chat.as_deref(), cx);
                let problem = crate::attachments::add_loaded(&mut pictures, &mut files, read);
                app.set_attachments(chat.as_deref(), pictures, cx);
                app.set_files(chat.as_deref(), files, cx);
                if let Some(problem) = problem {
                    app.attach_problem(chat.as_deref(), problem, cx);
                }
            });
        })
        .detach();
    }

    pub fn remove_attachment(&mut self, chat: Option<String>, index: usize, cx: &mut Context<Self>) {
        let mut list = self.attachments(chat.as_deref(), cx);
        if index < list.len() {
            list.remove(index);
            self.set_attachments(chat.as_deref(), list, cx);
        }
    }

    /// Why `resolved`'s model can't take the attached pictures, when it can't.
    pub fn pictures_refused_for(&self, resolved: Option<&wack_core::Resolved>, pictures: usize) -> Option<String> {
        self.pictures_refused(resolved, pictures)
    }

    /// Why a Team's crew holds Send: its model is gone, and a crew is never swapped for another.
    pub fn crew_missing(&self, crew: Option<&ModelChoice>) -> Option<String> {
        let crew = crew?;
        self.store.resolve_crew(crew).is_none().then(|| self.store.team_problem(Some(crew)).unwrap_or_default())
    }

    fn pictures_refused(&self, resolved: Option<&wack_core::Resolved>, pictures: usize) -> Option<String> {
        let resolved = resolved?;
        (pictures > 0 && !resolved.model.accepts_images).then(|| {
            format!("{} doesn't accept pictures. Turn on Vision for it in Settings › Providers, or remove the pictures.", resolved.model.id)
        })
    }

    fn note(&mut self, chat: &str, message: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.get(chat) {
            session.update(cx, |session, cx| session.note(message.to_string(), cx));
        }
    }

    /// Names a chat as the user typed it. It wins over an automatic title still on its way.
    pub fn rename_chat(&mut self, chat: &str, name: &str, cx: &mut Context<Self>) {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || self.store.chat(chat).is_none_or(|listed| listed.title == name) {
            return;
        }
        self.titling.remove(chat);
        self.title_swaps.remove(chat);
        if self.store.rename_chat(chat, &name).is_ok() {
            cx.notify();
        }
    }

    /// Opens a field over a chat's title, where it is shown, holding the title selected. Return
    /// or clicking away names the chat; Esc leaves it as it was.
    pub fn begin_rename(&mut self, chat: &str, place: RenamePlace, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.store.chat(chat).map(|listed| listed.title.clone()) else {
            return;
        };
        let field = cx.new(|cx| {
            let mut field = TextField::new(if place == RenamePlace::Header { Type::Heading } else { Type::Ui }, "Chat name", cx);
            field.set_text(&title, cx);
            field.select_all_text(cx);
            field
        });
        let id = chat.to_string();
        let entered = cx.subscribe_in(&field, window, move |app, field, event, window, cx| {
            if *event == TextFieldEvent::Enter {
                let name = field.read(cx).text().to_string();
                app.finish_rename(Some(&id), Some(name), Some(window), cx);
            }
        });
        let focus: FocusHandle = field.read(cx).focus_handle(cx);
        let id = chat.to_string();
        let blurred = cx.on_blur(&focus, window, move |app, _, cx| {
            let name = app.renaming.as_ref().filter(|renaming| renaming.chat == id).map(|renaming| renaming.field.read(cx).text().to_string());
            app.finish_rename(Some(&id), name, None, cx);
        });
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        let back = window.focused(cx);
        self.renaming = Some(Renaming { chat: chat.to_string(), place, field, back, _subscriptions: vec![entered, blurred] });
        cx.notify();
    }

    /// Closes the rename field: with a name, the chat takes it; without, it is left as it was.
    /// `chat` guards against a late blur from a field already replaced. Given the window, focus
    /// goes back to where it was before the field opened.
    pub fn finish_rename(&mut self, chat: Option<&str>, name: Option<String>, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(renaming) = self.renaming.take_if(|renaming| chat.is_none_or(|chat| chat == renaming.chat)) else {
            return;
        };
        if let Some(name) = name {
            self.rename_chat(&renaming.chat, &name, cx);
        }
        if let (Some(window), Some(back)) = (window, renaming.back) {
            window.defer(cx, move |window, cx| window.focus(&back, cx));
        }
        cx.notify();
    }

    /// Settings › Sub-agents › Automatic titles' switch. It cannot go on without a usable model.
    pub fn toggle_auto_titles(&mut self, cx: &mut Context<Self>) {
        let mut config = self.store.auto_title().clone();
        config.enabled = !config.enabled;
        self.set_auto_title(config, cx);
    }

    /// Picks the model that writes titles, and closes the list. Titles that are on stay on.
    pub fn choose_title_model(&mut self, connection: &str, model: &str, cx: &mut Context<Self>) {
        let config = AutoTitleConfig {
            enabled: self.store.auto_title().enabled,
            connection: Some(connection.to_string()),
            model: Some(model.to_string()),
        };
        self.title_models_open = false;
        self.set_auto_title(config, cx);
    }

    pub fn toggle_title_models(&mut self, cx: &mut Context<Self>) {
        self.title_models_open = !self.title_models_open;
        cx.notify();
    }

    fn set_auto_title(&mut self, config: AutoTitleConfig, cx: &mut Context<Self>) {
        self.title_error = self.store.set_auto_title(config).err().map(Into::into);
        cx.notify();
    }

    /// Settings › Sub-agents › Step summaries' switch. It cannot go on without a usable model.
    pub fn toggle_step_summaries(&mut self, cx: &mut Context<Self>) {
        let mut config = self.store.step_summary().clone();
        config.enabled = !config.enabled;
        self.set_step_summary(config, cx);
    }

    /// Picks the model that writes step summaries, and closes the list. Summaries that are on stay
    /// on.
    pub fn choose_summary_model(&mut self, connection: &str, model: &str, cx: &mut Context<Self>) {
        let config = AutoTitleConfig {
            enabled: self.store.step_summary().enabled,
            connection: Some(connection.to_string()),
            model: Some(model.to_string()),
        };
        self.summary_models_open = false;
        self.set_step_summary(config, cx);
    }

    pub fn toggle_summary_models(&mut self, cx: &mut Context<Self>) {
        self.summary_models_open = !self.summary_models_open;
        cx.notify();
    }

    fn set_step_summary(&mut self, config: AutoTitleConfig, cx: &mut Context<Self>) {
        self.summary_error = self.store.set_step_summary(config).err().map(Into::into);
        // A model that works again may warn again.
        self.summary_warned.clear();
        cx.notify();
    }

    /// The mode of the composer on screen, and whether it is locked by a run.
    pub fn mode_on_screen(&self, cx: &App) -> (AgentMode, bool) {
        if self.area() == ChatKind::Chat {
            return (AgentMode::Build, true);
        }
        match &self.selection {
            Selection::Chat(id) => {
                let mode = self.store.chat(id).map_or(AgentMode::Build, |chat| chat.mode);
                let locked = self.sessions.get(id).is_some_and(|session| session.read(cx).running());
                (mode, locked)
            }
            _ => (self.new_chat_mode, false),
        }
    }

    /// The Team crew of the composer on screen (`None`: Team is off), and whether a run locks it.
    pub fn team_on_screen(&self, cx: &App) -> (Option<ModelChoice>, bool) {
        if self.area() == ChatKind::Chat {
            return (None, true);
        }
        match &self.selection {
            Selection::Chat(id) => {
                let team = self.store.chat(id).and_then(|chat| chat.team.clone());
                let locked = self.sessions.get(id).is_some_and(|session| session.read(cx).running());
                (team, locked)
            }
            Selection::NewChat(_) => (self.new_chat_team.clone(), false),
            _ => (None, true),
        }
    }

    /// The model leading the composer on screen: its chat's, or a new chat's default.
    fn lead_on_screen(&self) -> ModelChoice {
        match &self.selection {
            Selection::Chat(id) => self.store.chat(id).map(|chat| chat.choice.clone()).unwrap_or_default(),
            _ => self.store.resolve_default(ChatKind::Code).map(|resolved| resolved.choice()).unwrap_or_default(),
        }
    }

    /// Switches the Team of the composer on screen on, with the default crew, or off. A chat
    /// that is working keeps its Team until the run ends.
    pub fn set_team(&mut self, on: bool, cx: &mut Context<Self>) {
        let (team, locked) = self.team_on_screen(cx);
        if locked || team.is_some() == on {
            return;
        }
        let crew = if on {
            match self.store.crew_to_start(&self.lead_on_screen()) {
                Some(crew) => Some(crew),
                None => {
                    self.notice = Some("Add a connection in Settings › Providers to give the Team a crew.".into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        self.put_team(crew, cx);
        // Turning it on opens the crew's menu, so the crew is chosen as well as switched on.
        self.menu = on.then_some(Menu::Crew);
        cx.notify();
    }

    /// The crew of the composer on screen's Team. A crew chosen becomes the default for new
    /// Teams too. `effort` of `None` keeps the crew's effort as it is.
    pub fn choose_crew(&mut self, connection: Option<String>, model: Option<String>, effort: Option<Option<Reasoning>>, cx: &mut Context<Self>) {
        let (team, locked) = self.team_on_screen(cx);
        let Some(current) = team.or_else(|| self.store.crew_to_start(&self.lead_on_screen())) else { return };
        if locked {
            return;
        }
        let model_changed = model.is_some() && model != current.model;
        let crew = ModelChoice {
            connection: connection.or(current.connection),
            model: model.or(current.model),
            // A new model starts from its own default effort, unless one is chosen with it.
            effort: effort.unwrap_or(if model_changed { None } else { current.effort }),
        };
        if let Err(message) = self.store.set_team_default(Some(crew.clone())) {
            self.notice = Some(message.into());
        }
        self.put_team(Some(crew), cx);
        if model_changed {
            self.menu = None;
        }
        cx.notify();
    }

    /// Records the Team of the composer on screen.
    fn put_team(&mut self, crew: Option<ModelChoice>, cx: &mut Context<Self>) {
        match self.selection.clone() {
            Selection::NewChat(_) => self.new_chat_team = crew,
            Selection::Chat(id) => {
                if let Err(message) = self.store.set_team(&id, crew) {
                    self.notice = Some(message.into());
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Switches the composer on screen to `mode`. A chat that is working keeps its mode.
    pub fn set_agent_mode(&mut self, mode: AgentMode, cx: &mut Context<Self>) {
        // The Chat area has no modes.
        if self.area() == ChatKind::Chat {
            return;
        }
        match self.selection.clone() {
            Selection::NewChat(project) => {
                self.new_chat_mode = mode;
                let words = self.new_chat_words(&project);
                self.draft.update(cx, |draft, cx| draft.set_placeholder(words, cx));
            }
            Selection::Chat(id) => {
                if self.mode_on_screen(cx).1 {
                    return;
                }
                self.set_chat_mode(&id, mode, cx);
            }
            _ => return,
        }
        cx.notify();
    }

    /// Records a chat's mode, whoever changed it, and shows it in its composer.
    pub fn set_chat_mode(&mut self, id: &str, mode: AgentMode, cx: &mut Context<Self>) {
        if self.store.chat(id).is_some_and(|chat| chat.mode != mode)
            && let Err(message) = self.store.set_mode(id, mode)
        {
            self.notice = Some(message.into());
        }
        if let Some(session) = self.sessions.get(id)
            && !self.store.chat(id).is_some_and(Chat::is_chat_area)
        {
            let composer = session.read(cx).composer.clone();
            composer.update(cx, |composer, cx| composer.set_placeholder(placeholder(mode, false), cx));
        }
        cx.notify();
    }

    /// Approve & implement: the chat on screen goes back to Build and is told to implement the
    /// plan awaiting review.
    pub fn approve_plan(&mut self, cx: &mut Context<Self>) {
        let Selection::Chat(id) = self.selection.clone() else {
            return;
        };
        let Some(session) = self.sessions.get(&id).cloned() else {
            return;
        };
        if session.read(cx).running() || session.read(cx).awaiting_plan().is_none() {
            return;
        }
        if let Some(problem) = self.store.chat(&id).and_then(|listed| self.store.chat_model_problem(listed)) {
            session.update(cx, |session, cx| session.plan_notice(problem.into(), cx));
            return;
        }
        self.set_chat_mode(&id, AgentMode::Build, cx);
        self.run(&id, Job::Prompt(crate::plan_card::IMPLEMENT.into()), cx);
        session.update(cx, |session, cx| session.focus_composer(cx));
    }

    /// Writes the plan awaiting review to PLAN.md in the chat's project.
    pub fn save_plan(&mut self, cx: &mut Context<Self>) {
        let Selection::Chat(id) = self.selection.clone() else {
            return;
        };
        let (Some(session), Some(chat)) = (self.sessions.get(&id).cloned(), self.store.chat(&id)) else {
            return;
        };
        let Some(project) = chat.project.as_deref().and_then(|id| self.store.project(id)).map(|project| project.path.clone()) else {
            return;
        };
        let Some(plan) = session.read(cx).awaiting_plan() else {
            return;
        };
        let notice = match wack_core::save_plan(&project, &plan) {
            Ok(path) => format!("Plan saved to {}", shown_path(&path)),
            Err(message) => message,
        };
        session.update(cx, |session, cx| session.plan_notice(notice, cx));
    }

    /// Discards the plan awaiting review; the chat returns to Build.
    pub fn discard_plan(&mut self, cx: &mut Context<Self>) {
        let Selection::Chat(id) = self.selection.clone() else {
            return;
        };
        let Some(session) = self.sessions.get(&id).cloned() else {
            return;
        };
        if let Err(message) = session.update(cx, |session, cx| session.discard_plan(cx)) {
            session.update(cx, |session, cx| session.plan_notice(message, cx));
        }
    }

    /// Settings › Tools' switch: off at once, on only after the user confirms.
    pub fn toggle_unrestricted_planning(&mut self, cx: &mut Context<Self>) {
        if self.store.planning_unrestricted() {
            self.set_unrestricted_planning(false, cx);
        } else {
            self.confirming_unrestricted = !self.confirming_unrestricted;
            cx.notify();
        }
    }

    pub fn set_unrestricted_planning(&mut self, unrestricted: bool, cx: &mut Context<Self>) {
        self.confirming_unrestricted = false;
        if let Err(message) = self.store.set_planning_unrestricted(unrestricted) {
            self.notice = Some(message.into());
        }
        cx.notify();
    }

    /// Shows a Settings section. Memory's overview is read afresh as it opens: a run may have
    /// saved a note since.
    pub fn open_settings_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.settings_section = section;
        if section == Section::Providers {
            self.ensure_catalog(cx);
        }
        if section == Section::Memory {
            self.refresh_memory();
        }
        if section == Section::Skills {
            self.skills.notice = None;
            self.refresh_skills();
        }
        if section == Section::Commands {
            self.refresh_templates();
        }
        if section == Section::Subagents {
            self.refresh_subagents();
        }
        if section == Section::Git {
            self.git_error = None;
        }
        if section == Section::Computer {
            self.computer_error = None;
        }
        if section == Section::Browser {
            self.browser_error = None;
            if !matches!(self.browser_clearing, crate::browser_page::Clearing::Pending) {
                self.browser_clearing = Default::default();
            }
        }
        cx.notify();
    }

    fn refresh_memory(&mut self) {
        self.memory.overview = self.store.memory_overview();
    }

    fn memory_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.memory.error = result.err().map(Into::into);
        self.refresh_memory();
        cx.notify();
    }

    /// Settings › Memory's master switch.
    pub fn toggle_memory(&mut self, cx: &mut Context<Self>) {
        let on = !self.store.memory_enabled();
        let result = self.store.set_memory_enabled(on);
        self.memory_result(result, cx);
    }

    pub fn switch_project_memory(&mut self, key: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_project_memory_enabled(key, on);
        self.memory_result(result, cx);
    }

    /// The editor for a new note in `dir`, or for its note `name`. Returns the title field.
    pub fn open_memory_editor(&mut self, dir: &Path, name: Option<&str>, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        self.refresh_memory();
        let project = self.memory.overview.projects.iter().find(|project| project.dir == dir)?.clone();
        let note = match name {
            Some(name) => Some(project.notes.iter().find(|note| note.name == name)?.clone()),
            None => None,
        };
        let editor = MemoryEditor::new(&project, note.as_ref(), cx);
        let title = editor.title.clone();
        self.memory.editor = Some(editor);
        self.memory.confirming = None;
        self.memory.error = None;
        self.watch_memory_editor(cx);
        cx.notify();
        Some(title)
    }

    pub fn close_memory_editor(&mut self, cx: &mut Context<Self>) {
        self.memory.editor = None;
        self.memory_subscriptions.clear();
        self.refresh_memory();
        cx.notify();
    }

    pub fn choose_memory_kind(&mut self, kind: wack_core::memory::Kind, cx: &mut Context<Self>) {
        let Some(editor) = self.memory.editor.as_mut() else {
            return;
        };
        editor.kind = kind;
        editor.error = None;
        if editor.name_follows {
            self.follow_title(cx);
        }
        cx.notify();
    }

    /// Writes the note's name from its title, without the name counting as edited by hand.
    fn follow_title(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.memory.editor.as_ref() else {
            return;
        };
        let title = editor.title.read(cx).text().trim().to_string();
        let name = if title.is_empty() { String::new() } else { editor.suggested_name(cx) };
        let field = editor.name.clone();
        if field.read(cx).text() != name {
            self.memory_subscriptions.clear();
            field.update(cx, |field, cx| field.set_text(&name, cx));
            self.watch_memory_editor(cx);
        }
    }

    /// Saves the note the editor describes, creating it or rewriting it (renaming its file if the
    /// name changed), then goes back to the list.
    pub fn save_memory(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.memory.editor.as_mut() else {
            return;
        };
        if let Some(issue) = editor.issue(cx) {
            editor.error = Some(issue);
            cx.notify();
            return;
        }
        let draft = editor.draft(cx);
        match wack_core::memory::save_draft(&editor.dir, editor.previous.as_deref(), &draft) {
            Ok(_) => self.close_memory_editor(cx),
            Err(message) => {
                editor.error = Some(message.into());
                cx.notify();
            }
        }
    }

    fn watch_memory_editor(&mut self, cx: &mut Context<Self>) {
        self.memory_subscriptions.clear();
        let Some(editor) = self.memory.editor.as_ref() else {
            return;
        };
        let [title, description, name, body] = editor.fields();
        let changed = |app: &mut AppState, cx: &mut Context<AppState>| {
            if let Some(editor) = app.memory.editor.as_mut() {
                editor.error = None;
            }
            cx.notify();
        };
        let save_on_enter = move |app: &mut AppState, event: &TextFieldEvent, cx: &mut Context<AppState>| match event {
            TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_memory(cx),
            TextFieldEvent::Changed => changed(app, cx),
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
        };
        self.memory_subscriptions.push(cx.subscribe(&title, move |app, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Changed) && app.memory.editor.as_ref().is_some_and(|editor| editor.name_follows) {
                app.follow_title(cx);
            }
            save_on_enter(app, event, cx);
        }));
        self.memory_subscriptions.push(cx.subscribe(&description, move |app, _, event: &TextFieldEvent, cx| save_on_enter(app, event, cx)));
        self.memory_subscriptions.push(cx.subscribe(&name, move |app, field, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Changed) {
                // Typed by hand: the name stops following the title, and is kept a valid name.
                if let Some(editor) = app.memory.editor.as_mut() {
                    editor.name_follows = false;
                }
                let text = field.read(cx).text().to_string();
                let tidy: String = text.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
                let tidy = if text.ends_with(char::is_whitespace) && !tidy.is_empty() { format!("{tidy}-") } else { tidy };
                if tidy != text {
                    field.update(cx, |field, cx| field.set_text(&tidy, cx));
                }
            }
            save_on_enter(app, event, cx);
        }));
        self.memory_subscriptions.push(cx.subscribe(&body, move |app, field, event: &TextFieldEvent, cx| match event {
            // Return writes a new line in the note; ⌘↩ saves it.
            TextFieldEvent::Enter => field.update(cx, |field, cx| field.insert_line_break(cx)),
            TextFieldEvent::SendNow => app.save_memory(cx),
            TextFieldEvent::Changed => changed(app, cx),
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
        }));
    }

    pub fn ask_delete_memory(&mut self, dir: &Path, name: &str, cx: &mut Context<Self>) {
        let title = self
            .memory
            .overview
            .projects
            .iter()
            .find(|project| project.dir == dir)
            .and_then(|project| project.notes.iter().find(|note| note.name == name))
            .map(|note| note.title.clone())
            .unwrap_or_else(|| name.to_string());
        self.memory.confirming = Some(MemoryConfirm::Note { dir: dir.to_path_buf(), name: name.to_string(), title });
        cx.notify();
    }

    pub fn ask_remove_memory_folder(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.memory.overview.projects.iter().find(|project| project.key == key) else {
            return;
        };
        self.memory.confirming = Some(MemoryConfirm::Folder {
            key: key.to_string(),
            dir: project.dir.clone(),
            name: project.name.clone(),
            notes: project.notes.len(),
        });
        cx.notify();
    }

    pub fn cancel_memory_delete(&mut self, cx: &mut Context<Self>) {
        self.memory.confirming = None;
        cx.notify();
    }

    /// Moves the note or folder the user confirmed to the macOS Trash. Only paths the overview
    /// listed are touched: a note by its checked name inside a listed folder, or a listed folder.
    pub fn confirm_memory_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.memory.confirming.take() else {
            return;
        };
        let listed = |dir: &Path| self.memory.overview.projects.iter().any(|project| project.dir == dir);
        let result = match confirm {
            MemoryConfirm::Note { dir, name, .. } => {
                if listed(&dir) && wack_core::memory::read(&dir, &name).is_some() {
                    crate::memory_page::move_to_trash(&dir.join(format!("{name}.md")))
                } else {
                    Err("That memory is gone.".into())
                }
            }
            MemoryConfirm::Folder { key, dir, .. } => {
                if listed(&dir) {
                    crate::memory_page::move_to_trash(&dir).and_then(|()| self.store.forget_memory_project(&key))
                } else {
                    Err("That memory folder is gone.".into())
                }
            }
        };
        self.memory_result(result, cx);
    }

    /// ⇧Tab: Build → Plan → Ultra Plan → Build.
    pub fn cycle_agent_mode(&mut self, cx: &mut Context<Self>) {
        if self.area() == ChatKind::Code && matches!(self.selection, Selection::Chat(_) | Selection::NewChat(_)) {
            let next = crate::composer::next_mode(self.mode_on_screen(cx).0);
            self.set_agent_mode(next, cx);
        }
    }

    /// ↩ in a chat's composer: send now, or queue behind the run. With `steer` (⌘↩) a message
    /// written during a run interrupts the agent with it instead. A `/command` is resolved
    /// here: WackCode's own act at once, and one for the model runs (or waits in the queue, as
    /// typed, to be resolved when it is sent; a command never steers).
    pub fn submit(&mut self, chat: &str, steer: bool, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let composer = session.read(cx).composer.clone();
        let text = composer.read(cx).text().trim().to_string();
        let pictures = session.read(cx).attachments.clone();
        let files = session.read(cx).files.clone();
        // Comments drafted on a canvas go with this message (`run`); alone, they send under a
        // stand-in line, since a message needs words.
        let comments = session.read(cx).canvases.draft_count() > 0;
        if text.is_empty() && pictures.is_empty() && files.is_empty() && !comments {
            return;
        }
        if let Some(problem) = crate::attachments::files_problem(&files) {
            session.update(cx, |session, cx| session.note(problem, cx));
            return;
        }
        if !files.is_empty() && text.starts_with('/') {
            session.update(cx, |session, cx| session.note(FILES_WITH_COMMAND.into(), cx));
            return;
        }
        // Attached text files go after the words, ahead of any chats referenced with `@`.
        let text = crate::attachments::compose(&text, &files);
        let mut text = text;
        if text.is_empty() && comments {
            text = COMMENTS_ONLY.to_string();
        }
        let cwd = session.read(cx).cwd().to_path_buf();
        let chat_area = self.store.chat(chat).is_some_and(Chat::is_chat_area);
        let outgoing = if chat_area { Ok(self.with_refs(Some(chat), chat_area_outgoing(&text))) } else { self.outgoing(Some(chat), &text, &cwd) };
        let outgoing = match outgoing {
            Ok(outgoing) => outgoing,
            Err(message) => {
                session.update(cx, |session, cx| session.note(message, cx));
                return;
            }
        };
        // Non-model builtins (/new, /name, /copy) still work while a chat's model is gone.
        let model_free_builtin = matches!(&outgoing, Outgoing::Builtin { name, .. } if *name != "compact");
        if !model_free_builtin {
            if let Some(problem) = self.store.chat(chat).and_then(|listed| self.store.chat_model_problem(listed)) {
                session.update(cx, |session, cx| session.note(problem.into(), cx));
                return;
            }
            if let Some(refused) = self
                .store
                .chat(chat)
                .and_then(|listed| self.pictures_refused(self.store.resolve(&listed.choice).as_ref(), pictures.len()))
            {
                session.update(cx, |session, cx| session.note(refused, cx));
                return;
            }
            if let Some(missing) = self.store.chat(chat).and_then(|listed| self.crew_missing(listed.team.as_ref())) {
                session.update(cx, |session, cx| session.note(missing, cx));
                return;
            }
        }
        let (running, compacting) = {
            let session = session.read(cx);
            (session.running(), session.compacting())
        };
        // What is queued or steered carries its referenced chats too.
        if let Outgoing::Message(message) = &outgoing {
            text = message.clone();
        }
        let owner = Some(chat.to_string());
        let clear = |app: &mut Self, cx: &mut Context<Self>| {
            app.sent_at = Some(std::time::Instant::now());
            app.mention_refs.remove(&owner);
            composer.update(cx, |composer, cx| composer.set_text("", cx));
            session.update(cx, |session, cx| {
                session.attachments.clear();
                session.files.clear();
                cx.notify();
            });
        };
        match outgoing {
            Outgoing::Builtin { name, args } => {
                // WackCode's own commands take words only: the pictures wait for a message.
                if self.builtin(chat, name, &args, cx) {
                    composer.update(cx, |composer, cx| composer.set_text("", cx));
                }
            }
            outgoing if running => {
                session.update(cx, |session, cx| {
                    let plain = matches!(outgoing, Outgoing::Message(_));
                    // Steering is words only: a message with pictures or comments waits its turn.
                    if steer && !compacting && plain && pictures.is_empty() && !comments {
                        session.steer(text, cx);
                    } else {
                        if !pictures.is_empty() {
                            session.queued_pictures.push((text.clone(), pictures.clone()));
                        }
                        session.queue.push(text);
                        session.sync_suggestion_queue(cx);
                    }
                });
                clear(self, cx);
            }
            Outgoing::Message(text) => {
                if self.run(chat, with_pictures(Job::Prompt(text), &pictures), cx) {
                    clear(self, cx);
                }
            }
            Outgoing::Prompt { text, command } => {
                if self.run(chat, with_pictures(Job::Command { text, command }, &pictures), cx) {
                    clear(self, cx);
                }
            }
        }
    }

    /// Runs one of WackCode's own commands in `chat`. False, with the reason in the transcript,
    /// when it could not.
    fn builtin(&mut self, chat: &str, name: &str, args: &str, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return false;
        };
        let refuse = |message: String, cx: &mut Context<Self>| {
            session.update(cx, |session, cx| session.note(message, cx));
            false
        };
        match name {
            "new" => {
                let project = self.store.chat(chat).and_then(|chat| chat.project.clone());
                self.new_chat(project, cx);
                true
            }
            "name" => {
                self.titling.remove(chat);
                self.rename_chat(chat, args, cx);
                true
            }
            "copy" => match session.read(cx).last_reply() {
                Some(reply) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(reply));
                    true
                }
                None => refuse("There is no reply to copy yet.".into(), cx),
            },
            "compact" => {
                let (running, empty) = {
                    let session = session.read(cx);
                    (session.running(), session.is_empty())
                };
                if running {
                    return refuse("Wait for the agent to finish before compacting.".into(), cx);
                }
                if let Some(problem) = self.store.chat(chat).and_then(|listed| self.store.chat_model_problem(listed)) {
                    return refuse(problem.into(), cx);
                }
                if empty {
                    return refuse("There is nothing to compact yet.".into(), cx);
                }
                let focus = Some(args.to_string()).filter(|focus| !focus.is_empty());
                self.run(chat, Job::Compact(focus), cx)
            }
            other => refuse(format!("/{other} can't be used here."), cx),
        }
    }

    pub fn stop(&mut self, chat: &str, cx: &mut Context<Self>) {
        // Stopping the first turn stops its title request too, as version 2 did.
        self.titling.remove(chat);
        if let Some(session) = self.sessions.get(chat) {
            session.update(cx, |session, cx| session.stop(cx));
        }
    }

    pub fn compact(&mut self, chat: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let (running, empty) = {
            let session = session.read(cx);
            (session.running(), session.is_empty())
        };
        if running {
            return;
        }
        if let Some(problem) = self.store.chat(chat).and_then(|listed| self.store.chat_model_problem(listed)) {
            session.update(cx, |session, cx| session.note(problem.into(), cx));
            return;
        }
        if !empty {
            self.run(chat, Job::Compact(None), cx);
        }
    }

    /// Takes a queued message back out to edit it: it joins the composer's text.
    pub fn unqueue(&mut self, chat: &str, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let text = session.update(cx, |session, cx| {
            let text = (index < session.queue.len()).then(|| session.queue.remove(index));
            session.sync_suggestion_queue(cx);
            text
        });
        if let Some(text) = text {
            let refs = restore(&session, vec![text], cx);
            self.keep_refs(chat, refs);
        }
    }

    /// A queued message sent to the agent now, as steering, rather than after the run. Only
    /// words steer: a command, or a message with pictures, waits its turn.
    pub fn steer_queued(&mut self, chat: &str, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        session.update(cx, |session, cx| {
            let Some(text) = session.queue.get(index).cloned() else { return };
            if !session.running() || session.compacting() {
                return;
            }
            if text.starts_with('/') || session.queued_pictures.iter().any(|(queued, _)| *queued == text) {
                session.note("Only words can steer: a command or a message with pictures waits for the agent to finish.".into(), cx);
                return;
            }
            session.queue.remove(index);
            session.sync_suggestion_queue(cx);
            session.steer(text, cx);
        });
    }

    /// A queued message dropped unsent, with any pictures it carried.
    pub fn remove_queued(&mut self, chat: &str, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        session.update(cx, |session, cx| {
            if index < session.queue.len() {
                let text = session.queue.remove(index);
                session.take_queued_pictures(&text);
                session.sync_suggestion_queue(cx);
                cx.notify();
            }
        });
    }

    /// What the cards of `chat`'s settled turns read their state from: dismissals and opened
    /// chats from the store; the chat itself fills in started and queued ids.
    fn suggestion_view(&self, chat: &Chat) -> crate::suggestions::View {
        let chats = chat.project.as_deref().map(|project| self.store.chats(project)).unwrap_or_default();
        crate::suggestions::View {
            dismissed: chat.dismissed_suggestions.iter().cloned().collect(),
            queued: Default::default(),
            links: crate::suggestions::links(&chats, &chat.id),
            model_missing: self.store.chat_model_problem(chat).is_some(),
        }
    }

    /// Brings every open chat's suggestion cards in line with the store's connections and models.
    fn sync_suggestion_views(&self, cx: &mut Context<Self>) {
        for (id, session) in &self.sessions {
            if let Some(chat) = self.store.chat(id) {
                let view = self.suggestion_view(chat);
                session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
            }
        }
    }

    /// A suggested-task card's Start: in this chat or in one of its own, in `mode`. The chat on
    /// screen is the one that acted, as with the plan card's actions.
    pub fn start_suggestion(&mut self, id: &str, target: crate::actions::SuggestionTarget, mode: AgentMode, cx: &mut Context<Self>) {
        let Selection::Chat(chat_id) = self.selection.clone() else {
            return;
        };
        let Some(chat) = self.store.chat(&chat_id).cloned() else {
            return;
        };
        let Some(session) = self.sessions.get(&chat.id).cloned() else {
            return;
        };
        let Some(card) = session.read(cx).suggestion(id) else {
            return;
        };
        match target {
            crate::actions::SuggestionTarget::New => self.start_suggestion_chat(&chat, &card, mode, cx),
            crate::actions::SuggestionTarget::Here => {
                if let Some(problem) = self.store.chat_model_problem(&chat) {
                    session.update(cx, |session, cx| session.note(problem.into(), cx));
                    return;
                }
                // "Plan it here" switches the chat to Plan: its mode is its own, and stays that way.
                if chat.mode != mode {
                    self.set_chat_mode(&chat.id, mode, cx);
                }
                let text = crate::suggestions::compose(&card, None);
                if session.read(cx).running() {
                    // Queued behind the run, like a message typed while it works: the card
                    // reads Queued until the queue sends it.
                    self.literal = Some((Some(chat.id.clone()), text.clone()));
                    session.update(cx, |session, cx| {
                        session.queue.push(text);
                        session.sync_suggestion_queue(cx);
                    });
                } else {
                    self.run(&chat.id, Job::Prompt(text), cx);
                }
            }
        }
    }

    /// A suggested task's new chat: the origin's project, model and effort — it is the same
    /// work — named after the suggestion, with the origin recorded so the card can link back to
    /// it. When the origin's own model is gone, falls back to the Code area's default so starting
    /// a new chat still works. It opens in a background tab: the user stays in the chat they were
    /// reading.
    fn start_suggestion_chat(&mut self, origin: &Chat, card: &crate::suggestions::Card, mode: AgentMode, cx: &mut Context<Self>) {
        let origin_of = wack_core::SuggestionOrigin { chat: origin.id.clone(), suggestion: card.id.to_string() };
        let Some(project) = origin.project.clone() else {
            return;
        };
        let Some(choice) = self
            .store
            .resolve(&origin.choice)
            .or_else(|| self.store.resolve_default(ChatKind::Code))
            .map(|resolved| resolved.choice())
        else {
            self.note(&origin.id, "Add a connection in Settings › Providers to start chatting.", cx);
            return;
        };
        let chat = match self
            .store
            .create_chat(&project, &card.title, choice, mode, Some(origin_of))
        {
            Ok(chat) => chat,
            Err(message) => {
                self.note(&origin.id, &message, cx);
                return;
            }
        };
        self.open_projects.insert(project);
        if !self.tabs.contains(&chat.id) {
            self.tabs.push(chat.id.clone());
        }
        let text = crate::suggestions::compose(card, Some(crate::suggestions::Origin { chat: origin.id.clone(), name: origin.title.clone() }));
        self.run(&chat.id, Job::Prompt(text), cx);
        self.start_title(&chat.id, &card.title, cx);
        // The card that sent it now links to the chat it opened.
        if let Some(session) = self.sessions.get(&origin.id) {
            let view = self.suggestion_view(origin);
            session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
        }
    }

    /// Folds a suggested task away, or brings it back.
    pub fn dismiss_suggestion(&mut self, id: &str, dismissed: bool, cx: &mut Context<Self>) {
        let Selection::Chat(chat_id) = self.selection.clone() else {
            return;
        };
        if let Err(message) = self.store.set_suggestion_dismissed(&chat_id, id, dismissed) {
            self.note(&chat_id, &message, cx);
            return;
        }
        let Some(session) = self.sessions.get(&chat_id).cloned() else {
            return;
        };
        let Some(chat) = self.store.chat(&chat_id).cloned() else {
            return;
        };
        let view = self.suggestion_view(&chat);
        session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
    }

    /// Copies the message a Start would send, for pasting anywhere.
    pub fn copy_suggestion(&mut self, id: &str, cx: &mut Context<Self>) {
        let Selection::Chat(chat_id) = self.selection.clone() else {
            return;
        };
        if let Some(card) = self
            .sessions
            .get(&chat_id)
            .and_then(|session| session.read(cx).suggestion(id))
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(crate::suggestions::compose(&card, None)));
            self.note(&chat_id, "Suggested task copied as a prompt.", cx);
        }
    }

    /// Starts a run in a chat. False, with the reason in its transcript, when it could not start.
    fn run(&mut self, chat_id: &str, job: Job, cx: &mut Context<Self>) -> bool {
        let Some(chat) = self.store.chat(chat_id).cloned() else {
            return false;
        };
        // An archived chat is read, not run: not by a message, a queued one, or a task's news
        // waking it. Unarchiving it brings that back.
        if chat.is_archived() {
            return false;
        }
        let session = self.ensure_session(&chat, cx);
        if session.read(cx).rewinding {
            session.update(cx, |session, cx| session.note("Wait a moment: the chat is going back to before a message.".into(), cx));
            return false;
        }
        // A message from the user lets task news wake the chat again.
        if matches!(job, Job::Prompt(_) | Job::Command { .. } | Job::Attached { .. }) {
            session.update(cx, |session, _| {
                session.wakes = 0;
                session.wake_paused = false;
            });
        }
        match self.store.run_setup(&chat) {
            Ok(mut setup) => {
                setup.mcp_hub = Some(self.mcp.clone());
                // browser_preview starts the Run command as the Run button does, and the Run
                // pane hears of it the same way. A Chat chat has no project to run.
                let tx = self.command_event_tx.clone();
                setup.commands = (!chat.is_chat_area()).then(|| wack_core::preview::RunCommands {
                    runner: self.runs.clone(),
                    events: std::sync::Arc::new(move |event| {
                        let _ = tx.unbounded_send(event);
                    }),
                });
                // Comments drafted on the chat's canvases go with the user's message: armed now,
                // told in the note before it (`CanvasFeature`), with a picture of each element
                // for a model that sees pictures.
                let armed = if matches!(job, Job::Prompt(_) | Job::Command { .. } | Job::Attached { .. }) {
                    session.read(cx).canvases.arm_drafts()
                } else {
                    Vec::new()
                };
                let sees = setup.settings.accepts_images;
                if armed.is_empty() || !sees {
                    session.update(cx, |session, cx| session.start(setup, job, cx));
                } else {
                    let canvases = session.read(cx).canvases.clone();
                    let session = session.clone();
                    cx.spawn(async move |_, cx| {
                        let crops = cx.background_spawn(async move { comment_crops(&canvases, &armed) }).await;
                        session.update(cx, |session, cx| session.start(setup, with_images(job, crops), cx));
                    })
                    .detach();
                }
                let _ = self.store.touch_chat(chat_id);
                cx.notify();
                true
            }
            Err(message) => {
                session.update(cx, |session, cx| session.show_failure(message, cx));
                false
            }
        }
    }

    /// A background task or sub-agent in an idle chat has news: the chat wakes so the model can act on it,
    /// unless something is waiting on the user (a plan to review, a queued message), the user
    /// stopped the last run, or it has already woken [`MAX_WAKES`] times since they last wrote.
    /// The news is never lost: it waits for the next run, and the tasks pill shows it.
    fn wake(&mut self, chat: &str, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        if self.store.chat(chat).is_none_or(|listed| self.store.chat_model_problem(listed).is_some()) {
            return;
        }
        // A sub-agent's report doesn't count against the cap: each answers an agent the model
        // started, and how many may start between the user's messages is capped instead.
        let (may, counted) = {
            let session = session.read(cx);
            let agents = session.agents.wakes();
            let tasks = session.tasks.wakes() && session.wakes < MAX_WAKES;
            let may = !session.running()
                && (agents || tasks)
                && session.awaiting_plan().is_none()
                && session.queue.is_empty()
                && session.asking.is_none()
                // The user is changing the conversation: the news waits for what they send.
                && session.editing.is_none()
                && session.rewind_card.is_none()
                && !session.rewinding
                && !session.wake_paused;
            (may, !agents)
        };
        if may {
            if counted {
                session.update(cx, |session, _| session.wakes += 1);
            }
            self.run(chat, Job::Wake, cx);
        }
    }

    /// Moves the command the agent is waiting on in the current chat to the background (⌘B).
    pub fn background_command(&mut self, chat: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.get(chat) {
            session.update(cx, |session, cx| session.background_command(cx));
        }
    }

    /// A run ended: mark the chat, then send what was queued, or hand back what was not sent.
    fn finished(&mut self, chat: &str, result: &Result<Outcome, String>, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let _ = self.store.touch_chat(chat);
        if let Some(project) = self.store.chat(chat).and_then(|chat| chat.project.clone()) {
            // The agent may have switched branches.
            self.look_up_branch(&project, cx);
        }
        let selected = self.selection == Selection::Chat(chat.to_string());
        let (unsent, next) = session.update(cx, |session, cx| {
            if !selected {
                session.unread = true;
            }
            let steering = std::mem::take(&mut session.steering);
            let queue = std::mem::take(&mut session.queue);
            let holding = session.awaiting_plan().is_some();
            let after = after_run(matches!(result, Ok(Outcome::Done)), holding, steering, queue);
            session.queue = after.queue;
            session.sync_suggestion_queue(cx);
            (after.unsent, after.next)
        });
        if !unsent.is_empty() {
            let refs = restore(&session, unsent, cx);
            self.keep_refs(chat, refs);
        }
        // The user stopped it, or it failed: no waking until they write again.
        if !matches!(result, Ok(Outcome::Done | Outcome::CutOff | Outcome::Refused)) {
            session.update(cx, |session, _| session.wake_paused = true);
        }
        match next {
            Some(next) => {
                if !self.send_queued(chat, &next, cx) {
                    let refs = restore(&session, vec![next], cx);
                    self.keep_refs(chat, refs);
                }
            }
            // News that came while it ran, after its last request, wakes it now.
            None => self.wake(chat, cx),
        }
        cx.notify();
    }

    /// Sends a message that waited for a run to end, resolving a `/command` in it now. False when
    /// it could not be sent, the reason in the transcript.
    fn send_queued(&mut self, chat: &str, text: &str, cx: &mut Context<Self>) -> bool {
        let pictures = match self.sessions.get(chat) {
            Some(session) => session.update(cx, |session, _| session.take_queued_pictures(text)),
            None => Vec::new(),
        };
        let sent = self.send_queued_with(chat, text, &pictures, cx);
        if !sent && !pictures.is_empty()
            && let Some(session) = self.sessions.get(chat)
        {
            // Not sent: they go back with the text (`restore`).
            session.update(cx, |session, _| session.queued_pictures.push((text.to_string(), pictures)));
        }
        sent
    }

    fn send_queued_with(&mut self, chat: &str, text: &str, pictures: &[crate::attachments::Attachment], cx: &mut Context<Self>) -> bool {
        // A queued suggested task's title may start with `/`: it was recorded literal when it
        // queued, so it sends as written rather than trying to resolve a command.
        if self.literal.take().is_some_and(|(from, literal)| from.as_deref() == Some(chat) && literal == text) {
            return self.run(chat, with_pictures(Job::Prompt(text.to_string()), pictures), cx);
        }
        let Some(cwd) = self.sessions.get(chat).map(|session| session.read(cx).cwd().to_path_buf()) else {
            return false;
        };
        let refused = |app: &mut Self, message: String, cx: &mut Context<Self>| {
            if let Some(session) = app.sessions.get(chat) {
                session.update(cx, |session, cx| session.note(message, cx));
            }
            false
        };
        match parse_line(text).map(|_| resolve_message(&self.store.command_catalog(), text, &cwd)) {
            None => self.run(chat, with_pictures(Job::Prompt(text.to_string()), pictures), cx),
            Some(Ok(Outgoing::Message(text))) => self.run(chat, with_pictures(Job::Prompt(text), pictures), cx),
            Some(Ok(Outgoing::Prompt { text, command })) => self.run(chat, with_pictures(Job::Command { text, command }, pictures), cx),
            Some(Ok(Outgoing::Builtin { name, args })) => self.builtin(chat, name, &args, cx) || true,
            Some(Err(message)) => refused(self, message, cx),
        }
    }

    /// A Team chat as its Agents pane's overview shows it; `None` while its Team is off.
    pub fn team_view(&self, id: &str, cx: &gpui::App) -> Option<crate::panes::agents::TeamView> {
        let chat = self.store.chat(id)?;
        let crew = chat.team.as_ref()?;
        let session = self.sessions.get(id)?.read(cx);
        let name = |choice: &ModelChoice| match self.store.resolve(choice) {
            Some(resolved) => resolved.model.label().to_string(),
            None => choice.model.as_deref().map(crate::shell::model_name).unwrap_or_else(|| "No model".into()),
        };
        Some(crate::panes::agents::TeamView {
            lead: name(&chat.choice).into(),
            crew: name(crew).into(),
            lead_tokens: session.lead_tokens(),
            todos: session.todos().visible().cloned().collect(),
        })
    }

    /// A Team chat's first worker since launch opens its Agents pane on the overview, once.
    fn team_worker_started(&mut self, id: &str, cx: &mut Context<Self>) {
        let team = self.store.chat(id).is_some_and(|chat| chat.team.is_some());
        if !team || !self.team_pane_opened.insert(id.to_string()) {
            return;
        }
        if let Some(session) = self.sessions.get(id).cloned() {
            session.update(cx, |session, cx| {
                session.change_layout(cx, |layout| {
                    layout.open(crate::panes::PaneKind::Agents);
                });
                session.panes.select_team(cx);
            });
        }
    }

    fn ensure_session(&mut self, chat: &Chat, cx: &mut Context<Self>) -> Entity<ChatSession> {
        if let Some(session) = self.sessions.get(&chat.id) {
            return session.clone();
        }
        let project = chat.project.as_deref().and_then(|id| self.store.project(id));
        // A Chat chat works in its scratchpad.
        let cwd = self.store.chat_folder(chat).unwrap_or_else(|| PathBuf::from("/"));
        let commands = (
            project.and_then(|project| project.command(CommandKind::Build)).map(str::to_string),
            project.and_then(|project| project.command(CommandKind::Run)).map(str::to_string),
        );
        let path = self.store.session_path(chat);
        let layout = Layout::from_saved(chat.layout.as_ref());
        let runner = self.runs.clone();
        let tasks_folder = self.store.tasks_folder(chat);
        let tabs = self.handed_tabs.remove(&chat.id).unwrap_or_else(|| {
            let saved: Vec<wack_browser::SavedTab> =
                chat.browser_tabs.as_ref().and_then(|value| serde_json::from_value(value.clone()).ok()).unwrap_or_default();
            let tabs = wack_browser::ChatTabs::new(Some(self.browser.clone()));
            // Listed now, made into pages only when shown or used.
            tabs.restore(saved);
            tabs
        });
        // A Chat chat's canvases take local pictures only from its scratchpad and its own saved
        // pictures (docs/security.md, "Chat's scratchpad").
        let canvases = if chat.is_chat_area() {
            let folders = vec![self.store.scratch_folder(chat), self.store.pictures_folder(chat)];
            wack_canvas::Canvases::confined(self.store.canvases_folder(chat), Some(self.browser.clone()), folders)
        } else {
            wack_canvas::Canvases::new(self.store.canvases_folder(chat), Some(self.browser.clone()))
        };
        let computer = (!chat.is_chat_area()).then(|| self.computer.chat());
        let session = cx.new(|cx| {
            let mut session = ChatSession::load(&chat.id, cwd, &path, layout, runner, commands, tasks_folder, tabs, canvases, cx);
            if chat.is_chat_area() {
                session.set_chat_area(cx);
            }
            if let Some(computer) = computer {
                session.set_computer(computer, cx);
            }
            session
        });
        let id = chat.id.clone();
        if !chat.is_chat_area() {
            // The Agents pane reads the chat's Team from the app as it draws.
            let app = cx.entity().downgrade();
            let chat_id = id.clone();
            let source: crate::panes::agents::TeamSource = std::rc::Rc::new(move |cx: &gpui::App| app.upgrade().and_then(|app| app.read(cx).team_view(&chat_id, cx)));
            session.update(cx, |session, cx| session.panes.set_team_source(source, cx));
        }
        // Cards read their dismissals and opened links from the store as it stands now.
        let view = self.suggestion_view(chat);
        session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
        self.chat_tasks.insert(id.clone(), session.read(cx).tasks.clone());
        self.chat_agents.insert(id.clone(), session.read(cx).agents.clone());
        let composer = session.read(cx).composer.clone();
        let words = if chat.is_chat_area() { CHAT_FOLLOW_UP_PLACEHOLDER } else { placeholder(chat.mode, false) };
        composer.update(cx, |composer, cx| composer.set_placeholder(words, cx));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, {
                let id = id.clone();
                move |app, _, event: &ChatEvent, cx| match event {
                    ChatEvent::Finished(result) => app.finished(&id, result, cx),
                    ChatEvent::LayoutChanged => app.layout_changed(&id, cx),
                    ChatEvent::ModeChanged(mode) => app.set_chat_mode(&id, *mode, cx),
                    ChatEvent::TasksNews | ChatEvent::AgentsNews => app.wake(&id, cx),
                    ChatEvent::AgentOpenedTab => {
                        // Only that chat's panes: a chat in the background never takes the
                        // window there.
                        if app.store.browser_config().agent_opens_pane
                            && let Some(session) = app.sessions.get(&id).cloned()
                        {
                            session.update(cx, |session, cx| {
                                session.change_layout(cx, |layout| {
                                    layout.open(crate::panes::PaneKind::Browser);
                                })
                            });
                        }
                    }
                    ChatEvent::AgentStarted => app.team_worker_started(&id, cx),
                    ChatEvent::Summarise(jobs) => app.summarise(&id, jobs.clone(), cx),
                    ChatEvent::AgentStartedCanvas => {
                        if let Some(session) = app.sessions.get(&id).cloned() {
                            session.update(cx, |session, cx| {
                                session.change_layout(cx, |layout| {
                                    layout.open(crate::panes::PaneKind::Canvas);
                                })
                            });
                        }
                    }
                    ChatEvent::BrowserTabsChanged(tabs) => {
                        let value = (!tabs.is_empty()).then(|| serde_json::to_value(tabs).ok()).flatten();
                        if let Err(message) = app.store.set_browser_tabs(&id, value) {
                            app.notice = Some(message.into());
                            cx.notify();
                        }
                    }
                }
            }),
            cx.subscribe(&composer, {
                let id = id.clone();
                move |app, _, event: &TextFieldEvent, cx| match event {
                    TextFieldEvent::Enter => app.submit(&id, false, cx),
                    TextFieldEvent::SendNow => app.submit(&id, true, cx),
                    TextFieldEvent::Changed | TextFieldEvent::Moved => app.composer_changed(Some(id.clone()), cx),
                    TextFieldEvent::Menu(key) => app.slash_key(Some(id.clone()), *key, cx),
                    TextFieldEvent::PastedPictures => app.paste_pictures(Some(id.clone()), cx),
                }
            }),
        ];
        self.sessions.insert(id.clone(), session.clone());
        self.session_subscriptions.insert(id, subscriptions);
        session
    }

    fn look_up_branch(&mut self, project: &str, cx: &mut Context<Self>) {
        let Some(path) = self.store.project(project).map(|project| project.path.clone()) else {
            return;
        };
        let project = project.to_string();
        let lookup = cx.background_spawn(async move { wack_core::git::branch(&path) });
        cx.spawn(async move |app, cx| {
            let branch = lookup.await;
            let _ = app.update(cx, |app, cx| {
                if app.branches.get(&project) != Some(&branch) {
                    app.branches.insert(project, branch);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn set_format(&mut self, format: WireFormat, cx: &mut Context<Self>) {
        self.form.set_format(format, cx);
        self.form_changed(cx);
    }

    /// A new connection's editor, with one empty model row.
    pub fn open_connection_form(&mut self, cx: &mut Context<Self>) {
        self.form.open_new(cx);
        self.watch_form(cx);
        cx.notify();
    }

    /// The editor for a saved connection; its key field starts empty, since blank keeps the key.
    pub fn edit_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(connection) = self.store.connections().iter().find(|connection| connection.id == id).cloned() else {
            return;
        };
        self.form.edit(&connection, cx);
        self.watch_form(cx);
        cx.notify();
    }

    /// Back to the overview: the editor forgets what was typed.
    pub fn close_connection_form(&mut self, cx: &mut Context<Self>) {
        self.form.close(cx);
        self.watch_form(cx);
        cx.notify();
    }

    /// The eye at the key field: shows the saved key, hides it again, or unmasks what was typed.
    pub fn toggle_connection_key(&mut self, cx: &mut Context<Self>) {
        let (masked, empty) = {
            let field = self.form.key.read(cx);
            (field.is_masked(), field.text().trim().is_empty())
        };
        if masked && empty {
            // Nothing typed: the key to look at is the saved one.
            let Some(id) = self.form.editing.clone() else {
                return;
            };
            match self.store.reveal_connection_key(&id) {
                Ok(key) => self.form.show_saved_key(&key, cx),
                Err(message) => {
                    self.form.error = Some(message.into());
                    self.form.notice = None;
                }
            }
        } else {
            self.form.toggle_key_reveal(cx);
        }
        cx.notify();
    }

    /// Puts the saved connection's fields back, keeping the editor open.
    pub fn discard_connection_changes(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.form.editing.clone() else {
            return;
        };
        let Some(connection) = self.store.connections().iter().find(|connection| connection.id == id).cloned() else {
            return;
        };
        self.form.edit(&connection, cx);
        self.watch_form(cx);
        cx.notify();
    }

    pub fn add_model_row(&mut self, cx: &mut Context<Self>) {
        self.form.add_model("", None, cx);
        self.watch_form(cx);
        self.form_changed(cx);
    }

    pub fn toggle_model_reasoning(&mut self, index: usize, cx: &mut Context<Self>) {
        self.form.toggle_reasoning(index);
        self.form_changed(cx);
    }

    pub fn toggle_model_vision(&mut self, index: usize, cx: &mut Context<Self>) {
        self.form.toggle_vision(index);
        self.form_changed(cx);
    }

    pub fn toggle_model_effort(&mut self, index: usize, effort: Reasoning, cx: &mut Context<Self>) {
        self.form.toggle_effort(index, effort);
        self.form_changed(cx);
    }

    pub fn remove_model_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.form.remove_model(index);
        self.watch_form(cx);
        self.form_changed(cx);
    }

    /// Saves the connection the editor describes, adding or changing it; the editor stays, now
    /// showing the saved connection, and the key field is emptied.
    pub fn save_connection(&mut self, cx: &mut Context<Self>) {
        let editing = self.form.editing.clone();
        let has_saved_key = editing.as_deref().is_some_and(|id| self.store.has_key(id));
        let (new, key) = match self.form.read(cx, !has_saved_key) {
            Ok(described) => described,
            Err(message) => {
                self.form.error = Some(message.into());
                self.form.notice = None;
                cx.notify();
                return;
            }
        };
        let result = match editing.as_deref() {
            Some(id) => self.store.update_connection(id, new, (!key.is_empty()).then_some(key.as_str())),
            None => self.store.add_connection(new, &key),
        };
        match result {
            Ok(connection) => {
                self.form.editing = Some(connection.id.clone());
                self.form.prune_blank_models(cx);
                // Empties the field; the key now lives only in the store.
                self.form.clear_key(cx);
                self.form.error = None;
                self.form.notice = Some(
                    if key.is_empty() { "Saved." } else { "Saved. The API key is stored on this Mac." }.into(),
                );
                self.form.notice_serial = self.form.notice_serial.wrapping_add(1);
                let serial = self.form.notice_serial;
                // The confirmation tucks itself away, as version 2's save bar did.
                cx.spawn(async move |app, cx| {
                    cx.background_executor().timer(SAVE_NOTICE_TIME).await;
                    let _ = app.update(cx, |app, cx| {
                        if app.form.notice_serial == serial {
                            app.form.notice = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
                self.watch_form(cx);
                self.sync_suggestion_views(cx);
            }
            Err(message) => {
                self.form.error = Some(message.into());
                self.form.notice = None;
            }
        }
        cx.notify();
    }

    /// Watches every field of the connection editor, so saving knows as soon as it can. A model
    /// id field also drives the catalogue's suggestions under it; any other field closes them.
    fn watch_form(&mut self, cx: &mut Context<Self>) {
        self.form_subscriptions.clear();
        let ids: HashSet<gpui::EntityId> = self.form.models.iter().map(|row| row.id.entity_id()).collect();
        for field in self.form.all_fields() {
            let suggests = ids.contains(&field.entity_id());
            self.form_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_connection(cx),
                TextFieldEvent::Changed => {
                    app.form_changed(cx);
                    if suggests {
                        app.suggest_models(field, cx);
                    } else {
                        app.close_model_suggestions(cx);
                    }
                }
                TextFieldEvent::Moved if suggests => app.suggest_models(field, cx),
                TextFieldEvent::Moved => app.close_model_suggestions(cx),
                TextFieldEvent::Menu(key) if suggests => app.suggest_key(field, *key, cx),
                TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }

    /// The model catalogue, read off the main thread the first time Settings › Providers needs it.
    pub fn ensure_catalog(&mut self, cx: &mut Context<Self>) {
        if self.catalog.current.is_some() || self.catalog.loading {
            return;
        }
        self.catalog.loading = true;
        let path = self.store.catalog_path();
        let load = cx.background_spawn(async move { wack_core::catalog::load(&path) });
        cx.spawn(async move |app, cx| {
            let catalog = load.await;
            let _ = app.update(cx, |app, cx| {
                app.catalog.loading = false;
                app.catalog.current.get_or_insert(catalog);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads models.dev's list again: the only time it is contacted. Whatever happens, the copy in
    /// use stays until a new one is saved, and the overview says how it went.
    pub fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        if self.catalog.refreshing {
            return;
        }
        let path = self.store.catalog_path();
        let current = self.catalog.current.clone().unwrap_or_else(|| wack_core::catalog::load(&path));
        self.catalog.current = Some(current.clone());
        self.catalog.refreshing = true;
        self.catalog.outcome = None;
        self.catalog.serial += 1;
        let serial = self.catalog.serial;
        let work = Tokio::spawn(cx, wack_core::catalog::refresh(path, current));
        cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("The refresh stopped.".to_string()));
            let _ = app.update(cx, |app, cx| {
                if app.catalog.serial != serial {
                    return;
                }
                app.catalog.refreshing = false;
                app.catalog.outcome = Some(match result {
                    Ok(refreshed) => {
                        let new = match refreshed.new {
                            0 => "nothing new".to_string(),
                            1 => "1 new".to_string(),
                            count => format!("{count} new"),
                        };
                        app.catalog.current = Some(refreshed.catalog);
                        Ok(format!("Refreshed: {} models, {new}.", crate::model_catalog::thousands(refreshed.models)).into())
                    }
                    Err(message) => Err(message.into()),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Follows what is typed in a model id field with the catalogue's suggestions under it.
    fn suggest_models(&mut self, field: Entity<TextField>, cx: &mut Context<Self>) {
        let id = field.entity_id();
        let text = field.read(cx).text().trim().to_string();
        if self.form.suggest_hold.as_ref().is_some_and(|(held, _)| *held != id) {
            self.form.suggest_hold = None;
        }
        // ⎋ or a pick keeps them shut for that text; a change opens them again.
        let held = self.form.suggest_hold.as_ref().is_some_and(|(_, at)| *at == text);
        if !held {
            self.form.suggest_hold = None;
        }
        let Some(catalog) = self.catalog.current.clone().filter(|_| !held) else {
            self.ensure_catalog(cx);
            self.close_model_suggestions(cx);
            return;
        };
        let address = self.form.address.read(cx).text().to_string();
        let matches = catalog.search(&text, &address, wack_core::catalog::SUGGESTIONS);
        if matches.is_empty() {
            self.close_model_suggestions(cx);
            return;
        }
        let previous = self.form.suggest.take();
        if let Some(other) = previous.as_ref().filter(|previous| previous.field != id).and_then(|previous| self.form_field(previous.field)) {
            other.update(cx, |field, cx| field.set_completing(false, cx));
        }
        let selected = previous
            .filter(|previous| previous.field == id && previous.query == text)
            .map_or(0, |previous| previous.selected.min(matches.len() - 1));
        self.form.suggest = Some(crate::settings::ModelSuggest { field: id, query: text, matches, selected });
        field.update(cx, |field, cx| field.set_completing(true, cx));
        cx.notify();
    }

    /// The model id field with this entity id.
    fn form_field(&self, id: gpui::EntityId) -> Option<Entity<TextField>> {
        self.form.models.iter().map(|row| row.id.clone()).find(|field| field.entity_id() == id)
    }

    /// A key the suggestions borrowed from their field.
    fn suggest_key(&mut self, field: Entity<TextField>, key: MenuKey, cx: &mut Context<Self>) {
        let Some(suggest) = self.form.suggest.as_mut().filter(|suggest| suggest.field == field.entity_id()) else {
            field.update(cx, |field, cx| field.set_completing(false, cx));
            return;
        };
        let count = suggest.matches.len();
        match key {
            MenuKey::Up | MenuKey::Down if count > 0 => {
                suggest.selected = match key {
                    MenuKey::Up => (suggest.selected + count - 1) % count,
                    _ => (suggest.selected + 1) % count,
                };
                cx.notify();
            }
            MenuKey::Pick => {
                let index = suggest.selected;
                self.pick_model_suggestion(index, cx);
            }
            MenuKey::Dismiss => {
                self.form.suggest_hold = Some((field.entity_id(), suggest.query.clone()));
                self.close_model_suggestions(cx);
            }
            _ => {}
        }
    }

    /// Fills the row under the suggestions from the `index`th one: its id, then every fact the
    /// catalogue knows. Everything stays editable.
    pub fn pick_model_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(suggest) = self.form.suggest.take() else {
            return;
        };
        let (Some(row), Some(found)) = (self.form.row_of(suggest.field), suggest.matches.get(index)) else {
            return;
        };
        self.form.suggest_hold = Some((suggest.field, found.id.clone()));
        if let Some(field) = self.form_field(suggest.field) {
            field.update(cx, |field, cx| field.set_completing(false, cx));
        }
        self.form.apply(row, Some(&found.id), found, cx);
        self.form_changed(cx);
    }

    /// Puts the suggestions away (a click elsewhere, another field, ⎋).
    pub fn close_model_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some(suggest) = self.form.suggest.take() {
            if let Some(field) = self.form_field(suggest.field) {
                field.update(cx, |field, cx| field.set_completing(false, cx));
            }
            cx.notify();
        }
    }

    /// Fetch models: asks the address in the editor which models it has, with the key typed there
    /// or, at the saved address, the saved one (`Store::draft_provider`).
    pub fn fetch_models(&mut self, cx: &mut Context<Self>) {
        self.ensure_catalog(cx);
        self.close_model_suggestions(cx);
        let typed = self.form.typed_key(cx);
        let address = self.form.address.read(cx).text().trim().to_string();
        let provider = self.store.draft_provider(self.form.editing.as_deref(), self.form.format, &address, typed.as_deref());
        self.form.fetch_serial = self.form.fetch_serial.wrapping_add(1);
        self.fetch_subscription = None;
        let provider = match provider {
            Ok(provider) => provider,
            Err(message) => {
                self.form.fetch = crate::settings::Fetch::Failed(message.into());
                cx.notify();
                return;
            }
        };
        let host: SharedString = crate::settings::host_of(&address).into();
        self.form.fetch = crate::settings::Fetch::Loading { host: host.clone() };
        let serial = self.form.fetch_serial;
        let work = Tokio::spawn(cx, async move { provider.list_models().await });
        cx.spawn(async move |app, cx| {
            let result = work.await.unwrap_or_else(|_| Err("Fetching the models stopped.".to_string()));
            let _ = app.update(cx, |app, cx| {
                if app.form.fetch_serial != serial {
                    return;
                }
                app.show_fetched(host, result, &address, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn show_fetched(&mut self, host: SharedString, result: Result<Vec<String>, String>, address: &str, cx: &mut Context<Self>) {
        let ids = match result {
            Ok(ids) => ids,
            Err(message) => {
                self.form.fetch = crate::settings::Fetch::Failed(message.into());
                cx.notify();
                return;
            }
        };
        let present = self.form.model_ids(cx);
        let catalog = self.catalog.current.clone();
        let models = ids
            .into_iter()
            .map(|id| crate::settings::Fetched {
                facts: catalog.as_ref().and_then(|catalog| catalog.lookup(&id, address)).map(|found| crate::settings::listing_facts(&found.listing)),
                present: present.contains(&id),
                ticked: false,
                id,
            })
            .collect();
        let filter = cx.new(|cx| TextField::new(Type::Ui, "Filter", cx));
        self.fetch_subscription = Some(cx.subscribe(&filter, |app, _, event: &TextFieldEvent, cx| match event {
            TextFieldEvent::Changed => cx.notify(),
            TextFieldEvent::Enter => app.add_fetched_models(cx),
            _ => {}
        }));
        self.form.fetch = crate::settings::Fetch::Listed(crate::settings::FetchList { host, models, filter });
        cx.notify();
    }

    pub fn toggle_fetched(&mut self, id: &str, cx: &mut Context<Self>) {
        if let crate::settings::Fetch::Listed(list) = &mut self.form.fetch
            && let Some(model) = list.models.iter_mut().find(|model| model.id == id && !model.present)
        {
            model.ticked = !model.ticked;
            cx.notify();
        }
    }

    /// Ticks every fetched model the connection doesn't have, that the filter leaves.
    pub fn tick_all_fetched(&mut self, cx: &mut Context<Self>) {
        if let crate::settings::Fetch::Listed(list) = &mut self.form.fetch {
            let filter = list.filter.read(cx).text().trim().to_lowercase();
            for model in list.models.iter_mut().filter(|model| !model.present && (filter.is_empty() || model.id.to_lowercase().contains(&filter))) {
                model.ticked = true;
            }
            cx.notify();
        }
    }

    /// Adds the ticked models as folded rows, each filled from the catalogue where it knows the
    /// model; a new connection's lone blank row gives way to them.
    pub fn add_fetched_models(&mut self, cx: &mut Context<Self>) {
        let crate::settings::Fetch::Listed(list) = &self.form.fetch else {
            return;
        };
        let ticked: Vec<String> = list.models.iter().filter(|model| model.ticked).map(|model| model.id.clone()).collect();
        if ticked.is_empty() {
            return;
        }
        if self.form.only_blank_row(cx) {
            self.form.models.clear();
        }
        let address = self.form.address.read(cx).text().to_string();
        let catalog = self.catalog.current.clone();
        for id in ticked {
            self.form.add_model(&id, None, cx);
            let index = self.form.models.len() - 1;
            if let Some(found) = catalog.as_ref().and_then(|catalog| catalog.lookup(&id, &address)) {
                self.form.apply(index, None, &found, cx);
            }
            if let Some(row) = self.form.models.last_mut() {
                row.open = false;
            }
        }
        self.close_fetched_models(cx);
        self.watch_form(cx);
        self.form_changed(cx);
    }

    pub fn close_fetched_models(&mut self, cx: &mut Context<Self>) {
        self.form.fetch = crate::settings::Fetch::Idle;
        self.form.fetch_serial = self.form.fetch_serial.wrapping_add(1);
        self.fetch_subscription = None;
        cx.notify();
    }

    fn form_changed(&mut self, cx: &mut Context<Self>) {
        self.form.error = None;
        self.form.notice = None;
        self.form.notice_serial = self.form.notice_serial.wrapping_add(1);
        cx.notify();
    }

    /// Switches a connection on or off; chats that chose it keep their choice and wait until it
    /// is back on or another model is picked.
    pub fn set_connection_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        match self.store.set_connection_enabled(id, on) {
            Ok(()) => {
                self.form.error = None;
                self.sync_suggestion_views(cx);
            }
            Err(message) => self.form.error = Some(message.into()),
        }
        cx.notify();
    }

    pub fn remove_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.store.remove_connection(id) {
            Ok(()) => {
                self.form.error = None;
                if self.form.editing.as_deref() == Some(id) {
                    self.form.close(cx);
                    self.watch_form(cx);
                }
                self.sync_suggestion_views(cx);
            }
            Err(message) => self.form.error = Some(message.into()),
        }
        cx.notify();
    }

    pub fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) { None } else { Some(menu) };
        cx.notify();
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The model choice the composer on screen is showing: the chat's, or the default.
    pub fn current_choice(&self) -> (Option<String>, ModelChoice) {
        match &self.selection {
            Selection::Chat(id) => match self.store.chat(id) {
                Some(chat) => (Some(id.clone()), chat.choice.clone()),
                None => (None, self.store.defaults().clone()),
            },
            _ => (None, self.store.defaults_for(self.area()).clone()),
        }
    }

    /// Sets the model for the chat on screen (and as the default for new chats).
    pub fn choose(&mut self, connection: Option<String>, model: Option<String>, effort: Option<Option<Reasoning>>, cx: &mut Context<Self>) {
        let (chat, current) = self.current_choice();
        let resolved = match &chat {
            Some(_) => self.store.resolve(&current),
            None => self.store.resolve_default(self.area()),
        };
        let current_effort = resolved.as_ref().map(|resolved| resolved.effort).or(current.effort);
        let choice = ModelChoice {
            connection: connection.or_else(|| resolved.as_ref().map(|resolved| resolved.connection.id.clone())).or(current.connection),
            model: model.or_else(|| resolved.as_ref().map(|resolved| resolved.model.id.clone())).or(current.model),
            effort: effort.unwrap_or(current_effort),
        };
        if let Err(message) = self.store.set_choice(self.area(), chat.as_deref(), choice) {
            self.notice = Some(message.into());
        } else if let Some(id) = chat.as_deref()
            && let Some(listed) = self.store.chat(id)
            && let Some(session) = self.sessions.get(id)
        {
            let view = self.suggestion_view(listed);
            session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
        }
        self.menu = None;
        cx.notify();
    }

    /// "now", "4m", "2h", "3d", "5w" since a chat was last active.
    pub fn age(updated: u64) -> String {
        let seconds = now_millis().saturating_sub(updated) / 1000;
        match seconds {
            0..60 => "now".into(),
            60..3_600 => format!("{}m", seconds / 60),
            3_600..86_400 => format!("{}h", seconds / 3_600),
            86_400..1_209_600 => format!("{}d", seconds / 86_400),
            _ => format!("{}w", seconds / 604_800),
        }
    }

    /// The draft panes for `NewChat`, if any have been opened for the current project.
    pub fn current_new_chat_panes(&self) -> Option<&ChatPanes> {
        self.new_chat_panes.get(&self.new_chat_key()?)
    }

    /// Which new-chat screen's panes are on screen: a project's, or the Chat area's.
    fn new_chat_key(&self) -> Option<String> {
        match &self.selection {
            Selection::NewChat(project) => Some(project.clone()),
            Selection::NewChatArea => Some(CHAT_AREA_PANES.to_string()),
            _ => None,
        }
    }

    /// Changes the panes on `Screen::NewChat` for the selected project.
    pub fn change_new_chat_layout<R>(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Layout) -> R,
    ) -> Option<R> {
        let project_id = self.new_chat_key()?;
        if !self.new_chat_panes.contains_key(&project_id) {
            let mut panes = match self.store.project(&project_id) {
                Some(project) => {
                    let mut panes = ChatPanes::new(Layout::default(), project.path.clone());
                    panes.set_runner(self.runs.clone());
                    panes.set_commands(
                        project.command(CommandKind::Build).map(str::to_string),
                        project.command(CommandKind::Run).map(str::to_string),
                        cx,
                    );
                    panes
                }
                // The Chat area's new chat has only the browser: no folder, terminal or commands.
                None if project_id == CHAT_AREA_PANES => ChatPanes::new(Layout::default(), self.store.chat_area_folder().ok()?),
                None => return None,
            };
            // Tabs of their own, so the Browser pane works before the first message; that
            // message's chat takes them over (`handed_tabs`).
            let tabs = wack_browser::ChatTabs::new(Some(self.browser.clone()));
            let (sender, mut receiver) = mpsc::unbounded::<wack_browser::Signal>();
            tabs.set_observer(move |signal| {
                let _ = sender.unbounded_send(signal);
            });
            let watched = project_id.clone();
            cx.spawn(async move |app, cx| {
                const MAX_BATCH: usize = 4_096;
                while receiver.next().await.is_some() {
                    for _ in 0..MAX_BATCH {
                        if receiver.try_recv().is_err() {
                            break;
                        }
                    }
                    let kept = app.update(cx, |app, cx| {
                        if let Some(panes) = app.new_chat_panes.get(&watched) {
                            panes.refresh_browser(cx);
                        }
                    });
                    if kept.is_err() {
                        return;
                    }
                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                }
            })
            .detach();
            panes.set_browser(tabs);
            self.new_chat_panes.insert(project_id.clone(), panes);
        }
        let panes = self.new_chat_panes.get_mut(&project_id)?;
        let result = change(&mut panes.layout);
        panes.sync(cx, |_, cx| cx.notify());
        cx.notify();
        Some(result)
    }

    /// View data for the `Build` and `Run` split buttons on the current project screen.
    pub fn command_buttons(&self) -> Option<[CommandButtonData; 2]> {
        let project = self.current_project()?;
        let data = |kind: CommandKind| CommandButtonData {
            kind,
            command: project.command(kind).map(Into::into),
            status: self
                .runs
                .session(&project.path, kind)
                .map_or(CommandStatus::Idle, |session| session.info().status),
            editor_open: self
                .command_editor
                .as_ref()
                .is_some_and(|editor| editor.project_id == project.id && editor.kind == kind),
        };
        Some([data(CommandKind::Build), data(CommandKind::Run)])
    }

    /// View data for the currently open command editor popover, if any.
    pub fn command_editor_data(&self, cx: &App) -> Option<CommandEditorData> {
        let editor = self.command_editor.as_ref()?;
        let project = self.store.project(&editor.project_id)?;
        let session = self.runs.session(&project.path, editor.kind);
        let active = session.as_ref().is_some_and(|session| session.is_active());
        let can_run = !editor.input.read(cx).text().trim().is_empty();
        Some(CommandEditorData {
            kind: editor.kind,
            project_name: project.name.clone().into(),
            project_path: shown_path(&project.path).into(),
            input: editor.input.clone(),
            has_session: session.is_some(),
            active,
            can_run,
            error: editor.error.clone(),
        })
    }

    /// Triggers `Build` or `Run`: stops an active session, starts an idle configured one, or opens
    /// the command editor when unconfigured. Returns the input field when the editor opened.
    pub fn trigger_command(&mut self, kind: CommandKind, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let project = self.current_project()?;
        let project_id = project.id.clone();
        let cwd = project.path.clone();
        let configured = project.command(kind).map(str::to_string);
        self.menu = None;
        self.command_editor = None;

        if let Some(session) = self.runs.session(&cwd, kind)
            && session.is_active()
        {
            if session.info().status == CommandStatus::Running {
                self.runs.stop(&cwd, kind);
            }
            cx.notify();
            return None;
        }

        if let Some(command) = configured {
            self.start_project_command(&project_id, kind, command, cx);
            None
        } else {
            self.open_command_editor(kind, cx)
        }
    }

    /// Opens (or toggles closed) the command editor popover for `kind`, returning the focused field
    /// when opened.
    pub fn open_command_editor(&mut self, kind: CommandKind, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let project = self.current_project()?;
        let project_id = project.id.clone();
        if self
            .command_editor
            .as_ref()
            .is_some_and(|editor| editor.project_id == project_id && editor.kind == kind)
        {
            self.command_editor = None;
            cx.notify();
            return None;
        }
        let initial = project.command(kind).unwrap_or("").to_string();
        let input = cx.new(|cx| TextField::new(Type::Mono, command_placeholder(kind), cx));
        input.update(cx, |field, cx| field.set_text(&initial, cx));
        let subscription = cx.subscribe(&input, |app, _, event, cx| match event {
            TextFieldEvent::Enter => {
                let run_after = app.command_editor.as_ref().is_some_and(|editor| {
                    let active = app
                        .store
                        .project(&editor.project_id)
                        .and_then(|project| app.runs.session(&project.path, editor.kind))
                        .is_some_and(|session| session.is_active());
                    !active && !editor.input.read(cx).text().trim().is_empty()
                });
                let _ = app.save_command_editor(run_after, cx);
            }
            TextFieldEvent::SendNow => {
                let _ = app.save_command_editor(false, cx);
            }
            TextFieldEvent::Changed => {
                if let Some(editor) = app.command_editor.as_mut()
                    && editor.error.take().is_some()
                {
                    cx.notify();
                }
            }
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
        });
        self.menu = None;
        self.command_editor = Some(CommandEditorState {
            project_id,
            kind,
            input: input.clone(),
            error: None,
            _subscription: subscription,
        });
        cx.notify();
        Some(input)
    }

    pub fn close_command_editor(&mut self, cx: &mut Context<Self>) {
        if self.command_editor.take().is_some() {
            cx.notify();
        }
    }

    /// Saves the command editor's text, optionally starting the command immediately when idle.
    pub fn save_command_editor(&mut self, run_after: bool, cx: &mut Context<Self>) -> bool {
        let Some((project_id, kind, raw)) = self
            .command_editor
            .as_ref()
            .map(|editor| (editor.project_id.clone(), editor.kind, editor.input.read(cx).text().to_string()))
        else {
            return false;
        };
        if let Err(message) = self.store.set_project_command(&project_id, kind, Some(&raw)) {
            if let Some(editor) = self.command_editor.as_mut() {
                editor.error = Some(message.into());
            }
            cx.notify();
            return false;
        }
        self.command_editor = None;
        self.sync_project_commands(&project_id, cx);

        if run_after
            && let Some(project) = self.store.project(&project_id)
            && let Some(command) = project.command(kind).map(str::to_string)
        {
            let active = self
                .runs
                .session(&project.path, kind)
                .is_some_and(|session| session.is_active());
            if !active {
                self.start_project_command(&project_id, kind, command, cx);
                return true;
            }
        }
        cx.notify();
        true
    }

    /// Brings up the `Build` or `Run` output pane on the current screen without stopping a run.
    pub fn show_command_output(&mut self, kind: CommandKind, cx: &mut Context<Self>) {
        self.command_editor = None;
        self.open_command_pane(kind, cx);
        cx.notify();
    }

    pub fn clear_command_output(&mut self, kind: CommandKind, cx: &mut Context<Self>) {
        let Some(cwd) = self.current_project().map(|project| project.path.clone()) else {
            return;
        };
        if let Some(session) = self.runs.session(&cwd, kind) {
            session.clear_output();
            self.wake_command_for_cwd(&cwd, kind, cx);
            cx.notify();
        }
    }

    pub fn stop_command(&mut self, kind: CommandKind, cx: &mut Context<Self>) {
        let Some(cwd) = self.current_project().map(|project| project.path.clone()) else {
            return;
        };
        self.runs.stop(&cwd, kind);
        cx.notify();
    }

    /// Terminates every active Build/Run process group, every chat's background commands and
    /// every MCP server, cleanly when WackCode quits.
    pub fn shutdown_runs(&self) {
        let tasks: Vec<_> = self.chat_tasks.values().cloned().collect();
        let mut stopping: Vec<_> = tasks.into_iter().map(|tasks| std::thread::spawn(move || tasks.terminate_all())).collect();
        let agents: Vec<_> = self.chat_agents.values().cloned().collect();
        stopping.extend(agents.into_iter().map(|agents| std::thread::spawn(move || agents.terminate_all())));
        let mcp = self.mcp.clone();
        stopping.push(std::thread::spawn(move || mcp.terminate_all()));
        self.runs.terminate_all();
        for thread in stopping {
            let _ = thread.join();
        }
        wack_browser::cef::shutdown();
    }

    fn open_command_pane(&mut self, kind: CommandKind, cx: &mut Context<Self>) {
        let pane_kind = command_pane_kind(kind);
        match self.selection.clone() {
            Selection::Chat(chat_id) => {
                if let Some(session) = self.sessions.get(&chat_id).cloned() {
                    session.update(cx, |session, cx| {
                        session.change_layout(cx, |layout| layout.open(pane_kind));
                    });
                }
            }
            Selection::NewChat(_) => {
                self.change_new_chat_layout(cx, |layout| layout.open(pane_kind));
            }
            Selection::Welcome | Selection::NewChatArea | Selection::Settings => {}
        }
    }

    fn start_project_command(&mut self, project_id: &str, kind: CommandKind, command: String, cx: &mut Context<Self>) {
        let Some(cwd) = self.store.project(project_id).map(|project| project.path.clone()) else {
            return;
        };
        self.open_command_pane(kind, cx);
        let tx = self.command_event_tx.clone();
        match self
            .runs
            .start(&cwd, kind, &command, wack_core::runner::PtySize::default(), move |event| {
                let _ = tx.unbounded_send(event);
            }) {
            Ok(_) => {
                self.notice = None;
                self.sync_command_for_cwd(&cwd, kind, cx);
            }
            Err(message) => {
                self.notice = Some(message.clone().into());
                if !matches!(self.selection, Selection::NewChat(_)) {
                    let _ = self.open_command_editor(kind, cx);
                    if let Some(editor) = self.command_editor.as_mut() {
                        editor.error = Some(message.into());
                    }
                }
            }
        }
        cx.notify();
    }

    fn handle_command_events(&mut self, events: Vec<CommandEvent>, cx: &mut Context<Self>) {
        let mut wake_targets: Vec<(PathBuf, CommandKind)> = Vec::new();
        let mut state_targets: Vec<(PathBuf, CommandKind)> = Vec::new();
        for event in events {
            match event {
                CommandEvent::Wakeup { kind, cwd, .. } => {
                    if !wake_targets.contains(&(cwd.clone(), kind)) {
                        wake_targets.push((cwd, kind));
                    }
                }
                CommandEvent::StateChanged(info) => {
                    if !state_targets.contains(&(info.cwd.clone(), info.kind)) {
                        state_targets.push((info.cwd, info.kind));
                    }
                }
            }
        }
        for (cwd, kind) in wake_targets {
            self.wake_command_for_cwd(&cwd, kind, cx);
        }
        if !state_targets.is_empty() {
            for (cwd, kind) in state_targets {
                self.sync_command_for_cwd(&cwd, kind, cx);
            }
            cx.notify();
        }
    }

    fn wake_command_for_cwd(&self, cwd: &Path, kind: CommandKind, cx: &mut App) {
        let canonical = Runner::canonical_cwd(cwd);
        for chat in self.sessions.values().cloned().collect::<Vec<_>>() {
            chat.update(cx, |chat, cx| {
                if Runner::canonical_cwd(chat.panes.folder()) == canonical {
                    chat.panes.wake_command(kind, cx);
                }
            });
        }
        for panes in self.new_chat_panes.values() {
            if Runner::canonical_cwd(panes.folder()) == canonical {
                panes.wake_command(kind, cx);
            }
        }
    }

    fn sync_command_for_cwd(&self, cwd: &Path, kind: CommandKind, cx: &mut App) {
        let canonical = Runner::canonical_cwd(cwd);
        let session = self.runs.session(&canonical, kind);
        let configured = self
            .store
            .projects()
            .iter()
            .find(|project| Runner::canonical_cwd(&project.path) == canonical)
            .and_then(|project| project.command(kind))
            .map(str::to_string);
        for chat in self.sessions.values().cloned().collect::<Vec<_>>() {
            chat.update(cx, |chat, cx| {
                if Runner::canonical_cwd(chat.panes.folder()) == canonical {
                    chat.panes.sync_command_session(kind, session.clone(), configured.clone(), cx);
                }
            });
        }
        for panes in self.new_chat_panes.values() {
            if Runner::canonical_cwd(panes.folder()) == canonical {
                panes.sync_command_session(kind, session.clone(), configured.clone(), cx);
            }
        }
    }

    fn sync_project_commands(&mut self, project_id: &str, cx: &mut App) {
        let Some(project) = self.store.project(project_id) else {
            return;
        };
        let canonical = Runner::canonical_cwd(&project.path);
        let build = project.command(CommandKind::Build).map(str::to_string);
        let run = project.command(CommandKind::Run).map(str::to_string);
        for chat in self.sessions.values().cloned().collect::<Vec<_>>() {
            chat.update(cx, |chat, cx| {
                if Runner::canonical_cwd(chat.panes.folder()) == canonical {
                    chat.panes.set_commands(build.clone(), run.clone(), cx);
                }
            });
        }
        for panes in self.new_chat_panes.values_mut() {
            if Runner::canonical_cwd(panes.folder()) == canonical {
                panes.set_commands(build.clone(), run.clone(), cx);
            }
        }
    }
}

/// Puts messages back in a chat's composer, ahead of anything already there.
/// What happens to the messages waiting when a run ends.
#[derive(Debug, PartialEq)]
struct AfterRun {
    /// Sent now.
    next: Option<String>,
    /// Still queued behind it.
    queue: Vec<String>,
    /// Handed back to the composer.
    unsent: Vec<String>,
}

/// A run that finished sends the next waiting message; one that failed or was stopped hands them
/// all back. So does a run that ends with a plan to review: sending would consume the plan unseen.
/// Steering that arrived as the run ended goes first: it was written before the queue.
fn after_run(done: bool, holding_plan: bool, steering: Vec<String>, queue: Vec<String>) -> AfterRun {
    let mut waiting = steering;
    waiting.extend(queue);
    if done && !holding_plan && !waiting.is_empty() {
        let next = waiting.remove(0);
        AfterRun { next: Some(next), queue: waiting, unsent: Vec::new() }
    } else {
        AfterRun { next: None, queue: Vec::new(), unsent: waiting }
    }
}

/// What a message made of comments alone says.
const COMMENTS_ONLY: &str = "I left comments on the canvas.";
/// The most pictures of commented elements one message carries.
const MOST_CROPS: usize = 6;

/// A picture of each commented element, for a model that sees pictures.
fn comment_crops(canvases: &wack_canvas::Canvases, armed: &[(String, wack_canvas::comments::Comment)]) -> Vec<wack_core::llm::Image> {
    armed
        .iter()
        .filter_map(|(canvas, comment)| {
            let png = canvases.crop(canvas, comment)?;
            Some(wack_core::llm::Image::new("image/png", wack_core::images::encode(&png)).labelled(format!("the element of comment {} on artboard {}", comment.id, comment.artboard)))
        })
        .take(MOST_CROPS)
        .collect()
}

/// `job` with more pictures, kept to what one message may carry.
fn with_images(job: Job, mut extra: Vec<wack_core::llm::Image>) -> Job {
    if extra.is_empty() {
        return job;
    }
    match job {
        Job::Prompt(text) => {
            extra.truncate(crate::attachments::MAX_ATTACHMENTS);
            Job::Attached { text, command: None, images: extra }
        }
        Job::Command { text, command } => {
            extra.truncate(crate::attachments::MAX_ATTACHMENTS);
            Job::Attached { text, command: Some(command), images: extra }
        }
        Job::Attached { text, command, mut images } => {
            extra.truncate(crate::attachments::MAX_ATTACHMENTS.saturating_sub(images.len()));
            images.extend(extra);
            Job::Attached { text, command, images }
        }
        other => other,
    }
}

/// `job` carrying `pictures` the user attached, when there are any.
fn with_pictures(job: Job, pictures: &[crate::attachments::Attachment]) -> Job {
    if pictures.is_empty() {
        return job;
    }
    let images = pictures.iter().map(crate::attachments::Attachment::image).collect();
    match job {
        Job::Prompt(text) => Job::Attached { text, command: None, images },
        Job::Command { text, command } => Job::Attached { text, command: Some(command), images },
        other => other,
    }
}

/// Messages that were not sent go back to the composer, their pictures back to its tray.
/// Puts unsent `messages` back in the composer, with their pictures. Returns the chats they
/// referenced, their sections split off the words.
fn restore(session: &Entity<ChatSession>, messages: Vec<String>, cx: &mut App) -> Vec<ChatRef> {
    session.update(cx, |session, cx| {
        for message in &messages {
            let pictures = session.take_queued_pictures(message);
            let room = crate::attachments::MAX_ATTACHMENTS.saturating_sub(session.attachments.len());
            session.attachments.extend(pictures.into_iter().take(room));
        }
        cx.notify();
    });
    let mut refs = Vec::new();
    let mut restored_files = Vec::new();
    let messages: Vec<String> = messages
        .into_iter()
        .map(|message| {
            let (words, referenced) = wack_core::chat_refs::split(&message);
            refs.extend(referenced);
            let (words, files) = crate::attachments::split(&words);
            restored_files.extend(files);
            words.to_string()
        })
        .collect();
    if !restored_files.is_empty() {
        session.update(cx, |session, cx| {
            session.files.extend(restored_files);
            cx.notify();
        });
    }
    let composer = session.read(cx).composer.clone();
    composer.update(cx, |composer, cx| {
        let mut parts = messages;
        let existing = composer.text().trim().to_string();
        if !existing.is_empty() {
            parts.push(existing);
        }
        composer.set_text(&parts.join("\n\n"), cx);
    });
    refs
}

/// `~/Documents/x` for a folder in the home folder.
pub fn shown_path(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        None => path.display().to_string(),
    }
}

// -------------------------------------------------------------------------------------------------
// Rewinding, editing and retrying the user's messages (`rewind.rs` draws them; the engine's
// `wack_core::rewind` does the work, off the UI thread)

impl AppState {
    /// The chat on screen, by id, with its session.
    fn chat_on_screen(&self) -> Option<(String, Entity<ChatSession>)> {
        match &self.selection {
            Selection::Chat(id) => self.sessions.get(id).map(|session| (id.clone(), session.clone())),
            _ => None,
        }
    }

    pub fn open_prompt_menu(&mut self, number: usize, position: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        let Some((chat, _)) = self.chat_on_screen() else {
            return;
        };
        self.prompt_menu = Some(PromptMenu { chat, number, position });
        cx.notify();
    }

    pub fn close_prompt_menu(&mut self, cx: &mut Context<Self>) {
        if self.prompt_menu.take().is_some() {
            cx.notify();
        }
    }

    /// The open message menu: where, and what it offers.
    pub fn prompt_menu_facts(&self, cx: &App) -> Option<(gpui::Point<gpui::Pixels>, crate::context_menu::PromptFacts)> {
        let menu = self.prompt_menu.as_ref()?;
        let prompts = self.sessions.get(&menu.chat)?.read(cx).prompt_view();
        Some((menu.position, crate::context_menu::PromptFacts { number: menu.number, idle: prompts.idle, latest: menu.number + 1 == prompts.count }))
    }

    /// Copies the words of the user's `number`-th message: a slash command as typed.
    pub fn copy_prompt(&mut self, number: usize, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        if let Some(prompt) = session.read(cx).sent_prompt(number) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(prompt.words));
        }
    }

    /// Opens the user's `number`-th message for editing, in place of its bubble, with the
    /// keyboard in it. Only while the chat is idle.
    pub fn edit_prompt(&mut self, number: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        if !session.read(cx).prompt_view().idle {
            return;
        }
        let Some(prompt) = session.read(cx).sent_prompt(number) else {
            return;
        };
        let field = cx.new(|cx| {
            let mut field = TextField::new(crate::typography::Type::Body, "Write the message again", cx).multiline(COMPOSER_ROWS);
            field.set_text(&prompt.words, cx);
            field
        });
        let entered = cx.subscribe(&field, |app, _, event: &TextFieldEvent, cx| {
            if matches!(event, TextFieldEvent::Enter | TextFieldEvent::SendNow) {
                app.send_edit(cx);
            }
        });
        let focus = field.read(cx).focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        session.update(cx, |session, cx| {
            let before = [session.editing.as_ref().map(|editing| editing.number), session.rewind_card.as_ref().map(|card| card.number)];
            session.rewind_card = None;
            session.resend = None;
            session.editing = Some(crate::chat::Editing { number, field, _subscriptions: vec![entered] });
            for shown in before.into_iter().flatten().chain([number]) {
                session.remeasure_prompt(shown);
            }
            cx.notify();
        });
    }

    /// Leaves the message being edited as it was, and gives the composer the keyboard back.
    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        let composer = session.update(cx, |session, cx| {
            let editing = session.editing.take()?;
            // An edit already asking about its files goes back to editing first.
            if session.rewind_card.as_ref().is_some_and(|card| card.number == editing.number && !card.working) {
                session.rewind_card = None;
                session.resend = None;
            }
            session.remeasure_prompt(editing.number);
            cx.notify();
            Some(session.composer.read(cx).focus_handle(cx))
        });
        if let Some(composer) = composer {
            window.focus(&composer, cx);
        }
    }

    /// Sends the message being edited. What it sends is worked out first, so a message that
    /// can't be sent leaves the chat as it was; then the chat goes back to before it (asking
    /// about the files when they changed) and it runs.
    pub fn send_edit(&mut self, cx: &mut Context<Self>) {
        let Some((chat, session)) = self.chat_on_screen() else {
            return;
        };
        let Some((number, words)) = session.read(cx).editing.as_ref().map(|editing| (editing.number, editing.field.read(cx).text().trim().to_string())) else {
            return;
        };
        self.resend_prompt(&chat, number, words, crate::rewind::Purpose::Edit, cx);
    }

    /// Sends the user's `number`-th message again, as it was.
    pub fn retry_prompt(&mut self, number: usize, cx: &mut Context<Self>) {
        let Some((chat, session)) = self.chat_on_screen() else {
            return;
        };
        let Some(prompt) = session.read(cx).sent_prompt(number) else {
            return;
        };
        self.resend_prompt(&chat, number, prompt.words, crate::rewind::Purpose::Retry, cx);
    }

    fn resend_prompt(&mut self, chat: &str, number: usize, words: String, purpose: crate::rewind::Purpose, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        if !session.read(cx).prompt_view().idle || session.read(cx).rewind_card.as_ref().is_some_and(|card| card.working) {
            return;
        }
        match self.resend_job(chat, number, &words, cx) {
            Ok(job) => {
                session.update(cx, |session, cx| {
                    let before = session.rewind_card.as_ref().map(|card| card.number);
                    session.resend = Some(job);
                    session.rewind_card = Some(crate::rewind::Card::new(number, purpose));
                    for shown in before.into_iter().chain([number]) {
                        session.remeasure_prompt(shown);
                    }
                    cx.notify();
                });
                self.look_before_rewind(chat, number, cx);
            }
            Err(message) => session.update(cx, |session, cx| session.note(message, cx)),
        }
    }

    /// What sending `words` again as the user's `number`-th message runs, with the pictures and
    /// files it carried: resolved as the composer resolves a message (a slash command, the chats
    /// it referenced), and refused for the same reasons.
    fn resend_job(&mut self, chat: &str, number: usize, words: &str, cx: &mut Context<Self>) -> Result<Job, String> {
        let session = self.sessions.get(chat).cloned().ok_or("That chat is no longer open.")?;
        let prompt = session.read(cx).sent_prompt(number).ok_or("That message is no longer in the chat.")?;
        let listed = self.store.chat(chat).cloned().ok_or("That chat is no longer in WackCode.")?;
        let words = words.trim();
        if words.is_empty() && prompt.pictures.is_empty() && prompt.files.is_empty() {
            return Err("Write something to send.".into());
        }
        if !prompt.files.is_empty() && words.starts_with('/') {
            return Err(FILES_WITH_COMMAND.into());
        }
        if let Some(problem) = self.store.chat_model_problem(&listed) {
            return Err(problem.into());
        }
        if let Some(refused) = self.pictures_refused(self.store.resolve(&listed.choice).as_ref(), prompt.pictures.len()) {
            return Err(refused);
        }
        if let Some(missing) = self.crew_missing(listed.team.as_ref()) {
            return Err(missing);
        }
        let text = crate::attachments::compose(words, &prompt.files);
        let cwd = session.read(cx).cwd().to_path_buf();
        let outgoing = if listed.is_chat_area() {
            match chat_area_outgoing(&text) {
                // The chats it referenced go again while their `@"Name"` is still in the words.
                Outgoing::Message(text) => Outgoing::Message(chat_refs::encode(&text, &chat_refs::surviving(&text, &prompt.refs))),
                other => other,
            }
        } else {
            self.outgoing(Some(chat), &text, &cwd)?
        };
        let job = match outgoing {
            Outgoing::Builtin { name, .. } => return Err(format!("/{name} is one of WackCode's own commands: use it from the composer.")),
            Outgoing::Message(text) => Job::Prompt(text),
            Outgoing::Prompt { text, command } => Job::Command { text, command },
        };
        Ok(with_images(job, prompt.pictures.iter().filter_map(picture_image).collect()))
    }

    /// Asks to take the chat on screen back to before the user's `number`-th message: the card
    /// under it, filled in once the engine has looked at the files.
    pub fn rewind_prompt(&mut self, number: usize, cx: &mut Context<Self>) {
        let Some((chat, session)) = self.chat_on_screen() else {
            return;
        };
        if !session.read(cx).prompt_view().idle || session.read(cx).rewind_card.as_ref().is_some_and(|card| card.working) {
            return;
        }
        session.update(cx, |session, cx| {
            let before = [session.editing.take().map(|editing| editing.number), session.rewind_card.as_ref().map(|card| card.number)];
            session.resend = None;
            session.rewind_card = Some(crate::rewind::Card::new(number, crate::rewind::Purpose::Rewind));
            for shown in before.into_iter().flatten().chain([number]) {
                session.remeasure_prompt(shown);
            }
            cx.notify();
        });
        self.look_before_rewind(&chat, number, cx);
    }

    /// The chat's file, folder and checkpoint store, for the engine to work on off the UI
    /// thread.
    fn rewind_parts(&self, chat: &str, cx: &App) -> Option<(PathBuf, PathBuf, wack_core::checkpoints::Checkpoints)> {
        let listed = self.store.chat(chat)?;
        let session = self.sessions.get(chat)?.read(cx);
        Some((session.session_path().to_path_buf(), session.cwd().to_path_buf(), self.store.checkpoints(listed)))
    }

    /// Asks the engine what rewinding to before message `number` would find. An edit or a retry
    /// with no changed files to ask about goes straight on.
    fn look_before_rewind(&mut self, chat: &str, number: usize, cx: &mut Context<Self>) {
        let Some((path, cwd, store)) = self.rewind_parts(chat, cx) else {
            return;
        };
        let chat = chat.to_string();
        cx.spawn(async move |app, cx| {
            let preview = cx.background_spawn(async move { wack_core::rewind::preview(&path, number, &store, &cwd) }).await;
            let _ = app.update(cx, |app, cx| app.found_before_rewind(&chat, number, preview, cx));
        })
        .detach();
    }

    fn found_before_rewind(&mut self, chat: &str, number: usize, preview: Result<wack_core::rewind::Preview, String>, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let straight_on = session.update(cx, |session, cx| {
            // Closed, or asked again about another message, since.
            let Some(card) = session.rewind_card.as_mut().filter(|card| card.number == number && card.preview.is_none()) else {
                return false;
            };
            match preview {
                Ok(preview) => card.found(preview),
                Err(message) => card.error = Some(message.into()),
            }
            let straight_on = card.preview.is_some() && card.purpose != crate::rewind::Purpose::Rewind && !card.has_changes();
            session.remeasure_prompt(number);
            cx.notify();
            straight_on
        });
        if straight_on {
            self.confirm_rewind(chat, false, cx);
        }
    }

    /// The rewind card's tick for one changed file.
    pub fn toggle_rewind_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        session.update(cx, |session, cx| {
            if let Some(card) = session.rewind_card.as_mut().filter(|card| !card.working) {
                if !card.ticked.remove(path) {
                    card.ticked.insert(path.to_string());
                }
                cx.notify();
            }
        });
    }

    /// The rewind card's tick for every changed file: all on, or, when all are on, all off.
    pub fn toggle_rewind_files(&mut self, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        session.update(cx, |session, cx| {
            let Some(card) = session.rewind_card.as_mut().filter(|card| !card.working) else {
                return;
            };
            let Some(preview) = &card.preview else {
                return;
            };
            if card.ticked.len() == preview.changes.len() {
                card.ticked.clear();
            } else {
                card.ticked = preview.changes.iter().map(|change| change.path.clone()).collect();
            }
            cx.notify();
        });
    }

    /// Closes the card under a message without rewinding. An edit's field stays open.
    pub fn cancel_rewind(&mut self, cx: &mut Context<Self>) {
        let Some((_, session)) = self.chat_on_screen() else {
            return;
        };
        session.update(cx, |session, cx| {
            if let Some(card) = session.rewind_card.take_if(|card| !card.working) {
                session.resend = None;
                session.remeasure_prompt(card.number);
                cx.notify();
            }
        });
    }

    /// Does what the rewind card on screen asks.
    pub fn confirm_rewind_on_screen(&mut self, files: bool, cx: &mut Context<Self>) {
        if let Some((chat, _)) = self.chat_on_screen() {
            self.confirm_rewind(&chat, files, cx);
        }
    }

    /// Takes the chat back to before the card's message, off the UI thread: with `files`, the
    /// ticked files go back as they were first. Then a rewind puts the message's words back in
    /// the composer, and an edit or a retry sends.
    fn confirm_rewind(&mut self, chat: &str, files: bool, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let Some((path, cwd, store)) = self.rewind_parts(chat, cx) else {
            return;
        };
        let card = {
            let session = session.read(cx);
            if session.running() || session.rewinding {
                return;
            }
            match session.rewind_card.clone() {
                Some(card) if !card.working && card.preview.is_some() => card,
                _ => return,
            }
        };
        let restore = files
            .then(|| card.preview.as_ref().and_then(|preview| preview.checkpoint.clone()))
            .flatten()
            .map(|checkpoint| (checkpoint, card.ticked.iter().cloned().collect::<Vec<_>>()));
        session.update(cx, |session, cx| {
            if let Some(card) = session.rewind_card.as_mut() {
                card.working = true;
                card.error = None;
            }
            session.rewinding = true;
            cx.notify();
        });
        let (chat, number, purpose) = (chat.to_string(), card.number, card.purpose);
        cx.spawn(async move |app, cx| {
            let result = cx
                .background_spawn(async move {
                    let restore = restore.as_ref().map(|(checkpoint, paths)| wack_core::rewind::Restore { store: &store, cwd: &cwd, checkpoint, paths });
                    wack_core::rewind::rewind(&path, number, restore)
                })
                .await;
            let _ = app.update(cx, |app, cx| app.rewound(&chat, number, purpose, result, cx));
        })
        .detach();
    }

    fn rewound(&mut self, chat: &str, number: usize, purpose: crate::rewind::Purpose, result: Result<wack_core::rewind::Files, String>, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let files = match result {
            Ok(files) => files,
            Err(message) => {
                session.update(cx, |session, cx| {
                    session.rewinding = false;
                    if let Some(card) = session.rewind_card.as_mut() {
                        card.working = false;
                        card.error = Some(message.into());
                    }
                    session.remeasure_prompt(number);
                    cx.notify();
                });
                return;
            }
        };
        // The message as it was, read before the conversation is read again without it.
        let prompt = session.read(cx).sent_prompt(number);
        let job = session.update(cx, |session, cx| {
            session.rewinding = false;
            session.rewind_card = None;
            session.editing = None;
            let job = session.resend.take();
            session.reload(cx);
            if !files.skipped.is_empty() {
                let shown: Vec<&str> = files.skipped.iter().take(3).map(String::as_str).collect();
                let more = if files.skipped.len() > 3 { "…" } else { "" };
                session.note(
                    format!(
                        "{} left as they are: something the snapshot doesn't hold is in the way (an ignored or very large file): {}{more}",
                        if files.skipped.len() == 1 { "1 file was".to_string() } else { format!("{} files were", files.skipped.len()) },
                        shown.join(", ")
                    ),
                    cx,
                );
            }
            job
        });
        match purpose {
            crate::rewind::Purpose::Rewind => {
                if let Some(prompt) = prompt {
                    self.words_back(chat, prompt, cx);
                }
            }
            crate::rewind::Purpose::Edit | crate::rewind::Purpose::Retry => {
                if let Some(job) = job {
                    self.run(chat, job, cx);
                }
            }
        }
        cx.notify();
    }

    /// Puts a rewound message back in its chat's composer, ahead of anything written there:
    /// its words, its files, the chats it referenced and (decoded off the UI thread) its
    /// pictures.
    fn words_back(&mut self, chat: &str, prompt: crate::chat::SentPrompt, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let message = chat_refs::encode(&crate::attachments::compose(&prompt.words, &prompt.files), &prompt.refs);
        let refs = restore(&session, vec![message], cx);
        self.keep_refs(chat, refs);
        let composer = session.read(cx).composer.clone();
        let written = composer.read(cx).text().to_string();
        session.update(cx, |session, cx| {
            session.rewound_words = Some(written);
            session.focus_request = Some(session.composer.read(cx).focus_handle(cx));
            cx.notify();
        });
        if prompt.pictures.is_empty() {
            return;
        }
        let pictures = prompt.pictures;
        cx.spawn(async move |_, cx| {
            let attachments = cx
                .background_spawn(async move {
                    pictures
                        .iter()
                        .filter_map(|picture| {
                            let bytes = wack_core::images::decode(&picture.data).ok()?;
                            let name = picture.label.as_ref().map_or("Picture".to_string(), |label| label.to_string());
                            crate::attachments::Attachment::from_bytes(bytes, &name).ok()
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            session.update(cx, |session, cx| {
                let room = crate::attachments::MAX_ATTACHMENTS.saturating_sub(session.attachments.len());
                session.attachments.extend(attachments.into_iter().take(room));
                cx.notify();
            });
        })
        .detach();
    }

    /// Undoes the rewind just made in the chat on screen: the messages after it come back, and
    /// the files it put back go as they were. The words it put in the composer leave it, if
    /// they are still all that is there.
    pub fn undo_rewind(&mut self, cx: &mut Context<Self>) {
        let Some((chat, session)) = self.chat_on_screen() else {
            return;
        };
        let Some((path, cwd, store)) = self.rewind_parts(&chat, cx) else {
            return;
        };
        if session.read(cx).running() || session.read(cx).rewinding {
            return;
        }
        session.update(cx, |session, cx| {
            session.rewinding = true;
            cx.notify();
        });
        cx.spawn(async move |_, cx| {
            let result = cx.background_spawn(async move { wack_core::rewind::undo(&path, &store, &cwd) }).await;
            session.update(cx, |session, cx| {
                session.rewinding = false;
                session.reload(cx);
                match result {
                    Ok(_) => {
                        let composer = session.composer.clone();
                        if session.rewound_words.take().is_some_and(|words| composer.read(cx).text() == words) {
                            composer.update(cx, |composer, cx| composer.set_text("", cx));
                            session.attachments.clear();
                            session.files.clear();
                        }
                    }
                    Err(message) => session.note(message, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// A picture the user attached to a message, to send with it again: its media type read from
/// its first bytes.
fn picture_image(picture: &crate::pictures::Picture) -> Option<wack_core::llm::Image> {
    let head: String = picture.data.chars().take(24).collect();
    let media_type = wack_core::images::media_type(&wack_core::images::decode(&head).ok()?)?;
    let image = wack_core::llm::Image::new(media_type, picture.data.to_string());
    Some(match &picture.label {
        Some(label) => image.labelled(label.to_string()),
        None => image,
    })
}

// -------------------------------------------------------------------------------------------------
// Settings › Skills

impl AppState {
    fn refresh_skills(&mut self) {
        self.skills.folders = self.store.skills_overview();
    }

    fn skills_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.skills.error = result.err().map(Into::into);
        self.refresh_skills();
        cx.notify();
    }

    pub fn switch_skill(&mut self, key: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_command_enabled(key, on);
        self.skills_result(result, cx);
    }

    pub fn switch_skill_folder(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_skill_folder_enabled(id, on);
        self.skills_result(result, cx);
    }

    pub fn add_skill_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let result = self.store.add_skill_folder(path);
        self.skills_result(result, cx);
    }

    pub fn remove_skill_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        let result = self.store.remove_skill_folder(id);
        self.skills_result(result, cx);
    }

    /// Imports skills into Your skills and says what came in and what was skipped.
    pub fn import_skills(&mut self, source: &Path, cx: &mut Context<Self>) {
        let names = |names: &[String]| names.iter().map(|name| format!("/{name}")).collect::<Vec<_>>().join(", ");
        match self.store.import_skills(source) {
            Ok((imported, skipped)) => {
                let mut words = Vec::new();
                if !imported.is_empty() {
                    words.push(format!("Imported {}.", names(&imported)));
                }
                if !skipped.is_empty() {
                    words.push(format!("Skipped {}: Your skills already has one by that name, or the name doesn't follow the skill format.", names(&skipped)));
                }
                self.skills.notice = Some(words.join(" ").into());
                self.skills_result(Ok(()), cx);
            }
            Err(message) => {
                self.skills.notice = None;
                self.skills_result(Err(message), cx);
            }
        }
    }

    /// Opens the skill at `file`: in the editor when it is in Your skills, else read-only.
    pub fn open_skill(&mut self, file: &Path, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        self.refresh_skills();
        let (folder, entry) = self.skills.find(file)?;
        let (library, folder_label, entry) = (folder.kind == wack_core::skills::FolderKind::Library, folder.label.clone(), entry.clone());
        self.skills.error = None;
        self.skills.notice = None;
        if !library {
            let body = std::fs::read_to_string(file).map(|text| wack_core::skills::split_body(&text)).unwrap_or_default();
            self.skills.viewing = Some(Viewing { entry, folder: folder_label, body });
            cx.notify();
            return None;
        }
        let document = match wack_core::skills::read_document(file) {
            Ok(document) => document,
            Err(message) => {
                self.skills_result(Err(message), cx);
                return None;
            }
        };
        Some(self.start_skill_editor(SkillEditor::new(Some(file.to_path_buf()), Some(&document), cx), cx))
    }

    /// The editor for a new skill. Returns its name field, for focus.
    pub fn new_skill(&mut self, cx: &mut Context<Self>) -> Entity<TextField> {
        self.skills.viewing = None;
        self.skills.notice = None;
        self.start_skill_editor(SkillEditor::new(None, None, cx), cx)
    }

    fn start_skill_editor(&mut self, editor: SkillEditor, cx: &mut Context<Self>) -> Entity<TextField> {
        let name = editor.name.clone();
        self.skills.editor = Some(editor);
        self.watch_skill_editor(cx);
        cx.notify();
        name
    }

    pub fn close_skill(&mut self, cx: &mut Context<Self>) {
        self.skills.editor = None;
        self.skills.viewing = None;
        self.skill_subscriptions.clear();
        self.refresh_skills();
        cx.notify();
    }

    pub fn choose_skill_use(&mut self, manual: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.skills.editor.as_mut() {
            editor.manual = manual;
            editor.error = None;
            cx.notify();
        }
    }

    /// Saves the skill the editor describes, then goes back to the list.
    pub fn save_skill(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.skills.editor.as_mut() else {
            return;
        };
        if let Some(issue) = editor.issue(cx) {
            editor.error = Some(issue);
            cx.notify();
            return;
        }
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        let (name, description, hint, body) = (text(&editor.name), text(&editor.description), text(&editor.hint), text(&editor.body));
        let manual = editor.manual;
        let file = editor.file.clone();
        let result = match &file {
            Some(file) => self.store.update_skill(file, &name, &description, manual, &hint, &body),
            None => self.store.create_skill(&name, &description, manual, &hint, &body),
        };
        match result {
            Ok(_) => self.close_skill(cx),
            Err(message) => {
                if let Some(editor) = self.skills.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    pub fn ask_delete_skill(&mut self, ask: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.skills.editor.as_mut() {
            editor.confirming = ask && editor.file.is_some();
            cx.notify();
        }
    }

    /// Moves the skill in the editor to the macOS Trash.
    pub fn confirm_skill_delete(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.skills.editor.as_ref().filter(|editor| editor.confirming).and_then(|editor| editor.file.clone()) else {
            return;
        };
        match self.store.delete_skill(&file, &crate::memory_page::move_to_trash) {
            Ok(()) => self.close_skill(cx),
            Err(message) => {
                if let Some(editor) = self.skills.editor.as_mut() {
                    editor.confirming = false;
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    /// Copies a skill from another folder into Your skills and opens the copy.
    pub fn copy_skill(&mut self, file: &Path, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let skill = self.skills.find(file)?.1.skill.clone();
        match self.store.copy_skill_to_library(&skill) {
            Ok(copy) => {
                self.skills.viewing = None;
                let field = self.open_skill(&copy, cx);
                self.skills.notice = Some(format!("Copied /{} to Your skills.", skill.name).into());
                field
            }
            Err(message) => {
                self.skills_result(Err(message), cx);
                None
            }
        }
    }

    fn watch_skill_editor(&mut self, cx: &mut Context<Self>) {
        self.skill_subscriptions.clear();
        let Some(editor) = self.skills.editor.as_ref() else {
            return;
        };
        let changed = |app: &mut AppState, cx: &mut Context<AppState>| {
            if let Some(editor) = app.skills.editor.as_mut() {
                editor.error = None;
            }
            cx.notify();
        };
        for (index, field) in editor.fields().into_iter().enumerate() {
            // The name and the hint are one line: ↩ saves. In the description and the
            // instructions ↩ is a new line and ⌘↩ saves.
            let one_line = index == 0 || index == 2;
            self.skill_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter if !one_line => field.update(cx, |field, cx| field.insert_line_break(cx)),
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_skill(cx),
                TextFieldEvent::Changed => changed(app, cx),
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Settings › Commands

impl AppState {
    fn refresh_templates(&mut self) {
        self.templates.entries = self.store.commands_overview();
        self.templates.folder = self.store.commands_folder();
    }

    fn templates_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.templates.error = result.err().map(Into::into);
        self.refresh_templates();
        cx.notify();
    }

    pub fn switch_slash_command(&mut self, key: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_command_enabled(key, on);
        self.templates_result(result, cx);
    }

    /// The editor for the user's command `name`, or a new one. Returns its name field.
    pub fn open_slash_command(&mut self, name: Option<&str>, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let template = match name {
            Some(name) => match self.store.read_template(name) {
                Some(template) => Some(template),
                None => {
                    self.templates_result(Err(format!("/{name} could not be read. It may have been moved or deleted.")), cx);
                    return None;
                }
            },
            None => None,
        };
        let editor = CommandEditor::new(template.as_ref(), cx);
        let field = editor.name.clone();
        self.templates.editor = Some(editor);
        self.templates.error = None;
        self.watch_template_editor(cx);
        cx.notify();
        Some(field)
    }

    pub fn close_slash_command(&mut self, cx: &mut Context<Self>) {
        self.templates.editor = None;
        self.template_subscriptions.clear();
        self.refresh_templates();
        cx.notify();
    }

    pub fn save_slash_command(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.templates.editor.as_mut() else {
            return;
        };
        if let Some(issue) = editor.issue(cx) {
            editor.error = Some(issue);
            cx.notify();
            return;
        }
        let text = |field: &Entity<TextField>| field.read(cx).text().to_string();
        let (name, description, hint, body) = (text(&editor.name), text(&editor.description), text(&editor.hint), text(&editor.body));
        let previous = editor.previous.clone();
        match self.store.save_template(previous.as_deref(), &name, &description, &hint, &body) {
            Ok(_) => self.close_slash_command(cx),
            Err(message) => {
                if let Some(editor) = self.templates.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    pub fn ask_delete_slash_command(&mut self, ask: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.templates.editor.as_mut() {
            editor.confirming = ask && editor.previous.is_some();
            cx.notify();
        }
    }

    pub fn confirm_slash_command_delete(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.templates.editor.as_ref().filter(|editor| editor.confirming).and_then(|editor| editor.previous.clone()) else {
            return;
        };
        match self.store.delete_template(&name, &crate::memory_page::move_to_trash) {
            Ok(()) => self.close_slash_command(cx),
            Err(message) => {
                if let Some(editor) = self.templates.editor.as_mut() {
                    editor.confirming = false;
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    fn watch_template_editor(&mut self, cx: &mut Context<Self>) {
        self.template_subscriptions.clear();
        let Some(editor) = self.templates.editor.as_ref() else {
            return;
        };
        for (index, field) in editor.fields().into_iter().enumerate() {
            // The prompt is the one field where ↩ is a new line; ⌘↩ saves from any.
            let prompt = index == 3;
            self.template_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter if prompt => field.update(cx, |field, cx| field.insert_line_break(cx)),
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_slash_command(cx),
                TextFieldEvent::Changed => {
                    if let Some(editor) = app.templates.editor.as_mut() {
                        editor.error = None;
                    }
                    cx.notify();
                }
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Settings › Sub-agents

impl AppState {
    fn refresh_subagents(&mut self) {
        let overview = self.store.agents_overview();
        self.subagents.entries = overview.entries;
        self.subagents.problems = overview.problems;
        self.subagents.folder = self.store.agents_folder();
    }

    fn subagents_result(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.subagents.error = result.err().map(Into::into);
        self.refresh_subagents();
        cx.notify();
    }

    pub fn choose_delegation(&mut self, delegation: wack_core::agents::Delegation, cx: &mut Context<Self>) {
        let result = self.store.set_delegation(delegation);
        self.subagents_result(result, cx);
    }

    pub fn toggle_helper_models(&mut self, cx: &mut Context<Self>) {
        self.subagents.helper_open = !self.subagents.helper_open;
        cx.notify();
    }

    pub fn choose_helper_model(&mut self, connection: Option<String>, model: Option<String>, cx: &mut Context<Self>) {
        let choice = connection.zip(model).map(|(connection, model)| wack_core::ModelChoice { connection: Some(connection), model: Some(model), effort: None });
        let result = self.store.set_helper(choice);
        self.subagents.helper_open = false;
        self.subagents_result(result, cx);
    }

    pub fn switch_agent(&mut self, name: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_agent_enabled(name, on);
        self.subagents_result(result, cx);
    }

    pub fn switch_agent_folder(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        let result = self.store.set_agent_folder(id, on);
        self.subagents_result(result, cx);
    }

    /// The editor on the agent `name`, or a new one. Returns its name field.
    pub fn open_agent(&mut self, name: Option<&str>, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let agent = match name {
            Some(name) => match self.store.read_agent(name) {
                Some(agent) => Some(agent),
                None => {
                    self.subagents_result(Err(format!("{name} could not be read. It may have been moved or deleted.")), cx);
                    return None;
                }
            },
            None => None,
        };
        let over_builtin = name.is_some_and(|name| self.store.builtin_agent(name).is_some());
        let editor = crate::subagents_page::AgentEditor::new(agent.as_ref(), false, over_builtin, cx);
        Some(self.show_agent_editor(editor, cx))
    }

    /// A new agent starting from the one in the editor as it stands.
    pub fn duplicate_agent(&mut self, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        let draft = self.subagents.editor.as_ref()?.draft(cx);
        let agent = wack_core::agents::AgentDef {
            name: draft.name.trim().to_string(),
            description: draft.description.clone(),
            prompt: draft.prompt.clone(),
            tools: draft.tool_list(),
            model: draft.model,
            team: draft.team,
            max_turns: draft.max_turns.trim().parse().unwrap_or(60),
            file: None,
            source: String::new(),
        };
        let editor = crate::subagents_page::AgentEditor::new(Some(&agent), true, false, cx);
        Some(self.show_agent_editor(editor, cx))
    }

    fn show_agent_editor(&mut self, editor: crate::subagents_page::AgentEditor, cx: &mut Context<Self>) -> Entity<TextField> {
        let field = editor.name.clone();
        self.subagents.editor = Some(editor);
        self.subagents.error = None;
        self.watch_agent_editor(cx);
        cx.notify();
        field
    }

    pub fn toggle_team_models(&mut self, cx: &mut Context<Self>) {
        self.subagents.team_open = !self.subagents.team_open;
        cx.notify();
    }

    pub fn choose_team_default(&mut self, connection: Option<String>, model: Option<String>, cx: &mut Context<Self>) {
        let choice = connection.zip(model).map(|(connection, model)| wack_core::ModelChoice { connection: Some(connection), model: Some(model), effort: None });
        let result = self.store.set_team_default(choice);
        self.subagents.team_open = false;
        self.subagents_result(result, cx);
    }

    pub fn choose_agent_team(&mut self, lead: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.subagents.editor.as_mut() {
            editor.team = lead.then_some(wack_core::agents::Tier::Lead);
            editor.error = None;
            cx.notify();
        }
    }

    pub fn choose_agent_model(&mut self, helper: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.subagents.editor.as_mut() {
            editor.model = if helper { wack_core::agents::ModelPref::Helper } else { wack_core::agents::ModelPref::Chat };
            editor.error = None;
            cx.notify();
        }
    }

    pub fn close_agent(&mut self, cx: &mut Context<Self>) {
        self.subagents.editor = None;
        self.subagent_subscriptions.clear();
        self.refresh_subagents();
        cx.notify();
    }

    pub fn save_agent(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.subagents.editor.as_mut() else {
            return;
        };
        if let Some(issue) = editor.issue(cx) {
            editor.error = Some(issue);
            cx.notify();
            return;
        }
        let draft = editor.draft(cx);
        let previous = editor.previous.clone();
        match self.store.save_agent(previous.as_deref(), &draft) {
            Ok(_) => self.close_agent(cx),
            Err(message) => {
                if let Some(editor) = self.subagents.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    pub fn ask_delete_agent(&mut self, ask: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.subagents.editor.as_mut() {
            editor.confirming = ask && editor.yours;
            cx.notify();
        }
    }

    pub fn confirm_agent_delete(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.subagents.editor.as_ref().filter(|editor| editor.confirming).and_then(|editor| editor.previous.clone()) else {
            return;
        };
        match self.store.delete_agent(&name, &crate::memory_page::move_to_trash) {
            Ok(()) => self.close_agent(cx),
            Err(message) => {
                if let Some(editor) = self.subagents.editor.as_mut() {
                    editor.confirming = false;
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    fn watch_agent_editor(&mut self, cx: &mut Context<Self>) {
        self.subagent_subscriptions.clear();
        let Some(editor) = self.subagents.editor.as_ref() else {
            return;
        };
        for (index, field) in editor.fields().into_iter().enumerate() {
            // The description and the instructions take new lines; ⌘↩ saves from any field.
            let lines = index == 1 || index == 4;
            self.subagent_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter if lines => field.update(cx, |field, cx| field.insert_line_break(cx)),
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_agent(cx),
                TextFieldEvent::Changed => {
                    if let Some(editor) = app.subagents.editor.as_mut() {
                        editor.error = None;
                    }
                    cx.notify();
                }
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
            }));
        }
    }

    // -------------------------------------------------------------------------------------------
    // Settings › Prompts

    /// Opens one built-in prompt on its page; the editor already open for it is kept, draft and
    /// all. Returns its text field for focus.
    pub fn open_prompt(&mut self, id: wack_core::PromptId, cx: &mut Context<Self>) -> Option<Entity<TextField>> {
        if let Some(editor) = self.prompts.editor.as_ref()
            && editor.id == id
        {
            return Some(editor.body.clone());
        }
        let editor = crate::prompts_page::PromptEditor::new(id, self.store.prompts(), cx);
        let field = editor.body.clone();
        self.prompts.editor = Some(editor);
        self.prompts.error = None;
        self.prompts.confirming_restore_all = false;
        self.watch_prompt_editor(cx);
        cx.notify();
        Some(field)
    }

    /// Back to the prompts list, forgetting unsaved edits.
    pub fn close_prompt(&mut self, cx: &mut Context<Self>) {
        self.prompts.editor = None;
        self.prompt_subscriptions.clear();
        cx.notify();
    }

    /// Saves the editor's text as the prompt's replacement; the built-in text or an over-long one
    /// never stores.
    pub fn save_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.prompts.editor.as_ref().map(|editor| editor.id) else {
            return;
        };
        let Some(text) = self.prompts.editor.as_ref().map(|editor| editor.body.read(cx).text().to_string()) else {
            return;
        };
        match self.store.set_prompt(id, Some(&text)) {
            Ok(()) => self.close_prompt(cx),
            Err(message) => {
                if let Some(editor) = self.prompts.editor.as_mut() {
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    pub fn ask_restore_prompt(&mut self, ask: bool, cx: &mut Context<Self>) {
        if let Some(editor) = self.prompts.editor.as_mut() {
            editor.confirming_restore = ask;
            cx.notify();
        }
    }

    /// Drops the prompt's replacement: the editor's text becomes WackCode's, shown straight away.
    pub fn confirm_restore_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.prompts.editor.as_ref().filter(|editor| editor.confirming_restore).map(|editor| editor.id) else {
            return;
        };
        match self.store.set_prompt(id, None) {
            Ok(()) => {
                if let Some(editor) = self.prompts.editor.as_mut() {
                    editor.confirming_restore = false;
                    editor.error = None;
                    editor.show_default = false;
                    editor.body.update(cx, |field, cx| field.set_text(id.default_text().as_ref(), cx));
                }
                cx.notify();
            }
            Err(message) => {
                if let Some(editor) = self.prompts.editor.as_mut() {
                    editor.confirming_restore = false;
                    editor.error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    pub fn toggle_prompt_default(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.prompts.editor.as_mut() {
            editor.show_default = !editor.show_default;
            cx.notify();
        }
    }

    pub fn ask_restore_all_prompts(&mut self, ask: bool, cx: &mut Context<Self>) {
        self.prompts.confirming_restore_all = ask;
        cx.notify();
    }

    pub fn confirm_restore_all_prompts(&mut self, cx: &mut Context<Self>) {
        self.prompts.confirming_restore_all = false;
        match self.store.reset_all_prompts() {
            Ok(()) => self.prompts.error = None,
            Err(message) => self.prompts.error = Some(message.into()),
        }
        cx.notify();
    }

    fn watch_prompt_editor(&mut self, cx: &mut Context<Self>) {
        self.prompt_subscriptions.clear();
        let Some(editor) = self.prompts.editor.as_ref() else {
            return;
        };
        let field = editor.body.clone();
        // The prompt takes new lines; ⌘↩ saves from anywhere in it.
        self.prompt_subscriptions.push(cx.subscribe(&field, move |app, field, event: &TextFieldEvent, cx| match event {
            TextFieldEvent::Enter => field.update(cx, |field, cx| field.insert_line_break(cx)),
            TextFieldEvent::SendNow => app.save_prompt(cx),
            TextFieldEvent::Changed => {
                if let Some(editor) = app.prompts.editor.as_mut() {
                    editor.error = None;
                }
                cx.notify();
            }
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
        }));
    }
}

// -------------------------------------------------------------------------------------------------
// The `/` menu

impl AppState {
    /// The composer `owner` names: a chat's, or the new chat's for `None`.
    fn composer_of(&self, owner: Option<&str>, cx: &App) -> Option<Entity<TextField>> {
        match owner {
            None => Some(self.draft.clone()),
            Some(chat) => self.sessions.get(chat).map(|session| session.read(cx).composer.clone()),
        }
    }

    /// The project folder `owner`'s composer works in, when it is a Code composer.
    fn project_folder_for(&self, owner: Option<&str>) -> Option<PathBuf> {
        let project_id = match owner {
            None => match &self.selection {
                Selection::NewChat(project) => Some(project.as_str()),
                _ => None,
            },
            Some(chat) => self.store.chat(chat).and_then(|chat| chat.project.as_deref()),
        };
        project_id.and_then(|id| self.store.project(id)).map(|project| project.path.clone())
    }

    fn close_slash(&mut self, cx: &mut Context<Self>) {
        if let Some(slash) = self.slash.take()
            && let Some(field) = self.composer_of(slash.owner.as_deref(), cx)
        {
            field.update(cx, |field, cx| field.set_completing(false, cx));
        }
    }

    /// Dismisses an open `/` or `@` card for its current word (used when ⎋ reaches the shell
    /// while a loading, error, or empty card has no rows to borrow the field's keys).
    pub fn dismiss_slash(&mut self, cx: &mut Context<Self>) {
        if let Some(slash) = self.slash.take() {
            self.slash_dismissed = Some((slash.owner.clone(), slash.range.start));
            if let Some(field) = self.composer_of(slash.owner.as_deref(), cx) {
                field.update(cx, |field, cx| field.set_completing(false, cx));
            }
            cx.notify();
        }
    }

    fn file_matches(cache: Option<&WorkspaceFilesCache>, query: &str) -> Matches {
        match cache {
            Some(cache) => {
                let (entries, truncated) = match &cache.files {
                    Some(listed) => (listed.rank(query, FILE_MENTION_LIMIT), listed.truncated),
                    None => (Vec::new(), false),
                };
                Matches::Files {
                    entries,
                    loading: cache.loading,
                    truncated,
                    error: if cache.files.is_none() { cache.error.clone() } else { None },
                }
            }
            None => Matches::Files { entries: Vec::new(), loading: false, truncated: false, error: None },
        }
    }

    fn refresh_workspace_files(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        let cache = self.workspace_files.entry(folder.clone()).or_default();
        cache.loading = true;
        cache.error = None;
        cache.request += 1;
        let request = cache.request;
        let lookup = cx.background_spawn({
            let folder = folder.clone();
            async move { files::list(&folder) }
        });
        cx.spawn(async move |app, cx| {
            let result = lookup.await;
            let _ = app.update(cx, |app, cx| {
                let Some(cache) = app.workspace_files.get_mut(&folder) else {
                    return;
                };
                if cache.request != request {
                    return;
                }
                cache.loading = false;
                match result {
                    Ok(listed) => {
                        cache.files = Some(listed);
                        cache.error = None;
                    }
                    Err(error) => {
                        cache.files = None;
                        cache.error = Some(error);
                    }
                }
                if let Some(owner) = app.slash.as_ref().filter(|slash| matches!(slash.matches, Matches::Files { .. })).map(|slash| slash.owner.clone())
                    && app.project_folder_for(owner.as_deref()).as_deref() == Some(folder.as_path())
                {
                    app.update_file_matches(owner, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_file_matches(&mut self, owner: Option<String>, cx: &mut Context<Self>) {
        let Some(folder) = self.project_folder_for(owner.as_deref()) else {
            return;
        };
        let Some(query) = self
            .slash
            .as_ref()
            .filter(|slash| slash.owner == owner && matches!(slash.matches, Matches::Files { .. }))
            .map(|slash| slash.query.clone())
        else {
            return;
        };
        let matches = Self::file_matches(self.workspace_files.get(&folder), &query);
        let completing = !matches.is_empty();
        if let Some(slash) = self.slash.as_mut() {
            slash.selected = slash.selected.min(matches.len().saturating_sub(1));
            slash.matches = matches;
        }
        if let Some(field) = self.composer_of(owner.as_deref(), cx) {
            field.update(cx, |field, cx| field.set_completing(completing, cx));
        }
    }

    /// Retries loading the workspace file list for an open `@` file picker whose scan failed.
    pub fn retry_workspace_files(&mut self, cx: &mut Context<Self>) {
        let Some(owner) = self.slash.as_ref().filter(|slash| matches!(slash.matches, Matches::Files { .. })).map(|slash| slash.owner.clone()) else {
            return;
        };
        let Some(folder) = self.project_folder_for(owner.as_deref()) else {
            return;
        };
        self.refresh_workspace_files(folder, cx);
        self.update_file_matches(owner, cx);
        cx.notify();
    }

    /// The composer's text or caret changed: open, follow or close the `/` or `@` menu, and light
    /// a typed `/command` that exists.
    pub fn composer_changed(&mut self, owner: Option<String>, cx: &mut Context<Self>) {
        let Some(field) = self.composer_of(owner.as_deref(), cx) else {
            return;
        };
        let (text, caret) = {
            let field = field.read(cx);
            (field.text().to_string(), field.caret())
        };
        if self.is_chat_area_owner(owner.as_deref()) {
            self.mention_changed(owner, &field, &text, caret, cx);
            return;
        }
        enum Trigger {
            Slash(std::ops::Range<usize>),
            File(std::ops::Range<usize>, String),
        }
        let mention = files::active_mention(&text, caret);
        let quoted_mention = mention.as_ref().is_some_and(|(range, _)| text[range.start..].starts_with("@\""));
        let trigger = if quoted_mention {
            mention.map(|(range, query)| Trigger::File(range, query))
        } else if let Some(range) = active_slash(&text, caret) {
            Some(Trigger::Slash(range))
        } else {
            mention.map(|(range, query)| Trigger::File(range, query))
        };
        let start = trigger.as_ref().map(|trigger| match trigger {
            Trigger::Slash(range) | Trigger::File(range, _) => range.start,
        });
        // ⎋ keeps the menu shut for its word only; another word opens it again.
        if self.slash_dismissed.as_ref().is_some_and(|(from, at)| *from != owner || start != Some(*at)) {
            self.slash_dismissed = None;
        }
        let trigger = trigger.filter(|_| self.slash_dismissed.as_ref() != Some(&(owner.clone(), start.unwrap())));
        let previous = self.slash.take().filter(|slash| slash.owner == owner);
        let other = previous.is_none() && self.slash.is_some();
        match trigger {
            Some(Trigger::Slash(range)) => {
                let fresh = previous.as_ref().is_none_or(|slash| !matches!(slash.matches, Matches::Commands(_)) || slash.range.start != range.start);
                if fresh || self.commands.is_empty() {
                    self.commands = self.store.command_catalog();
                }
                let query = text.get(range.start + 1..caret.max(range.start + 1)).unwrap_or("").to_string();
                let matches = Matches::Commands(matching(&self.commands, &query).into_iter().cloned().collect());
                let (selected, scroll) = match previous {
                    Some(slash) if !fresh && slash.query == query => (slash.selected.min(matches.len().saturating_sub(1)), slash.scroll),
                    Some(slash) => (0, slash.scroll),
                    None => (0, gpui::ScrollHandle::new()),
                };
                if selected == 0 {
                    scroll.scroll_to_item(0);
                }
                let completing = !matches.is_empty();
                self.slash = Some(Slash { owner: owner.clone(), range, query, matches, selected, scroll });
                field.update(cx, |field, cx| field.set_completing(completing, cx));
            }
            Some(Trigger::File(range, query)) => {
                if let Some(folder) = self.project_folder_for(owner.as_deref()) {
                    let fresh = previous.as_ref().is_none_or(|slash| !matches!(slash.matches, Matches::Files { .. }) || slash.range.start != range.start);
                    if fresh {
                        self.refresh_workspace_files(folder.clone(), cx);
                    }
                    let matches = Self::file_matches(self.workspace_files.get(&folder), &query);
                    let (selected, scroll) = match previous {
                        Some(slash) if !fresh && slash.query == query => (slash.selected.min(matches.len().saturating_sub(1)), slash.scroll),
                        Some(slash) => (0, slash.scroll),
                        None => (0, gpui::ScrollHandle::new()),
                    };
                    if selected == 0 {
                        scroll.scroll_to_item(0);
                    }
                    let completing = !matches.is_empty();
                    self.slash = Some(Slash { owner: owner.clone(), range, query, matches, selected, scroll });
                    field.update(cx, |field, cx| field.set_completing(completing, cx));
                } else if !other {
                    field.update(cx, |field, cx| field.set_completing(false, cx));
                }
            }
            None if !other => field.update(cx, |field, cx| field.set_completing(false, cx)),
            None => {}
        }
        if text.starts_with('/') && self.commands.is_empty() {
            self.commands = self.store.command_catalog();
        }
        let lit = parse_line(&text).filter(|(name, _)| find(&self.commands, name).is_some()).map(|(name, _)| 0..1 + name.len());
        field.update(cx, |field, cx| field.set_highlight(lit, cx));
        cx.notify();
    }

    /// A key the menu borrowed from `owner`'s composer.
    pub fn slash_key(&mut self, owner: Option<String>, key: MenuKey, cx: &mut Context<Self>) {
        let Some(slash) = self.slash.as_mut().filter(|slash| slash.owner == owner) else {
            return;
        };
        let count = slash.matches.len();
        match key {
            MenuKey::Up | MenuKey::Down if count > 0 => {
                slash.selected = match key {
                    MenuKey::Up => (slash.selected + count - 1) % count,
                    _ => (slash.selected + 1) % count,
                };
                slash.scroll.scroll_to_item(slash.selected);
                cx.notify();
            }
            MenuKey::Pick => {
                let index = slash.selected;
                self.pick_command(index, cx);
            }
            MenuKey::Dismiss => {
                self.slash_dismissed = Some((owner.clone(), slash.range.start));
                self.slash = None;
                if let Some(field) = self.composer_of(owner.as_deref(), cx) {
                    field.update(cx, |field, cx| field.set_completing(false, cx));
                }
                cx.notify();
            }
            _ => {}
        }
    }

    /// Puts the menu's `index`th command at the front of its composer's message, the text that
    /// was around the `/word` becoming its arguments; or, in the `@` picker, writes the chat's
    /// `@"Name"` in place of the `@word` and keeps its id for the message.
    pub fn pick_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(slash) = self.slash.take() else {
            return;
        };
        let Some(field) = self.composer_of(slash.owner.as_deref(), cx) else {
            return;
        };
        let command = match &slash.matches {
            Matches::Commands(commands) => commands.get(index),
            Matches::Files { entries, .. } => {
                let Some(entry) = entries.get(index).cloned() else { return };
                let text = field.read(cx).text().to_string();
                let Some(range) = Some(slash.range.clone()).filter(|range| range.end <= text.len()) else { return };
                let (next, caret) = files::pick(&text, range, &entry);
                if entry.directory {
                    // Keep the picker attached to this `@` word so `composer_changed` lists the
                    // folder's children immediately without restarting the workspace scan.
                    self.slash = Some(slash);
                }
                field.update(cx, |field, cx| {
                    if !entry.directory {
                        field.set_completing(false, cx);
                    }
                    field.replace_text(&next, caret, cx);
                });
                cx.notify();
                return;
            }
            Matches::Chats(chats) => {
                let Some((chat, _)) = chats.get(index) else { return };
                let text = field.read(cx).text().to_string();
                let Some(range) = Some(slash.range.clone()).filter(|range| range.end <= text.len()) else { return };
                let token = format!("{} ", chat_refs::mention(&chat.name));
                let caret = range.start + token.len();
                let after = text[range.end..].strip_prefix(' ').unwrap_or(&text[range.end..]);
                let text = format!("{}{token}{after}", &text[..range.start]);
                let refs = self.mention_refs.entry(slash.owner.clone()).or_default();
                if !refs.iter().any(|kept| kept.id == chat.id) {
                    refs.push(chat.clone());
                }
                field.update(cx, |field, cx| {
                    field.set_completing(false, cx);
                    field.replace_text(&text, caret, cx);
                });
                cx.notify();
                return;
            }
        };
        let Some(command) = command else {
            return;
        };
        let text = field.read(cx).text().to_string();
        if slash.range.end > text.len() {
            return;
        }
        let (text, caret) = pick(&text, slash.range.clone(), &command.name);
        field.update(cx, |field, cx| {
            field.set_completing(false, cx);
            field.replace_text(&text, caret, cx);
        });
        cx.notify();
    }

    /// Whether `owner`'s composer is in the Chat area: the new chat's while the Chat area's new chat
    /// shows, or a Chat chat's.
    fn is_chat_area_owner(&self, owner: Option<&str>) -> bool {
        match owner {
            None => self.selection == Selection::NewChatArea,
            Some(chat) => self.store.chat(chat).is_some_and(Chat::is_chat_area),
        }
    }

    /// The `@` picker's half of [`Self::composer_changed`], for a Chat-area composer: it lists
    /// the area's other chats for an `@word`, and lights a typed `/new`, `/name`, `/copy` or
    /// `/compact` (the only commands there).
    fn mention_changed(&mut self, owner: Option<String>, field: &Entity<TextField>, text: &str, caret: usize, cx: &mut Context<Self>) {
        let word = chat_refs::active_mention(text, caret);
        if self.slash_dismissed.as_ref().is_some_and(|(from, start)| *from != owner || word.as_ref().is_none_or(|(range, _)| range.start != *start)) {
            self.slash_dismissed = None;
        }
        let word = word.filter(|(range, _)| self.slash_dismissed.as_ref() != Some(&(owner.clone(), range.start)));
        let previous = self.slash.take().filter(|slash| slash.owner == owner);
        let other = previous.is_none() && self.slash.is_some();
        match word {
            Some((range, query)) => {
                let chats: Vec<ChatRef> = self
                    .store
                    .chat_area_chats()
                    .into_iter()
                    .filter(|chat| Some(&chat.id) != owner.as_ref())
                    .map(|chat| ChatRef { id: chat.id.clone(), name: chat.title.clone() })
                    .collect();
                let ages: HashMap<String, u64> = self.store.chat_area_chats().into_iter().map(|chat| (chat.id.clone(), chat.updated)).collect();
                let matches: Vec<(ChatRef, String)> = chat_refs::rank(&chats, &query, MENTION_LIMIT)
                    .into_iter()
                    .map(|chat| {
                        let age = ages.get(&chat.id).map(|updated| Self::age(*updated)).unwrap_or_default();
                        (chat, age)
                    })
                    .collect();
                let fresh = previous.as_ref().is_none_or(|slash| slash.range.start != range.start);
                let (selected, scroll) = match previous {
                    Some(slash) if !fresh && slash.query == query => (slash.selected.min(matches.len().saturating_sub(1)), slash.scroll),
                    Some(slash) => (0, slash.scroll),
                    None => (0, gpui::ScrollHandle::new()),
                };
                if selected == 0 {
                    scroll.scroll_to_item(0);
                }
                let completing = !matches.is_empty();
                self.slash = Some(Slash { owner: owner.clone(), range, query, matches: Matches::Chats(matches), selected, scroll });
                field.update(cx, |field, cx| field.set_completing(completing, cx));
            }
            None if !other => field.update(cx, |field, cx| field.set_completing(false, cx)),
            None => {}
        }
        let lit = parse_line(text).filter(|(name, _)| CHAT_AREA_COMMANDS.contains(name)).map(|(name, _)| 0..1 + name.len());
        field.update(cx, |field, cx| field.set_highlight(lit, cx));
        cx.notify();
    }

    /// `outgoing` with the chats picked in `owner`'s composer, when it is words: their section
    /// rides at the end of the message, for the ones whose `@"Name"` is still there.
    fn with_refs(&self, owner: Option<&str>, outgoing: Outgoing) -> Outgoing {
        match outgoing {
            Outgoing::Message(text) => {
                let refs = self.mention_refs.get(&owner.map(str::to_string)).map(|refs| chat_refs::surviving(&text, refs)).unwrap_or_default();
                Outgoing::Message(chat_refs::encode(&text, &refs))
            }
            other => other,
        }
    }

    /// Remembers `refs` for `chat`'s composer again, after its unsent words came back to it.
    fn keep_refs(&mut self, chat: &str, refs: Vec<ChatRef>) {
        if refs.is_empty() {
            return;
        }
        let kept = self.mention_refs.entry(Some(chat.to_string())).or_default();
        for chat in refs {
            if !kept.iter().any(|seen| seen.id == chat.id) {
                kept.push(chat);
            }
        }
    }

    /// The menu for `owner`'s composer, when it is open there.
    pub fn slash_data(&self, owner: Option<&str>) -> Option<SlashData> {
        self.slash.as_ref().filter(|slash| slash.owner.as_deref() == owner).map(Slash::data)
    }

    /// Under a bare `/command` in `owner`'s composer: what it takes after it.
    pub fn command_hint(&self, owner: Option<&str>, cx: &App) -> Option<SharedString> {
        if self.slash_data(owner).is_some() {
            return None;
        }
        let field = self.composer_of(owner, cx)?;
        let text = field.read(cx).text();
        let (name, args) = parse_line(text.trim_start())?;
        if !args.is_empty() {
            return None;
        }
        crate::slash_menu::hint_note(find(&self.commands, name)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_to_review_holds_the_queue() {
        let words = |list: &[&str]| list.iter().map(|text| text.to_string()).collect::<Vec<_>>();
        let sent = after_run(true, false, words(&["steer"]), words(&["a", "b"]));
        assert_eq!(sent, AfterRun { next: Some("steer".into()), queue: words(&["a", "b"]), unsent: Vec::new() });
        let held = after_run(true, true, Vec::new(), words(&["a", "b"]));
        assert_eq!(held, AfterRun { next: None, queue: Vec::new(), unsent: words(&["a", "b"]) });
        let stopped = after_run(false, false, Vec::new(), words(&["a"]));
        assert_eq!(stopped.unsent, words(&["a"]));
    }

    #[test]
    fn paths_in_the_home_folder_start_with_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(shown_path(&home.join("Documents/x")), "~/Documents/x");
        assert_eq!(shown_path(&home), "~");
        assert_eq!(shown_path(Path::new("/opt/elsewhere")), "/opt/elsewhere");
    }
}
