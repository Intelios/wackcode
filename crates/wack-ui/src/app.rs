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
//! - `tabs` lists chats that exist, each once; a chat on screen is always one of them. Tabs, the
//!   chat in front and whether the sidebar is hidden are saved as they change, and a chat's pane
//!   layout shortly after it changes (a drag changes it on every frame).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, SharedString, Subscription, Task};
use wack_core::{Appearance, Chat, Effort, Job, ModelChoice, Outcome, Project, Store, Ui, WireFormat, title_from};

use crate::{
    backdrop::Picture,
    chat::{ChatEvent, ChatSession, now_millis},
    panes::Layout,
    settings::{ProviderForm, Section},
    theme::{self, DEFAULT_ACCENT, DEFAULT_THEME_COLOUR},
    text_field::{TextField, TextFieldEvent},
    typography::Type,
};

pub const NEW_CHAT_PLACEHOLDER: &str = "Describe a task, or ask about the code";

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
    /// Why the last chat could not start, shown under the new-chat composer.
    pub notice: Option<SharedString>,
    /// Settings › Providers' add form.
    pub form: ProviderForm,
    pub menu: Option<Menu>,
    /// The section Settings shows.
    pub settings_section: Section,
    /// Settings › Appearance's hex fields, for colours the chips do not offer.
    pub theme_field: Entity<TextField>,
    pub accent_field: Entity<TextField>,
    /// Why the last appearance change could not be made.
    pub appearance_notice: Option<SharedString>,
    /// A backdrop picture is being read.
    pub importing: bool,
    /// Chats open as tabs, in order.
    tabs: Vec<String>,
    sidebar_hidden: bool,
    /// Chats whose pane layout changed since it was last saved.
    unsaved_layouts: HashSet<String>,
    saving_layouts: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// How long a layout must stay still before it is saved.
const LAYOUT_SAVE_DELAY: Duration = Duration::from_millis(400);

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
        let theme_field = cx.new(|cx| TextField::new(Type::Mono, DEFAULT_THEME_COLOUR, cx));
        let accent_field = cx.new(|cx| TextField::new(Type::Mono, DEFAULT_ACCENT, cx));
        let mut subscriptions = vec![draft_subscription];
        for (field, accent) in [(&theme_field, false), (&accent_field, true)] {
            subscriptions.push(cx.subscribe(field, move |app, field, event, cx| {
                let text = field.read(cx).text().to_string();
                match event {
                    // A complete colour applies as it is typed, so the page previews it.
                    // Writing the colour in use into the field changes nothing.
                    TextFieldEvent::Changed => {
                        if let Some(hex) = wack_core::appearance::hex_colour(&text).filter(|_| text.trim().trim_start_matches('#').len() == 6)
                            && hex != app.colour(accent)
                        {
                            app.set_colour(accent, Some(hex), false, cx);
                        }
                    }
                    TextFieldEvent::Enter | TextFieldEvent::SendNow => match wack_core::appearance::hex_colour(&text) {
                        Some(hex) => app.set_colour(accent, Some(hex), true, cx),
                        None if text.trim().is_empty() => app.set_colour(accent, None, true, cx),
                        None => {
                            app.appearance_notice = Some(format!("“{}” is not a colour. Write it as #rrggbb.", text.trim()).into());
                            cx.notify();
                        }
                    },
                }
            }));
        }
        for field in form.fields() {
            subscriptions.push(cx.subscribe(field, |app, _, event, cx| match event {
                TextFieldEvent::Enter | TextFieldEvent::SendNow => app.add_connection(cx),
                TextFieldEvent::Changed => {
                    if app.form.error.take().is_some() {
                        cx.notify();
                    }
                }
            }));
        }
        let ui = store.ui().clone();
        let mut tabs: Vec<String> = Vec::new();
        for id in ui.tabs {
            if store.chat(&id).is_some() && !tabs.contains(&id) {
                tabs.push(id);
            }
        }
        let selection = match (ui.current.filter(|id| tabs.contains(id)), store.projects().first()) {
            (Some(chat), _) => Selection::Chat(chat),
            (None, Some(project)) => Selection::NewChat(project.id.clone()),
            (None, None) => Selection::Welcome,
        };
        let mut app = Self {
            store,
            before_settings: selection.clone(),
            selection: Selection::Welcome,
            sessions: HashMap::new(),
            session_subscriptions: HashMap::new(),
            open_projects: HashSet::new(),
            branches: HashMap::new(),
            draft,
            notice: None,
            form,
            menu: None,
            settings_section: Section::default(),
            theme_field,
            accent_field,
            appearance_notice: None,
            importing: false,
            tabs,
            sidebar_hidden: ui.sidebar_hidden,
            unsaved_layouts: HashSet::new(),
            saving_layouts: None,
            _subscriptions: subscriptions,
        };
        theme::apply(app.store.appearance(), cx);
        app.show_colours(cx);
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

    /// Sets the theme colour or the accent; `None` is the default. `show` also writes the colour
    /// into its hex field (not while the user is typing there).
    pub fn set_colour(&mut self, accent: bool, hex: Option<String>, show: bool, cx: &mut Context<Self>) {
        // The default chosen by hand is kept as the default, so it follows any later change to it.
        let default = if accent { DEFAULT_ACCENT } else { DEFAULT_THEME_COLOUR };
        let hex = hex.filter(|hex| !hex.eq_ignore_ascii_case(default));
        self.set_appearance(
            |appearance| {
                if accent {
                    appearance.accent = hex;
                } else {
                    appearance.theme_colour = hex;
                }
            },
            cx,
        );
        if show {
            self.show_colours(cx);
        }
    }

    /// The theme colour or accent in use, as `#rrggbb`.
    pub fn colour(&self, accent: bool) -> String {
        let appearance = self.store.appearance();
        let (chosen, default) =
            if accent { (&appearance.accent, DEFAULT_ACCENT) } else { (&appearance.theme_colour, DEFAULT_THEME_COLOUR) };
        chosen.clone().unwrap_or_else(|| default.into())
    }

    /// Writes the colours in use into their hex fields.
    fn show_colours(&mut self, cx: &mut Context<Self>) {
        for (field, hex) in [(self.theme_field.clone(), self.colour(false)), (self.accent_field.clone(), self.colour(true))] {
            field.update(cx, |field, cx| {
                if field.text() != hex {
                    field.set_text(&hex, cx);
                }
            });
        }
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
                    Ok(mut backdrop) => {
                        // A new picture keeps the dim and texture the old one had.
                        if let Some(old) = &app.store.appearance().backdrop {
                            backdrop.dim = old.dim;
                            backdrop.texture = old.texture;
                        }
                        app.set_appearance(|appearance| appearance.backdrop = Some(backdrop), cx);
                    }
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
        let backdrop = self.store.appearance().backdrop.as_ref()?;
        Some(Picture {
            path: self.store.backdrop_path(backdrop).to_string_lossy().to_string().into(),
            soft_path: self.store.backdrop_soft_path(backdrop).to_string_lossy().to_string().into(),
            width: backdrop.width,
            height: backdrop.height,
            focus: (backdrop.focus_x, backdrop.focus_y),
            dim: backdrop.dim as f32 / 100.,
            texture: backdrop.texture,
            shade: backdrop.shade.unwrap_or([0, 255]),
        })
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
        let current = match &self.selection {
            Selection::Chat(id) => Some(id.clone()),
            _ => self.store.ui().current.clone().filter(|id| self.tabs.contains(id)),
        };
        let ui = Ui { sidebar_hidden: self.sidebar_hidden, tabs: self.tabs.clone(), current };
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
        match &self.selection {
            Selection::NewChat(_) => Some(self.draft.clone()),
            Selection::Chat(chat) => self.sessions.get(chat).map(|session| session.read(cx).composer.clone()),
            Selection::Settings => (self.settings_section == Section::Providers).then(|| self.form.name.clone()),
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
            }
            Selection::Welcome => {}
        }
        self.notice = None;
        self.menu = None;
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
        let chats: Vec<String> = self.store.chats(id).iter().map(|chat| chat.id.clone()).collect();
        if let Err(message) = self.store.remove_project(id) {
            self.notice = Some(message.into());
            cx.notify();
            return;
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
        let chat = match self.store.create_chat(&project, &title_from(&text), choice) {
            Ok(chat) => chat,
            Err(message) => {
                self.notice = Some(message.into());
                cx.notify();
                return;
            }
        };
        self.draft.update(cx, |draft, cx| draft.set_text("", cx));
        self.select(Selection::Chat(chat.id.clone()), cx);
        self.run(&chat.id, Job::Prompt(text), cx);
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
            // Steering that arrived as the run ended goes first: it was written before the queue.
            let mut waiting = std::mem::take(&mut session.steering);
            waiting.append(&mut session.queue);
            cx.notify();
            if matches!(result, Ok(Outcome::Done)) && !waiting.is_empty() {
                let next = waiting.remove(0);
                session.queue = waiting;
                (Vec::new(), Some(next))
            } else {
                (waiting, None)
            }
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
        let cwd = self.store.project(&chat.project).map(|project| project.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        let path = self.store.session_path(chat);
        let layout = Layout::from_saved(chat.layout.as_ref());
        let session = cx.new(|cx| ChatSession::load(&chat.id, cwd, &path, layout, cx));
        let id = chat.id.clone();
        let composer = session.read(cx).composer.clone();
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, {
                let id = id.clone();
                move |app, _, event: &ChatEvent, cx| match event {
                    ChatEvent::Finished(result) => app.finished(&id, result, cx),
                    ChatEvent::LayoutChanged => app.layout_changed(&id, cx),
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
        self.form.format = format;
        cx.notify();
    }

    /// Adds the connection the Settings form describes; the form is cleared once it is saved.
    pub fn add_connection(&mut self, cx: &mut Context<Self>) {
        let result = match self.form.read(cx) {
            Ok((new, key)) => self.store.add_connection(new, &key),
            Err(message) => Err(message),
        };
        match result {
            Ok(_) => self.form.clear(cx),
            Err(message) => self.form.error = Some(message.into()),
        }
        cx.notify();
    }

    pub fn remove_connection(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(message) = self.store.remove_connection(id) {
            self.form.error = Some(message.into());
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
    pub fn choose(&mut self, connection: Option<String>, model: Option<String>, effort: Option<Option<Effort>>, cx: &mut Context<Self>) {
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
}

/// Puts messages back in a chat's composer, ahead of anything already there.
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
    fn paths_in_the_home_folder_start_with_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(shown_path(&home.join("Documents/x")), "~/Documents/x");
        assert_eq!(shown_path(&home), "~");
        assert_eq!(shown_path(Path::new("/opt/elsewhere")), "/opt/elsewhere");
    }
}
