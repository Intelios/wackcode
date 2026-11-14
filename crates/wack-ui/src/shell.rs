//! The window's root view: the sidebar beside one screen.
//!
//! The app draws its own title strip, so the 40-point band across the top of both columns is what
//! moves the window. The traffic lights sit in the sidebar's band.
//!
//! Composition is deliberately off-centre: content hangs from one left edge (`CONTENT_INSET` into
//! the main column) and the space to its right is left open, because that is where a chat's panes
//! will open. Nothing is centred in the window.
//!
//! The shell draws from plain data ([`SidebarData`], [`Screen`]). In the app that data is read
//! from [`AppState`] on every frame and the shell handles the actions its views dispatch; a
//! gallery scene hands it fixed data instead and handles nothing.

use std::rc::Rc;

use gpui::{
    App, Context, Entity, Focusable, Hsla, ListState, MouseButton, PathPromptOptions, Pixels, PromptLevel, SharedString, Window,
    WindowControlArea, div, linear_color_stop, linear_gradient, list, prelude::*, px, relative,
};

use crate::{
    actions::{
        self, AddConnection, AddFolder, Back, ChooseEffort, ChooseFormat, ChooseModel, CloseMenu, CloseSettings, Compact,
        DeleteChat, FocusNext, FocusPrevious, NewChat, NewChatIn, OpenEffortMenu, OpenModelMenu, OpenSettings,
        PinProject, RemoveConnection, RemoveProject, SelectChat, Send, Stop, ToggleProject, Unqueue,
    },
    app::{AppState, Menu, Selection, shown_path},
    chat::ChatSession,
    composer::{Choice, Composer, ComposerData, Input, MenuData, MenuGroup, MenuItem, Meter, Waiting},
    settings::{self, ProvidersData},
    icon::Icon,
    sidebar::{ChatRow, ChatState, ProjectRow, SIDEBAR_WIDTH, Sidebar, SidebarData},
    tags::{self, Tag},
    theme::Theme,
    transcript::{self, Handlers, Item, Transcript},
    typography::{Type, Typography},
};

/// Height of the strip across the top that holds the traffic lights and drags the window.
pub const TITLE_STRIP_HEIGHT: Pixels = px(40.);
/// Where content starts, measured from the main column's left edge.
pub const CONTENT_INSET: Pixels = px(64.);
/// The transcript's widest line.
pub const TRANSCRIPT_MAX_WIDTH: Pixels = px(700.);
/// How far the composer's fade reaches up over the transcript.
const DOCK_FADE: Pixels = px(36.);

#[derive(Clone, Debug)]
pub struct ProjectInfo {
    pub name: SharedString,
    /// Shown shortened, with `~` for the home folder.
    pub path: SharedString,
    pub branch: Option<SharedString>,
}

/// What a chat's transcript is drawn from.
#[derive(Clone)]
pub enum Body {
    /// Fixed items, drawn all at once: for scenes.
    Items(Vec<Item>),
    /// A live chat, drawn through GPUI's virtual list.
    Live(Entity<ChatSession>, ListState),
}

#[derive(Clone)]
pub enum Screen {
    /// No project yet: the first launch.
    Welcome { notice: Option<SharedString> },
    NewChat { project: ProjectInfo, composer: ComposerData, notice: Option<SharedString> },
    Chat { title: SharedString, project: ProjectInfo, body: Body, composer: ComposerData },
    Settings(ProvidersData),
    /// The data folder could not be opened.
    Failed(SharedString),
}

enum Source {
    Live(Entity<AppState>),
    Fixed(Box<(SidebarData, Screen)>),
    Failed(SharedString),
}

pub struct Shell {
    source: Source,
    /// The gallery has no real window controls; it draws neutral circles where they would be.
    traffic_light_stand_ins: bool,
    /// The selection whose text field was last given focus.
    focused: Option<Selection>,
}

impl Shell {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = match AppState::open_store() {
            Ok(store) => {
                let app = cx.new(|cx| AppState::new(store, cx));
                cx.observe(&app, |_, _, cx| cx.notify()).detach();
                Source::Live(app)
            }
            Err(message) => Source::Failed(message.into()),
        };
        Self { source, traffic_light_stand_ins: false, focused: None }
    }

    pub fn showing(sidebar: SidebarData, screen: Screen) -> Self {
        Self { source: Source::Fixed(Box::new((sidebar, screen))), traffic_light_stand_ins: true, focused: None }
    }

    fn app(&self) -> Option<Entity<AppState>> {
        match &self.source {
            Source::Live(app) => Some(app.clone()),
            _ => None,
        }
    }

    /// Calls `f` on the app state, for an action handler.
    fn with_app(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut AppState, &mut Context<AppState>)) {
        if let Some(app) = self.app() {
            app.update(cx, f);
        }
    }

    fn selected_chat(&self, cx: &App) -> Option<String> {
        match self.app()?.read(cx).selection() {
            Selection::Chat(id) => Some(id.clone()),
            _ => None,
        }
    }

    fn add_folder(&mut self, _: &AddFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                app.update(cx, |app, cx| app.add_project(&path, cx));
            }
        })
        .detach();
    }

    fn remove_project(&mut self, action: &RemoveProject, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let Some(name) = app.read(cx).store().project(&action.id).map(|project| project.name.clone()) else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Remove “{name}” from WackCode?"),
            Some("Its chats are removed with it. The folder and its files stay where they are."),
            &["Remove", "Cancel"],
            cx,
        );
        let id = action.id.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                app.update(cx, |app, cx| app.remove_project(&id, cx));
            }
        })
        .detach();
    }

    fn delete_chat(&mut self, action: &DeleteChat, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let Some(title) = app.read(cx).store().chat(&action.id).map(|chat| chat.title.clone()) else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Delete “{title}”?"),
            Some("The chat and its history are deleted. This can’t be undone."),
            &["Delete", "Cancel"],
            cx,
        );
        let id = action.id.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                app.update(cx, |app, cx| app.delete_chat(&id, cx));
            }
        })
        .detach();
    }

    fn send(&mut self, _: &Send, _: &mut Window, cx: &mut Context<Self>) {
        match self.selected_chat(cx) {
            Some(chat) => self.with_app(cx, |app, cx| app.submit(&chat, false, cx)),
            None => self.with_app(cx, |app, cx| app.start_chat(cx)),
        }
    }

    fn stop(&mut self, _: &Stop, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.selected_chat(cx) {
            self.with_app(cx, |app, cx| app.stop(&chat, cx));
        }
    }

    fn remove_connection(&mut self, action: &RemoveConnection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let Some(name) = app.read(cx).store().connections().iter().find(|connection| connection.id == action.id).map(|connection| connection.name.clone()) else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Remove “{name}”?"),
            Some("Its key is deleted from WackCode. Chats that used it switch to another connection."),
            &["Remove", "Cancel"],
            cx,
        );
        let id = action.id.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                app.update(cx, |app, cx| app.remove_connection(&id, cx));
            }
        })
        .detach();
    }

    fn open_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        self.with_app(cx, |app, cx| {
            // With nothing to pick from, the place to go is where connections are added.
            if app.store().connections().is_empty() {
                app.select(Selection::Settings, cx);
            } else {
                app.toggle_menu(menu, cx);
            }
        });
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        if app.read(cx).menu.is_some() {
            app.update(cx, |app, cx| app.close_menu(cx));
            return;
        }
        if *app.read(cx).selection() == Selection::Settings {
            app.update(cx, |app, cx| app.close_settings(cx));
            return;
        }
        self.stop(&Stop, window, cx);
    }

    /// Gives the current screen's text field focus once, when the screen changes.
    fn focus_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let app = app.read(cx);
        let selection = app.selection().clone();
        if self.focused.as_ref() == Some(&selection) {
            return;
        }
        self.focused = Some(selection);
        if let Some(field) = app.active_field(cx) {
            window.defer(cx, move |window, cx| {
                let handle = field.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            });
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        self.focus_field(window, cx);
        let (sidebar_data, screen) = match &self.source {
            Source::Live(app) => live_view(app.read(cx), cx),
            Source::Fixed(fixed) => fixed.as_ref().clone(),
            Source::Failed(message) => (SidebarData::default(), Screen::Failed(message.clone())),
        };

        let lights = div().flex().gap(px(8.)).pl(px(14.)).pt(px(14.)).when(self.traffic_light_stand_ins, |lights| {
            lights.children((0..3).map(|_| div().size(px(12.)).rounded_full().bg(theme.border_strong)))
        });
        let sidebar = div()
            .flex_none()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar)
            .child(drag_strip("sidebar-strip", lights))
            .child(Sidebar::new(sidebar_data));

        let (strip, body) = match screen {
            Screen::Welcome { notice } => (div(), welcome(notice, &theme).into_any_element()),
            Screen::NewChat { project, composer, notice } => {
                (div(), new_chat(project, composer, notice, &theme).into_any_element())
            }
            Screen::Chat { title, project, body, composer } => {
                (chat_strip(title, project, &theme), chat(body, composer, &theme).into_any_element())
            }
            Screen::Settings(data) => (
                div().size_full().child(settings::strip(&theme)),
                div()
                    .size_full()
                    .key_context(actions::FORM_CONTEXT)
                    .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
                    .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
                    .child(settings::page(data, &theme))
                    .into_any_element(),
            ),
            Screen::Failed(message) => (div(), failed(message, &theme).into_any_element()),
        };

        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .child(drag_strip("title-strip", strip))
            .child(div().flex_1().min_h_0().relative().child(body));

        tags::root(
            div()
                .key_context(actions::context())
                .on_action(cx.listener(|shell, _: &NewChat, _, cx| shell.with_app(cx, |app, cx| app.new_chat(None, cx))))
                .on_action(cx.listener(|shell, action: &NewChatIn, _, cx| {
                    let project = action.project.clone();
                    shell.with_app(cx, |app, cx| app.new_chat(Some(project), cx));
                }))
                .on_action(cx.listener(Self::add_folder))
                .on_action(cx.listener(|shell, _: &OpenSettings, _, cx| {
                    shell.with_app(cx, |app, cx| app.select(Selection::Settings, cx))
                }))
                .on_action(cx.listener(|shell, _: &CloseSettings, _, cx| shell.with_app(cx, |app, cx| app.close_settings(cx))))
                .on_action(cx.listener(|shell, action: &SelectChat, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.select(Selection::Chat(id), cx));
                }))
                .on_action(cx.listener(|shell, action: &ToggleProject, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.toggle_project(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &PinProject, _, cx| {
                    let (id, pinned) = (action.id.clone(), action.pinned);
                    shell.with_app(cx, |app, cx| app.set_pinned(&id, pinned, cx));
                }))
                .on_action(cx.listener(Self::remove_project))
                .on_action(cx.listener(Self::delete_chat))
                .on_action(cx.listener(Self::send))
                .on_action(cx.listener(Self::stop))
                .on_action(cx.listener(Self::back))
                .on_action(cx.listener(|shell, _: &Compact, _, cx| {
                    if let Some(chat) = shell.selected_chat(cx) {
                        shell.with_app(cx, |app, cx| app.compact(&chat, cx));
                    }
                }))
                .on_action(cx.listener(|shell, _: &OpenModelMenu, _, cx| shell.open_menu(Menu::Model, cx)))
                .on_action(cx.listener(|shell, _: &OpenEffortMenu, _, cx| shell.open_menu(Menu::Effort, cx)))
                .on_action(cx.listener(|shell, _: &CloseMenu, _, cx| shell.with_app(cx, |app, cx| app.close_menu(cx))))
                .on_action(cx.listener(|shell, action: &ChooseModel, _, cx| {
                    let (connection, model) = (action.connection.clone(), action.model.clone());
                    shell.with_app(cx, |app, cx| app.choose(Some(connection), Some(model), None, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseEffort, _, cx| {
                    let effort = action.effort;
                    shell.with_app(cx, |app, cx| app.choose(None, None, Some(effort), cx));
                }))
                .on_action(cx.listener(|shell, _: &AddConnection, _, cx| shell.with_app(cx, |app, cx| app.add_connection(cx))))
                .on_action(cx.listener(|shell, action: &ChooseFormat, _, cx| {
                    let format = action.format;
                    shell.with_app(cx, |app, cx| app.set_format(format, cx));
                }))
                .on_action(cx.listener(Self::remove_connection))
                .on_action(cx.listener(|shell, action: &Unqueue, _, cx| {
                    if let Some(chat) = shell.selected_chat(cx) {
                        let index = action.index;
                        shell.with_app(cx, |app, cx| app.unqueue(&chat, index, cx));
                    }
                }))
                .size_full()
                .flex()
                .bg(theme.window)
                .text_color(theme.text)
                .typography(Type::Ui)
                .child(sidebar)
                .child(main),
        )
    }
}

/// The sidebar and screen for the app's current state.
fn live_view(app: &AppState, cx: &App) -> (SidebarData, Screen) {
    let store = app.store();
    let mut sidebar = SidebarData::default();
    for project in store.projects() {
        let chats = store
            .chats(&project.id)
            .into_iter()
            .map(|chat| {
                let session = app.session(&chat.id).map(|session| session.read(cx));
                ChatRow {
                    id: chat.id.clone().into(),
                    title: chat.title.clone().into(),
                    age: AppState::age(chat.updated).into(),
                    state: match session {
                        Some(session) if session.running() => ChatState::Working,
                        Some(session) if session.unread => ChatState::Unread,
                        _ => ChatState::Idle,
                    },
                    selected: *app.selection() == Selection::Chat(chat.id.clone()),
                }
            })
            .collect();
        let row = ProjectRow {
            id: project.id.clone().into(),
            name: project.name.clone().into(),
            chats,
            open: app.is_open(&project.id),
            pinned: project.pinned,
        };
        if project.pinned {
            sidebar.pinned.push(row);
        } else {
            sidebar.projects.push(row);
        }
    }

    let info = |id: &str| {
        store.project(id).map(|project| ProjectInfo {
            name: project.name.clone().into(),
            path: shown_path(&project.path).into(),
            branch: app.branch(id).map(|branch| branch.to_string().into()),
        })
    };
    let screen = match app.selection() {
        Selection::Welcome => Screen::Welcome { notice: app.notice.clone() },
        Selection::Settings => Screen::Settings(ProvidersData::new(store.connections(), &app.form)),
        Selection::NewChat(project) => match info(project) {
            Some(project) => {
                let choice = store.defaults().clone();
                let resolved = store.resolve(&choice);
                let (model, effort) = model_words(resolved.as_ref());
                Screen::NewChat {
                    project,
                    composer: ComposerData {
                        input: Input::Field(app.draft.clone()),
                        model,
                        effort,
                        meter: None,
                        working: false,
                        waiting: Vec::new(),
                        roomy: true,
                        menu: menu_data(app, resolved.as_ref()),
                    },
                    notice: app.notice.clone(),
                }
            }
            None => Screen::Welcome { notice: None },
        },
        Selection::Chat(id) => {
            let (Some(chat), Some(session)) = (store.chat(id), app.session(id)) else {
                return (sidebar, Screen::Welcome { notice: None });
            };
            let project = info(&chat.project).unwrap_or(ProjectInfo { name: "".into(), path: "".into(), branch: None });
            let resolved = store.resolve(&chat.choice);
            let (model, effort) = model_words(resolved.as_ref());
            let state = session.read(cx);
            let window = resolved.as_ref().and_then(|resolved| resolved.model.context_window);
            let meter = state.context_tokens.map(|tokens| meter(tokens, window, !state.is_empty()));
            let mut waiting: Vec<Waiting> =
                state.steering.iter().map(|text| Waiting { text: first_line(text), steering: true }).collect();
            waiting.extend(state.queue.iter().map(|text| Waiting { text: first_line(text), steering: false }));
            Screen::Chat {
                title: chat.title.clone().into(),
                project,
                body: Body::Live(session.clone(), state.list.clone()),
                composer: ComposerData {
                    input: Input::Field(state.composer.clone()),
                    model,
                    effort,
                    meter,
                    working: state.running(),
                    waiting,
                    roomy: false,
                    menu: menu_data(app, resolved.as_ref()),
                },
            }
        }
    };
    (sidebar, screen)
}

/// The open menu's choices, with the one in use marked.
fn menu_data(app: &AppState, resolved: Option<&wack_core::Resolved>) -> Option<MenuData> {
    let kind = app.menu?;
    let groups = match kind {
        Menu::Model => app
            .store()
            .connections()
            .iter()
            .map(|connection| MenuGroup {
                title: Some(connection.name.clone().into()),
                items: connection
                    .models
                    .iter()
                    .map(|model| MenuItem {
                        label: model_name(&model.id).into(),
                        detail: model.context_window.map(|window| crate::work::count(window).into()),
                        selected: resolved.is_some_and(|resolved| {
                            resolved.connection.id == connection.id && resolved.model.id == model.id
                        }),
                        choice: Choice::Model { connection: connection.id.clone(), model: model.id.clone() },
                    })
                    .collect(),
            })
            .collect(),
        Menu::Effort => {
            use wack_core::Effort;
            let current = resolved.and_then(|resolved| resolved.effort);
            let item = |effort: Option<Effort>, label: &'static str, detail: Option<&'static str>| MenuItem {
                label: label.into(),
                detail: detail.map(Into::into),
                selected: current == effort,
                choice: Choice::Effort(effort),
            };
            vec![MenuGroup {
                title: Some("Reasoning effort".into()),
                items: vec![
                    item(None, "Default", Some("the provider’s")),
                    item(Some(Effort::Low), "Low", None),
                    item(Some(Effort::Medium), "Medium", None),
                    item(Some(Effort::High), "High", None),
                    item(Some(Effort::Xhigh), "Extra high", None),
                    item(Some(Effort::Max), "Max", None),
                ],
            }]
        }
    };
    Some(MenuData { kind, groups })
}

fn first_line(text: &str) -> SharedString {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim().to_string().into()
}

/// The words the composer shows for the model and its effort.
fn model_words(resolved: Option<&wack_core::Resolved>) -> (SharedString, Option<SharedString>) {
    match resolved {
        Some(resolved) => (
            model_name(&resolved.model.id).into(),
            Some(
                match resolved.effort {
                    None => "Default effort",
                    Some(wack_core::Effort::Low) => "Low",
                    Some(wack_core::Effort::Medium) => "Medium",
                    Some(wack_core::Effort::High) => "High",
                    Some(wack_core::Effort::Xhigh) => "Extra high",
                    Some(wack_core::Effort::Max) => "Max",
                }
                .into(),
            ),
        ),
        None => ("No model".into(), None),
    }
}

/// `zai-org/GLM-5.3-Flash` reads as `GLM-5.3-Flash`.
pub fn model_name(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

/// "14% context", or "41.2k context" when the model's window is unknown. Compact is offered
/// once there is enough to be worth summarising.
fn meter(tokens: u64, window: Option<u64>, has_messages: bool) -> Meter {
    match window.filter(|window| *window > 0) {
        Some(window) => {
            let percent = tokens as f32 / window as f32 * 100.;
            let label = if percent > 0. && percent < 1. {
                "<1% context".to_string()
            } else {
                format!("{}% context", percent.round().min(100.))
            };
            Meter { label: label.into(), high: percent >= 80., can_compact: has_messages && percent >= 25. }
        }
        None => Meter {
            label: format!("{} context", crate::work::count(tokens)).into(),
            high: false,
            can_compact: has_messages && tokens >= 50_000,
        },
    }
}

fn drag_strip(name: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(name)
        .flex_none()
        .w_full()
        .h(TITLE_STRIP_HEIGHT)
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.titlebar_double_click();
            } else {
                window.start_window_move();
            }
        })
        .child(content)
        .tag(name)
}

/// The words above a screen's display line, with the duck.
fn kicker(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Icon::Duck.view(px(15.), theme.text_soft))
        .child(div().typography(Type::Label).text_color(theme.text_dim).child(text))
}

fn notice_line(notice: Option<SharedString>, theme: &Theme) -> impl IntoElement {
    div().when_some(notice, |line, notice| line.pt(px(14.)).typography(Type::Ui).text_color(theme.danger).child(notice))
}

fn welcome(notice: Option<SharedString>, theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(CONTENT_INSET)
        .pr(px(48.))
        .child(div().h(relative(0.26)))
        .child(
            div()
                .max_w(px(560.))
                .flex()
                .flex_col()
                .child(kicker("Welcome to WackCode", theme))
                .child(div().pt(px(10.)).typography(Type::Display).text_color(theme.text).child("Open a folder to begin."))
                .child(
                    div().pt(px(14.)).typography(Type::Body).text_color(theme.text_soft).child(
                        "WackCode works inside a folder on your Mac: it reads, edits and runs \
                         what is there. Add one, then start a chat in it.",
                    ),
                )
                .child(
                    div()
                        .pt(px(28.))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(
                            div()
                                .id("add-folder-button")
                                .h(px(34.))
                                .px(px(16.))
                                .rounded(px(9.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .bg(theme.accent)
                                .hover(|button| button.bg(theme.accent_hover))
                                .on_click(|_, window, cx| window.dispatch_action(Box::new(AddFolder), cx))
                                .child(Icon::Folder.view(px(15.), theme.on_accent))
                                .child(div().typography(Type::UiStrong).text_color(theme.on_accent).child("Add folder…"))
                                .tag("add-folder-button"),
                        )
                        .child(div().typography(Type::Small).text_color(theme.text_faint).child("⌘O")),
                )
                .child(notice_line(notice, theme)),
        )
}

fn new_chat(project: ProjectInfo, composer: ComposerData, notice: Option<SharedString>, theme: &Theme) -> impl IntoElement {
    let hint = |keys: &'static str, what: &'static str| {
        div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.text_dim).child(keys))
            .child(div().text_color(theme.text_faint).child(what))
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(CONTENT_INSET)
        .pr(px(48.))
        .child(div().h(relative(0.22)))
        .child(
            div()
                .max_w(px(640.))
                .flex()
                .flex_col()
                .child(kicker("New chat in", theme))
                .child(div().pt(px(6.)).typography(Type::Display).text_color(theme.text).child(project.name.clone()))
                .child(project_line(&project, theme).pt(px(8.)))
                .child(div().pt(px(30.)).child(Composer::new(composer)))
                .child(
                    div()
                        .pt(px(12.))
                        .px(px(16.))
                        .flex()
                        .gap(px(20.))
                        .typography(Type::Small)
                        .child(hint("↩", "send"))
                        .child(hint("⇧↩", "new line")),
                )
                .child(div().px(px(16.)).child(notice_line(notice, theme))),
        )
}

fn project_line(project: &ProjectInfo, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .typography(Type::Mono)
        .text_color(theme.text_dim)
        .child(div().min_w_0().truncate().child(project.path.clone()))
        .when_some(project.branch.clone(), |line, branch| {
            line.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(Icon::Branch.view(px(13.), theme.text_faint))
                    .child(branch),
            )
        })
}

fn chat_strip(title: SharedString, project: ProjectInfo, theme: &Theme) -> gpui::Div {
    div()
        .size_full()
        .pl(CONTENT_INSET)
        .pr(px(20.))
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().min_w_0().truncate().typography(Type::UiStrong).text_color(theme.text).child(title))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.))
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(project.name)
                .when_some(project.branch, |line, branch| line.child("·").child(branch)),
        )
}

/// One transcript item in the column: inset from the left edge, no wider than a reading line.
fn column(child: impl IntoElement) -> impl IntoElement {
    div().w_full().pl(CONTENT_INSET).pr(px(48.)).child(div().max_w(TRANSCRIPT_MAX_WIDTH).child(child))
}

fn chat(body: Body, composer: ComposerData, theme: &Theme) -> impl IntoElement {
    let clear = Hsla { a: 0., ..theme.window };
    let transcript = match body {
        Body::Items(items) => div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .pb(DOCK_FADE)
            .child(column(Transcript::new(items)))
            .into_any_element(),
        Body::Live(session, list_state) => {
            let state = session.clone();
            let handlers = Handlers {
                toggle: Rc::new(move |key, _, cx| state.update(cx, |chat, cx| chat.toggle(key, cx))),
                open_link: Rc::new(|url, _, cx| {
                    // Only web links leave the app; anything else in a reply is just text.
                    if url.starts_with("https://") || url.starts_with("http://") {
                        cx.open_url(&url);
                    }
                }),
            };
            list(list_state, move |index, _, cx| {
                let theme = Theme::get(cx);
                let chat = session.read(cx);
                let items = chat.items();
                let Some(current) = items.get(index) else {
                    return div().into_any_element();
                };
                let previous = index.checked_sub(1).and_then(|index| items.get(index));
                column(transcript::item(current, previous, theme, &handlers)).into_any_element()
            })
            .size_full()
            .pt(px(24.))
            .pb(DOCK_FADE)
            .into_any_element()
        }
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(transcript)
                // Text scrolls out under the title strip through a short fade, not a hard edge.
                .child(div().absolute().top_0().left_0().right_0().h(px(28.)).bg(linear_gradient(
                    180.,
                    linear_color_stop(theme.window, 0.),
                    linear_color_stop(clear, 1.),
                ))),
        )
        .child(
            div()
                .flex_none()
                // Reaches up over the transcript, so text passes under the composer through a fade.
                .mt(-DOCK_FADE)
                .pl(CONTENT_INSET - px(16.))
                .pr(px(32.))
                .pt(DOCK_FADE)
                .pb(px(18.))
                .bg(linear_gradient(180., linear_color_stop(clear, 0.), linear_color_stop(theme.window, 0.3)))
                .child(div().max_w(TRANSCRIPT_MAX_WIDTH + px(32.)).child(Composer::new(composer))),
        )
}

fn failed(message: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(CONTENT_INSET)
        .pr(px(48.))
        .child(div().h(relative(0.26)))
        .child(kicker("WackCode could not start", theme))
        .child(div().pt(px(10.)).max_w(px(560.)).typography(Type::Body).text_color(theme.text_soft).child(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_reads_as_a_share_and_offers_compact_from_a_quarter() {
        let small = meter(4_000, Some(1_000_000), true);
        assert_eq!(small.label.as_ref(), "<1% context");
        assert!(!small.can_compact && !small.high);

        let quarter = meter(250_000, Some(1_000_000), true);
        assert_eq!(quarter.label.as_ref(), "25% context");
        assert!(quarter.can_compact);
        assert!(!meter(250_000, Some(1_000_000), false).can_compact, "nothing to compact in an empty chat");

        assert!(meter(850_000, Some(1_000_000), true).high);
        assert_eq!(meter(0, Some(1_000), true).label.as_ref(), "0% context");
        assert_eq!(meter(2_000_000, Some(1_000_000), true).label.as_ref(), "100% context");
    }

    #[test]
    fn without_a_window_the_meter_counts_tokens() {
        let unknown = meter(41_200, None, true);
        assert_eq!(unknown.label.as_ref(), "41.2k context");
        assert!(!unknown.can_compact);
        assert!(meter(60_000, Some(0), true).can_compact, "a zero window counts as unknown");
    }

    #[test]
    fn model_names_drop_the_organisation() {
        assert_eq!(model_name("zai-org/GLM-5.3-Flash"), "GLM-5.3-Flash");
        assert_eq!(model_name("claude-opus-5-5"), "claude-opus-5-5");
    }
}
