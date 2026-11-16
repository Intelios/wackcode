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
    AutoTitleConfig, Project, Reasoning, Runner, Store, Ui, WireFormat, title, title_from,
};

use crate::{
    appearance_page::Thumbnail,
    backdrop::Picture,
    chat::{ChatEvent, ChatSession, now_millis},
    memory_page::{MemoryConfirm, MemoryEditor, MemoryState},
    motion::TitleSwap,
    panes::{ChatPanes, Layout, PaneKind},
    run_button::{CommandButtonData, CommandEditorData},
    settings::{ProviderForm, Section},
    text_field::{TextField, TextFieldEvent},
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
            }
        });
        let form = ProviderForm::new(cx);
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
            session_subscriptions: HashMap::new(),
            open_projects: HashSet::new(),
            branches: HashMap::new(),
            draft,
            new_chat_mode: AgentMode::Build,
            notice: None,
            form,
            menu: None,
            settings_section: Section::default(),
            confirming_unrestricted: false,
            memory: MemoryState::default(),
            memory_subscriptions: Vec::new(),
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
        app
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

    /// Sends the new-chat composer's text as the first message of a new chat.
    pub fn start_chat(&mut self, cx: &mut Context<Self>) {
        let Selection::NewChat(project) = self.selection.clone() else {
            return;
        };
        let text = self.draft.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let choice: ModelChoice = self.store.defaults().clone();
        if self.store.resolve(&choice).is_none() {
            self.notice = Some("Add a connection in Settings › Providers to start chatting.".into());
            cx.notify();
            return;
        }
        let chat = match self.store.create_chat(&project, &title_from(&text), choice, self.new_chat_mode) {
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
        self.run(&chat.id, Job::Prompt(text.clone()), cx);
        self.start_title(&chat.id, &text, cx);
    }

    /// Asks the title model for a name for a chat that has just started, alongside its first run.
    /// The opening-line title stays until the answer comes, and after a failure.
    fn start_title(&mut self, chat: &str, opening: &str, cx: &mut Context<Self>) {
        let (provider, model) = match self.store.title_setup() {
            None => return,
            Some(Ok(setup)) => setup,
            Some(Err(_)) => {
                self.note(chat, title::UNUSABLE, cx);
                return;
            }
        };
        let request = title::request(&model, opening);
        self.title_attempts += 1;
        let attempt = self.title_attempts;
        let id = chat.to_string();
        let work = Tokio::spawn(cx, async move { title::generate(&provider, &request).await });
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
        result: Result<(Option<String>, Option<wack_core::Usage>), String>,
        cx: &mut Context<Self>,
    ) {
        // A rename, stop or delete since: the answer is no longer wanted.
        if self.titling.get(chat).map(|(current, _)| *current) != Some(attempt) {
            return;
        }
        self.titling.remove(chat);
        // The usage ledger records this request (`purpose: "title"`) once its writer exists.
        let Ok((Some(name), _usage)) = result else {
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

    /// Settings › Providers › Automatic titles' switch. It cannot go on without a usable model.
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
    /// written during a run goes to the agent at its next step instead.
    pub fn submit(&mut self, chat: &str, steer: bool, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let composer = session.read(cx).composer.clone();
        let text = composer.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let (running, compacting) = {
            let session = session.read(cx);
            (session.running(), session.compacting())
        };
        if running {
            session.update(cx, |session, cx| {
                if steer && !compacting {
                    session.steer(text, cx);
                } else {
                    session.queue.push(text);
                    cx.notify();
                }
            });
            composer.update(cx, |composer, cx| composer.set_text("", cx));
            return;
        }
        if self.run(chat, Job::Prompt(text), cx) {
            composer.update(cx, |composer, cx| composer.set_text("", cx));
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
            self.run(chat, Job::Compact, cx);
        }
    }

    /// Takes a queued message back out to edit it: it joins the composer's text.
    pub fn unqueue(&mut self, chat: &str, index: usize, cx: &mut Context<Self>) {
        let Some(session) = self.sessions.get(chat).cloned() else {
            return;
        };
        let text = session.update(cx, |session, cx| {
            cx.notify();
            (index < session.queue.len()).then(|| session.queue.remove(index))
        });
        if let Some(text) = text {
            restore(&session, vec![text], cx);
        }
    }

    /// Starts a run in a chat. False, with the reason in its transcript, when it could not start.
    fn run(&mut self, chat_id: &str, job: Job, cx: &mut Context<Self>) -> bool {
        let Some(chat) = self.store.chat(chat_id).cloned() else {
            return false;
        };
        let session = self.ensure_session(&chat, cx);
        match self.store.run_setup(&chat) {
            Ok(setup) => {
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
            cx.notify();
            (after.unsent, after.next)
        });
        if !unsent.is_empty() {
            restore(&session, unsent, cx);
        }
        if let Some(next) = next
            && !self.run(chat, Job::Prompt(next.clone()), cx)
        {
            restore(&session, vec![next], cx);
        }
        cx.notify();
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
        let session = cx.new(|cx| ChatSession::load(&chat.id, cwd, &path, layout, runner, commands, cx));
        let id = chat.id.clone();
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
                }
            }),
            cx.subscribe(&composer, {
                let id = id.clone();
                move |app, _, event: &TextFieldEvent, cx| match event {
                    TextFieldEvent::Enter => app.submit(&id, false, cx),
                    TextFieldEvent::SendNow => app.submit(&id, true, cx),
                    TextFieldEvent::Changed => {}
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
                self.form.key.update(cx, |field, cx| field.set_text("", cx));
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
        let current_effort = resolved.as_ref().and_then(|resolved| resolved.effort);
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

    /// Terminates every active Build/Run process group cleanly when WackCode quits.
    pub fn shutdown_runs(&self) {
        self.runs.terminate_all();
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
