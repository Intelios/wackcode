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
    AgentMode, Appearance, Chat, CommandEvent, CommandKind, CommandStatus, Job, ModelChoice, Outcome,
    AutoTitleConfig, Project, Reasoning, Runner, Store, Ui, WireFormat,
    skills::{Command, CommandUse, Outgoing, active_slash, find, matching, parse_line, pick, resolve_message},
    title, title_from,
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
    slash_menu::{Slash, SlashData},
    text_field::{MenuKey, TextField, TextFieldEvent},
    theme,
    typography::Type,
};

pub const NEW_CHAT_PLACEHOLDER: &str = "Describe a task, or ask about the code";
pub const PLAN_PLACEHOLDER: &str = "Describe the work — in Plan mode WackCode inspects and proposes a plan for your approval…";
pub const ULTRA_PLAN_PLACEHOLDER: &str = "Describe the work — in Ultra Plan WackCode interviews you one question at a time…";

/// What an empty composer says: the planning modes explain themselves.
pub fn placeholder(mode: AgentMode, new_chat: bool) -> &'static str {
    match mode {
        AgentMode::Plan => PLAN_PLACEHOLDER,
        AgentMode::UltraPlan => ULTRA_PLAN_PLACEHOLDER,
        AgentMode::Build if new_chat => NEW_CHAT_PLACEHOLDER,
        AgentMode::Build => crate::chat::FOLLOW_UP_PLACEHOLDER,
    }
}

/// A menu open over the composer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Model,
    Effort,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// No project yet.
    Welcome,
    /// Writing the first message of a chat in this project.
    NewChat(String),
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
    /// Each open chat's shell tasks, so quitting can stop them without a window context.
    chat_tasks: HashMap<String, wack_core::tasks::Tasks>,
    /// Each open chat's sub-agents, likewise.
    chat_agents: HashMap<String, wack_core::agents::Agents>,
    session_subscriptions: HashMap<String, Vec<Subscription>>,
    /// Projects whose chats the sidebar lists.
    open_projects: HashSet<String>,
    /// Each project's checked-out branch, looked up off the UI thread.
    branches: HashMap<String, Option<String>>,
    /// The new-chat composer, shared by every project.
    pub draft: Entity<TextField>,
    /// The mode the new chat starts in.
    pub new_chat_mode: AgentMode,
    /// Why the last chat could not start, shown under the new-chat composer.
    pub notice: Option<SharedString>,
    /// Settings › Providers' connection editor.
    pub form: ProviderForm,
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
    /// Long-lived Build and Run PTY commands keyed by `(canonical_cwd, kind)`.
    runs: Runner,
    command_event_tx: mpsc::UnboundedSender<CommandEvent>,
    /// Draft panes (for example Build / Run output) open on `Screen::NewChat` before a chat starts.
    new_chat_panes: HashMap<String, ChatPanes>,
    /// The open Build/Run command editor popover, if any.
    pub command_editor: Option<CommandEditorState>,
    /// Chats open as tabs, in order.
    tabs: Vec<String>,
    sidebar_hidden: bool,
    /// Chats whose pane layout changed since it was last saved.
    unsaved_layouts: HashSet<String>,
    saving_layouts: Option<Task<()>>,
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
    /// The chat being renamed in place, if any.
    pub renaming: Option<Renaming>,
    /// The `/` menu, open over one composer.
    pub slash: Option<Slash>,
    /// Where ⎋ closed the menu (whose composer, the word's start): it stays shut for that word.
    slash_dismissed: Option<(Option<String>, usize)>,
    /// The commands as last read, for the menu and for lighting a typed `/command`. Read again
    /// each time the menu opens.
    commands: Vec<Command>,
    /// A message refused as an unknown command (whose composer, its text): ↩ again with the text
    /// unchanged sends it as written.
    literal: Option<(Option<String>, String)>,
    _subscriptions: Vec<Subscription>,
}

/// A chat's title being edited where it is shown: on its tab or its sidebar row.
pub struct Renaming {
    pub chat: String,
    /// Where the field is drawn: the sidebar row or the tab.
    pub place: RenamePlace,
    pub field: Entity<TextField>,
    /// What had focus before, to give it back.
    back: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenamePlace {
    Tab,
    Sidebar,
}

impl Drop for AppState {
    fn drop(&mut self) {
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

    pub fn new(store: Store, cx: &mut Context<Self>) -> Self {
        let draft = cx.new(|cx| TextField::new(Type::Body, NEW_CHAT_PLACEHOLDER, cx).multiline(14));
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
        });
        let form = ProviderForm::new(cx);
        let mcp = wack_core::mcp_client::Hub::new(Some(store.mcp_cache_folder()));
        let subscriptions = vec![draft_subscription];
        let ui = store.ui().clone();
        // A fresh app: no tabs, no chat in front, and a new chat in the project last worked in (as
        // version 2 opened), rather than wherever the last one was left.
        let selection = match store.projects() {
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
            while let Some(first) = command_event_rx.next().await {
                let mut batch = vec![first];
                while let Ok(next) = command_event_rx.try_recv() {
                    batch.push(next);
                }
                if app.update(cx, |app, cx| app.handle_command_events(batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut app = Self {
            store,
            before_settings: selection.clone(),
            selection: Selection::Welcome,
            sessions: HashMap::new(),
            chat_tasks: HashMap::new(),
            chat_agents: HashMap::new(),
            session_subscriptions: HashMap::new(),
            open_projects: HashSet::new(),
            branches: HashMap::new(),
            draft,
            slash: None,
            slash_dismissed: None,
            commands: Vec::new(),
            literal: None,
            new_chat_mode: AgentMode::Build,
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
            runs,
            command_event_tx,
            new_chat_panes: HashMap::new(),
            command_editor: None,
            appearance_notice: None,
            importing: false,
            tabs,
            sidebar_hidden: ui.sidebar_hidden,
            unsaved_layouts: HashSet::new(),
            saving_layouts: None,
            form_subscriptions: Vec::new(),
            titling: HashMap::new(),
            title_attempts: 0,
            title_swaps: HashMap::new(),
            title_models_open: false,
            title_error: None,
            renaming: None,
            _subscriptions: subscriptions,
        };
        app.watch_form(cx);
        theme::apply(app.store.appearance(), cx);
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
            }));
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Settings › MCP servers

    /// Closes connections to servers that were removed, switched off or changed.
    fn mcp_changed(&mut self) {
        self.mcp.retain(&self.store.mcp_live_keys());
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
                ServerRow {
                    server: server.clone(),
                    live,
                    reach: crate::mcp_page::reach(&server.projects, &names),
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

    pub fn choose_mcp_scope(&mut self, only: bool, cx: &mut Context<Self>) {
        use wack_core::mcp::Projects;
        self.mcp_editor(
            |editor| {
                editor.projects = match (only, &editor.projects) {
                    (false, _) => Projects::All,
                    (true, Projects::Only(ids)) => Projects::Only(ids.clone()),
                    (true, Projects::All) => Projects::Only(Vec::new()),
                }
            },
            cx,
        );
    }

    pub fn toggle_mcp_project(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mcp_editor(
            |editor| {
                if let wack_core::mcp::Projects::Only(ids) = &mut editor.projects {
                    match ids.iter().position(|chosen| chosen == id) {
                        Some(index) => {
                            ids.remove(index);
                        }
                        None => ids.push(id.to_string()),
                    }
                }
            },
            cx,
        );
    }

    /// Saves the editor's server, closes the editor and tests the connection in the background,
    /// which lists its tools on its row.
    pub fn save_mcp_server(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.mcp_settings.editor.as_ref() else {
            return;
        };
        let result = editor.draft(cx).and_then(|draft| self.store.save_mcp_server(&draft));
        match result {
            Ok(id) => {
                self.close_mcp_editor(cx);
                self.mcp_changed();
                self.mcp_settings.expanded = Some(id.clone());
                self.test_mcp_server(&id, cx);
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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

    /// The backdrop to draw, if there is one.
    pub fn backdrop(&self) -> Option<Picture> {
        self.store.appearance().backdrop().map(|backdrop| self.picture(backdrop))
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
        self.tabs.remove(index);
        if self.selection == Selection::Chat(id.to_string()) {
            // The neighbour on the right takes its place, as in a browser; else the one before.
            match self.tabs.get(index).or_else(|| index.checked_sub(1).and_then(|before| self.tabs.get(before))).cloned() {
                Some(next) => self.select(Selection::Chat(next), cx),
                None => {
                    let project = self.store.chat(id).map(|chat| chat.project.clone());
                    match project {
                        Some(project) => self.select(Selection::NewChat(project), cx),
                        None => self.select_somewhere(cx),
                    }
                }
            }
        }
        self.save_ui();
        cx.notify();
    }

    /// Steps through the tabs, wrapping round; from the new-chat screen, to the last tab.
    pub fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let count = self.tabs.len();
        let next = match &self.selection {
            Selection::Chat(id) => match self.tabs.iter().position(|tab| tab == id) {
                Some(index) if forward => (index + 1) % count,
                Some(index) => (index + count - 1) % count,
                None => 0,
            },
            _ => count - 1,
        };
        let id = self.tabs[next].clone();
        self.select(Selection::Chat(id), cx);
    }

    fn save_ui(&mut self) {
        // Settings has no project of its own, so the one last worked in stays.
        let last_project = self
            .current_project()
            .map(|project| project.id.clone())
            .or_else(|| self.store.ui().last_project.clone().filter(|id| self.store.project(id).is_some()));
        let ui = Ui { sidebar_hidden: self.sidebar_hidden, last_project };
        if let Err(message) = self.store.set_ui(ui) {
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
            Selection::Chat(chat) => self.store.chat(chat).map(|chat| chat.project.as_str()),
            Selection::Welcome | Selection::Settings => None,
        };
        id.and_then(|id| self.store.project(id))
    }

    /// The text field that should have focus on the current screen.
    pub fn active_field(&self, cx: &App) -> Option<Entity<TextField>> {
        if let Some(editor) = &self.command_editor {
            return Some(editor.input.clone());
        }
        match &self.selection {
            Selection::NewChat(_) => Some(self.draft.clone()),
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
                self.open_projects.insert(chat.project.clone());
                self.look_up_branch(&chat.project, cx);
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
            }
            Selection::Settings => {
                if self.selection != Selection::Settings {
                    self.before_settings = self.selection.clone();
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
            }
            Selection::Welcome => {}
        }
        self.notice = None;
        self.menu = None;
        self.command_editor = None;
        self.selection = selection;
        self.save_ui();
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.selection == Selection::Settings {
            self.form.put_key_away(cx);
            let back = self.before_settings.clone();
            self.selection = Selection::Welcome;
            self.select(back, cx);
            if self.selection == Selection::Welcome {
                self.select_somewhere(cx);
            }
        }
    }

    /// A new chat in `project`, or in the current project when `None`.
    pub fn new_chat(&mut self, project: Option<String>, cx: &mut Context<Self>) {
        let project = project
            .or_else(|| self.current_project().map(|project| project.id.clone()))
            .or_else(|| self.store.projects().first().map(|project| project.id.clone()));
        match project {
            Some(project) => self.select(Selection::NewChat(project), cx),
            None => self.select(Selection::Welcome, cx),
        }
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
        self.new_chat_panes.remove(id);
        if self.command_editor.as_ref().is_some_and(|editor| editor.project_id == id) {
            self.command_editor = None;
        }
        if let Some(path) = project_path {
            self.runs.stop_project(&path);
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
        let project = self.store.chat(id).map(|chat| chat.project.clone());
        self.drop_session(id);
        if let Err(message) = self.store.delete_chat(id) {
            self.notice = Some(message.into());
        }
        // A chat a suggestion card linked to is gone: the card is actionable again.
        if let Some(project) = &project {
            let project = project.clone();
            for (chat_id, session) in self.sessions.clone() {
                if self.store.chat(&chat_id).is_some_and(|chat| chat.project == project) {
                    let chat = self.store.chat(&chat_id).unwrap().clone();
                    let view = self.suggestion_view(&chat);
                    session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
                }
            }
        }
        if self.selection == Selection::Chat(id.to_string()) {
            match project {
                Some(project) => self.select(Selection::NewChat(project), cx),
                None => self.select_somewhere(cx),
            }
        }
        self.save_ui();
        cx.notify();
    }

    /// After the selected thing went away: a new chat in the first project, or the welcome.
    fn select_somewhere(&mut self, cx: &mut Context<Self>) {
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
        let Selection::NewChat(project) = self.selection.clone() else {
            return;
        };
        let text = self.draft.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let cwd = self.store.project(&project).map(|project| project.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        let (job, opening) = match self.outgoing(None, &text, &cwd) {
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
            Ok(Outgoing::Message(text)) => (Job::Prompt(text.clone()), text),
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
        let choice: ModelChoice = self.store.defaults().clone();
        if self.store.resolve(&choice).is_none() {
            self.notice = Some("Add a connection in Settings › Providers to start chatting.".into());
            cx.notify();
            return;
        }
        let title = match &job {
            Job::Command { command, .. } => title_from(&command.line()),
            _ => title_from(&text),
        };
        let chat = match self.store.create_chat(&project, &title, choice, self.new_chat_mode, None) {
            Ok(chat) => chat,
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
        };
        // Preserve any Build/Run output panes opened on NewChat into the new chat's layout so
        // live command output stays visible seamlessly after the first message sends.
        if let Some(draft_panes) = self.new_chat_panes.remove(&project)
            && !draft_panes.layout.panes().is_empty()
            && let Some(value) = serde_json::to_value(&draft_panes.layout).ok()
        {
            let _ = self.store.set_layout(&chat.id, Some(value));
        }
        // The next new chat starts in Build again: a chat's mode is its own.
        self.new_chat_mode = AgentMode::Build;
        self.draft.update(cx, |draft, cx| {
            draft.set_text("", cx);
            draft.set_placeholder(placeholder(AgentMode::Build, true), cx);
        });
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
        let request = title::request(&model, opening);
        // The ledger records the request against the chat's project, as a run's calls are.
        let recorder = self
            .store
            .chat(chat)
            .and_then(|listed| self.store.project(&listed.project))
            .map(|project| self.store.usage_recorder(&connection, chat, &project.path));
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

    /// A word to the user in a chat's transcript.
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
            let mut field = TextField::new(Type::Ui, "Chat name", cx);
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

    /// The mode of the composer on screen, and whether it is locked by a run.
    pub fn mode_on_screen(&self, cx: &App) -> (AgentMode, bool) {
        match &self.selection {
            Selection::Chat(id) => {
                let mode = self.store.chat(id).map_or(AgentMode::Build, |chat| chat.mode);
                let locked = self.sessions.get(id).is_some_and(|session| session.read(cx).running());
                (mode, locked)
            }
            _ => (self.new_chat_mode, false),
        }
    }

    /// Switches the composer on screen to `mode`. A chat that is working keeps its mode.
    pub fn set_agent_mode(&mut self, mode: AgentMode, cx: &mut Context<Self>) {
        match self.selection.clone() {
            Selection::NewChat(_) => {
                self.new_chat_mode = mode;
                self.draft.update(cx, |draft, cx| draft.set_placeholder(placeholder(mode, true), cx));
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
        if let Some(session) = self.sessions.get(id) {
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
        let Some(project) = self.store.project(&chat.project).map(|project| project.path.clone()) else {
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
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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
        if matches!(self.selection, Selection::Chat(_) | Selection::NewChat(_)) {
            let next = crate::composer::next_mode(self.mode_on_screen(cx).0);
            self.set_agent_mode(next, cx);
        }
    }

    /// ↩ in a chat's composer: send now, or queue behind the run. With `steer` (⌘↩) a message
    /// written during a run goes to the agent at its next step instead. A `/command` is resolved
    /// here: WackCode's own act at once, and one for the model runs (or waits in the queue, as
    /// typed, to be resolved when it is sent; a command never steers).
    pub fn submit(&mut self, chat: &str, steer: bool, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let composer = session.read(cx).composer.clone();
        let text = composer.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let cwd = session.read(cx).cwd().to_path_buf();
        let outgoing = match self.outgoing(Some(chat), &text, &cwd) {
            Ok(outgoing) => outgoing,
            Err(message) => {
                session.update(cx, |session, cx| session.note(message, cx));
                return;
            }
        };
        let (running, compacting) = {
            let session = session.read(cx);
            (session.running(), session.compacting())
        };
        let clear = |cx: &mut Context<Self>| composer.update(cx, |composer, cx| composer.set_text("", cx));
        match outgoing {
            Outgoing::Builtin { name, args } => {
                if self.builtin(chat, name, &args, cx) {
                    clear(cx);
                }
            }
            outgoing if running => {
                session.update(cx, |session, cx| {
                    let plain = matches!(outgoing, Outgoing::Message(_));
                    if steer && !compacting && plain {
                        session.steer(text, cx);
                    } else {
                        session.queue.push(text);
                        session.sync_suggestion_queue(cx);
                    }
                });
                clear(cx);
            }
            Outgoing::Message(text) => {
                if self.run(chat, Job::Prompt(text), cx) {
                    clear(cx);
                }
            }
            Outgoing::Prompt { text, command } => {
                if self.run(chat, Job::Command { text, command }, cx) {
                    clear(cx);
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
                let project = self.store.chat(chat).map(|chat| chat.project.clone());
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
        let idle = self.sessions.get(chat).is_some_and(|session| {
            let session = session.read(cx);
            !session.running() && !session.is_empty()
        });
        if idle {
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
            restore(&session, vec![text], cx);
        }
    }

    /// What the cards of `chat`'s settled turns read their state from: dismissals and opened
    /// chats from the store; the chat itself fills in started and queued ids.
    fn suggestion_view(&self, chat: &Chat) -> crate::suggestions::View {
        let chats = self.store.chats(&chat.project);
        crate::suggestions::View {
            dismissed: chat.dismissed_suggestions.iter().cloned().collect(),
            queued: Default::default(),
            links: crate::suggestions::links(&chats, &chat.id),
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
    /// it. It opens in a background tab: the user stays in the chat they were reading.
    fn start_suggestion_chat(&mut self, origin: &Chat, card: &crate::suggestions::Card, mode: AgentMode, cx: &mut Context<Self>) {
        let origin_of = wack_core::SuggestionOrigin { chat: origin.id.clone(), suggestion: card.id.to_string() };
        let chat = match self
            .store
            .create_chat(&origin.project, &card.title, origin.choice.clone(), mode, Some(origin_of))
        {
            Ok(chat) => chat,
            Err(message) => {
                self.note(&origin.id, &message, cx);
                return;
            }
        };
        self.open_projects.insert(origin.project.clone());
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
        let session = self.ensure_session(&chat, cx);
        // A message from the user lets task news wake the chat again.
        if matches!(job, Job::Prompt(_) | Job::Command { .. }) {
            session.update(cx, |session, _| {
                session.wakes = 0;
                session.wake_paused = false;
            });
        }
        match self.store.run_setup(&chat) {
            Ok(mut setup) => {
                setup.mcp_hub = Some(self.mcp.clone());
                session.update(cx, |session, cx| session.start(setup, job, cx));
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
        if let Some(project) = self.store.chat(chat).map(|chat| chat.project.clone()) {
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
            restore(&session, unsent, cx);
        }
        // The user stopped it, or it failed: no waking until they write again.
        if !matches!(result, Ok(Outcome::Done | Outcome::CutOff | Outcome::Refused)) {
            session.update(cx, |session, _| session.wake_paused = true);
        }
        match next {
            Some(next) => {
                if !self.send_queued(chat, &next, cx) {
                    restore(&session, vec![next], cx);
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
        // A queued suggested task's title may start with `/`: it was recorded literal when it
        // queued, so it sends as written rather than trying to resolve a command.
        if self.literal.take().is_some_and(|(from, literal)| from.as_deref() == Some(chat) && literal == text) {
            return self.run(chat, Job::Prompt(text.to_string()), cx);
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
            None => self.run(chat, Job::Prompt(text.to_string()), cx),
            Some(Ok(Outgoing::Message(text))) => self.run(chat, Job::Prompt(text), cx),
            Some(Ok(Outgoing::Prompt { text, command })) => self.run(chat, Job::Command { text, command }, cx),
            Some(Ok(Outgoing::Builtin { name, args })) => self.builtin(chat, name, &args, cx) || true,
            Some(Err(message)) => refused(self, message, cx),
        }
    }

    fn ensure_session(&mut self, chat: &Chat, cx: &mut Context<Self>) -> Entity<ChatSession> {
        if let Some(session) = self.sessions.get(&chat.id) {
            return session.clone();
        }
        let project = self.store.project(&chat.project);
        let cwd = project.map(|project| project.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        let commands = (
            project.and_then(|project| project.command(CommandKind::Build)).map(str::to_string),
            project.and_then(|project| project.command(CommandKind::Run)).map(str::to_string),
        );
        let path = self.store.session_path(chat);
        let layout = Layout::from_saved(chat.layout.as_ref());
        let runner = self.runs.clone();
        let tasks_folder = self.store.tasks_folder(chat);
        let session = cx.new(|cx| ChatSession::load(&chat.id, cwd, &path, layout, runner, commands, tasks_folder, cx));
        let id = chat.id.clone();
        // Cards read their dismissals and opened links from the store as it stands now.
        let view = self.suggestion_view(chat);
        session.update(cx, |session, cx| session.sync_suggestion_view(view, cx));
        self.chat_tasks.insert(id.clone(), session.read(cx).tasks.clone());
        self.chat_agents.insert(id.clone(), session.read(cx).agents.clone());
        let composer = session.read(cx).composer.clone();
        let mode = chat.mode;
        composer.update(cx, |composer, cx| composer.set_placeholder(placeholder(mode, false), cx));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, {
                let id = id.clone();
                move |app, _, event: &ChatEvent, cx| match event {
                    ChatEvent::Finished(result) => app.finished(&id, result, cx),
                    ChatEvent::LayoutChanged => app.layout_changed(&id, cx),
                    ChatEvent::ModeChanged(mode) => app.set_chat_mode(&id, *mode, cx),
                    ChatEvent::TasksNews | ChatEvent::AgentsNews => app.wake(&id, cx),
                }
            }),
            cx.subscribe(&composer, {
                let id = id.clone();
                move |app, _, event: &TextFieldEvent, cx| match event {
                    TextFieldEvent::Enter => app.submit(&id, false, cx),
                    TextFieldEvent::SendNow => app.submit(&id, true, cx),
                    TextFieldEvent::Changed | TextFieldEvent::Moved => app.composer_changed(Some(id.clone()), cx),
                    TextFieldEvent::Menu(key) => app.slash_key(Some(id.clone()), *key, cx),
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
            }
            Err(message) => {
                self.form.error = Some(message.into());
                self.form.notice = None;
            }
        }
        cx.notify();
    }

    /// Watches every field of the connection editor, so saving knows as soon as it can.
    fn watch_form(&mut self, cx: &mut Context<Self>) {
        self.form_subscriptions.clear();
        for field in self.form.all_fields() {
            self.form_subscriptions.push(cx.subscribe(&field, |app, _, event: &TextFieldEvent, cx| match event {
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.save_connection(cx),
                TextFieldEvent::Changed => app.form_changed(cx),
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
            }));
        }
    }

    fn form_changed(&mut self, cx: &mut Context<Self>) {
        self.form.error = None;
        self.form.notice = None;
        self.form.notice_serial = self.form.notice_serial.wrapping_add(1);
        cx.notify();
    }

    /// Switches a connection on or off; chats that used one switched off fall back to another.
    pub fn set_connection_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        match self.store.set_connection_enabled(id, on) {
            Ok(()) => self.form.error = None,
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
            _ => (None, self.store.defaults().clone()),
        }
    }

    /// Sets the model for the chat on screen (and as the default for new chats).
    pub fn choose(&mut self, connection: Option<String>, model: Option<String>, effort: Option<Option<Reasoning>>, cx: &mut Context<Self>) {
        let (chat, choice) = self.current_choice();
        let resolved = self.store.resolve(&choice);
        let current_effort = resolved.as_ref().map(|resolved| resolved.effort);
        let choice = ModelChoice {
            connection: connection.or_else(|| resolved.as_ref().map(|resolved| resolved.connection.id.clone())),
            model: model.or_else(|| resolved.as_ref().map(|resolved| resolved.model.id.clone())),
            effort: effort.unwrap_or(current_effort),
        };
        if let Err(message) = self.store.set_choice(chat.as_deref(), choice) {
            self.notice = Some(message.into());
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
        let Selection::NewChat(project) = &self.selection else {
            return None;
        };
        self.new_chat_panes.get(project)
    }

    /// Changes the panes on `Screen::NewChat` for the selected project.
    pub fn change_new_chat_layout<R>(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Layout) -> R,
    ) -> Option<R> {
        let Selection::NewChat(project_id) = self.selection.clone() else {
            return None;
        };
        if !self.new_chat_panes.contains_key(&project_id) {
            let project = self.store.project(&project_id)?;
            let mut panes = ChatPanes::new(Layout::default(), project.path.clone());
            panes.set_runner(self.runs.clone());
            panes.set_commands(
                project.command(CommandKind::Build).map(str::to_string),
                project.command(CommandKind::Run).map(str::to_string),
                cx,
            );
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
            TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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
            Selection::Welcome | Selection::Settings => {}
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

fn restore(session: &Entity<ChatSession>, messages: Vec<String>, cx: &mut App) {
    let composer = session.read(cx).composer.clone();
    composer.update(cx, |composer, cx| {
        let mut parts = messages;
        let existing = composer.text().trim().to_string();
        if !existing.is_empty() {
            parts.push(existing);
        }
        composer.set_text(&parts.join("\n\n"), cx);
    });
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) => {}
            }));
        }
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

    /// The composer's text or caret changed: open, follow or close the menu, and light a typed
    /// `/command` that exists.
    pub fn composer_changed(&mut self, owner: Option<String>, cx: &mut Context<Self>) {
        let Some(field) = self.composer_of(owner.as_deref(), cx) else {
            return;
        };
        let (text, caret) = {
            let field = field.read(cx);
            (field.text().to_string(), field.caret())
        };
        let word = active_slash(&text, caret);
        // ⎋ keeps the menu shut for its word only; another word opens it again.
        if self.slash_dismissed.as_ref().is_some_and(|(from, start)| *from != owner || word.as_ref().is_none_or(|word| word.start != *start)) {
            self.slash_dismissed = None;
        }
        let word = word.filter(|word| self.slash_dismissed.as_ref() != Some(&(owner.clone(), word.start)));
        let previous = self.slash.take().filter(|slash| slash.owner == owner);
        let other = previous.is_none() && self.slash.is_some();
        match word {
            Some(range) => {
                let fresh = previous.as_ref().is_none_or(|slash| slash.range.start != range.start);
                if fresh || self.commands.is_empty() {
                    self.commands = self.store.command_catalog();
                }
                let query = text.get(range.start + 1..caret.max(range.start + 1)).unwrap_or("").to_string();
                let matches: Vec<Command> = matching(&self.commands, &query).into_iter().cloned().collect();
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
    /// was around the `/word` becoming its arguments.
    pub fn pick_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(slash) = self.slash.take() else {
            return;
        };
        let (Some(command), Some(field)) = (slash.matches.get(index), self.composer_of(slash.owner.as_deref(), cx)) else {
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
