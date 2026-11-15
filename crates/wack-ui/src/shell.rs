//! The window's root view: the sidebar beside one screen.
//!
//! The app draws its own title strip, so the 40-point band across the top of both columns is what
//! moves the window. The traffic lights sit in the sidebar's band.
//!
//! Composition is deliberately off-centre: content hangs from one left edge (`CONTENT_INSET` into
//! the main column) and the space to its right is left open, because that is where a chat's panes
//! will open. Nothing is centred in the window.
//!
//! Where things go comes from the pane solver (`panes::solve`): the sidebar, the transcript and a
//! chat's docks are placed from the window's size on every frame, so the transcript keeps its
//! minimum width at any size the window allows, and the sidebar steps aside before a dock shrinks.
//!
//! The shell draws from plain data ([`SidebarData`], [`Screen`]). In the app that data is read
//! from [`AppState`] on every frame and the shell handles the actions its views dispatch; a
//! gallery scene hands it fixed data instead and handles nothing.

use std::rc::Rc;

use gpui::{
    App, Context, DragMoveEvent, Entity, Focusable, Hsla, ListState, MouseButton, PathPromptOptions, Pixels, PromptLevel,
    SharedString, Window, WindowControlArea, div, linear_color_stop, linear_gradient, list, point, prelude::*, px, relative,
};

use crate::{
    actions::{
        self, ActivatePane, AddConnection, AddFolder, Back, ChooseEffort, ChooseFormat, ChooseModel, CloseChatTab, CloseMenu,
        ClosePane, ClosePaneId, CloseSettings, CloseTab, Compact, DeleteChat, EqualizeSplit, FocusNext, FocusPrevious,
        MovePane, NewChat, NewChatIn, NewTerminal, NewTerminalWith, NextTab, OpenEffortMenu, OpenModelMenu, OpenSettings,
        PinProject, PreviousTab, RemoveConnection, RemoveProject, ResetDockSize, SelectChat, Send, Stop, ToggleChanges,
        ToggleProject, ToggleSidebar, ToggleTerminal, Unqueue,
    },
    app::{AppState, Menu, Selection, shown_path},
    chat::ChatSession,
    composer::{Choice, Composer, ComposerData, Input, MenuData, MenuGroup, MenuItem, Meter, Waiting},
    panes::{
        Axis, ChatPanes, Dock, Layout, PaneId, PaneKind, PaneModel,
        dock::{self, DraggedTab, Resizing},
        solve::{self, DOCK_MIN_HEIGHT, DOCK_MIN_WIDTH, STACK_MIN_HEIGHT, STACK_MIN_WIDTH, TRANSCRIPT_MIN_WIDTH, solve},
    },
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
/// Below this width the transcript draws closer to its edges, so lines stay long enough to read.
const NARROW_TRANSCRIPT: f32 = 600.;
const NARROW_INSET: Pixels = px(32.);
/// The main column's width from which the title strip also names the chat's project.
const ROOMY_STRIP: f32 = 980.;

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
    Fixed(Box<(SidebarData, Screen, StripData)>),
    Failed(SharedString),
}

pub struct Shell {
    source: Source,
    /// The gallery has no real window controls; it draws neutral circles where they would be.
    traffic_light_stand_ins: bool,
    /// The selection whose text field was last given focus.
    focused: Option<Selection>,
    /// The pane whose tab is being dragged.
    dragging: Option<PaneId>,
    /// A scene's panes and sidebar preference; the app's come from the chat and the store.
    scene_panes: Option<ChatPanes>,
    scene_sidebar_hidden: bool,
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
        Self {
            source,
            traffic_light_stand_ins: false,
            focused: None,
            dragging: None,
            scene_panes: None,
            scene_sidebar_hidden: false,
        }
    }

    pub fn showing(sidebar: SidebarData, screen: Screen) -> Self {
        Self::showing_tabs(sidebar, screen, StripData::default())
    }

    pub fn showing_tabs(sidebar: SidebarData, screen: Screen, strip: StripData) -> Self {
        Self {
            source: Source::Fixed(Box::new((sidebar, screen, strip))),
            traffic_light_stand_ins: true,
            focused: None,
            dragging: None,
            scene_panes: None,
            scene_sidebar_hidden: false,
        }
    }

    /// A scene's chat panes, and whether its sidebar is hidden.
    pub fn with_panes(mut self, panes: ChatPanes, sidebar_hidden: bool) -> Self {
        self.scene_panes = Some(panes);
        self.scene_sidebar_hidden = sidebar_hidden;
        self
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

    /// The chat on screen, if any.
    fn current_session(&self, cx: &App) -> Option<Entity<ChatSession>> {
        self.app()?.read(cx).current_chat().cloned()
    }

    /// Changes the layout of the chat on screen.
    fn change_layout<R>(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Layout) -> R) -> Option<R> {
        let session = self.current_session(cx)?;
        Some(session.update(cx, |session, cx| session.change_layout(cx, change)))
    }

    fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.current_session(cx) else {
            return;
        };
        let handle = session.read(cx).panes.model(pane).map(|model| model.focus_handle(cx));
        if let Some(handle) = handle {
            window.focus(&handle, cx);
        }
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.app().and_then(|app| app.read(cx).active_field(cx)) {
            let handle = field.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// The pane holding focus, if one does.
    fn focused_pane(&self, window: &Window, cx: &App) -> Option<PaneId> {
        let session = self.current_session(cx)?;
        let panes = &session.read(cx).panes;
        panes.layout.visible().into_iter().map(|pane| pane.id).find(|id| {
            panes.model(*id).is_some_and(|model| model.focus_handle(cx).contains_focused(window, cx))
        })
    }

    /// ⌃` and ⌘⇧G: a terminal takes focus when it shows, and focus goes back to the composer
    /// when the pane that had it hides.
    fn toggle_pane(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focused_pane(window, cx);
        match self.change_layout(cx, |layout| layout.toggle(kind)) {
            Some(Some(pane)) if kind == PaneKind::Terminal => self.focus_pane(pane, window, cx),
            Some(None) if had_focus.is_some() => self.focus_composer(window, cx),
            _ => {}
        }
    }

    /// A new terminal: as a tab beside `with`, or in the bottom dock.
    fn new_terminal(&mut self, with: Option<PaneId>, window: &mut Window, cx: &mut Context<Self>) {
        let made = self.change_layout(cx, |layout| {
            let terminal = layout.add(PaneKind::Terminal);
            if let Some(with) = with {
                layout.move_pane(terminal, crate::panes::Target::With(with));
            }
            terminal
        });
        if let Some(terminal) = made {
            self.focus_pane(terminal, window, cx);
        }
    }

    fn close_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let had_focus = self.focused_pane(window, cx) == Some(pane);
        self.change_layout(cx, |layout| layout.close(pane));
        if had_focus {
            self.focus_composer(window, cx);
        }
    }

    fn close_focused_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        match self.focused_pane(window, cx) {
            Some(pane) => self.close_pane(pane, window, cx),
            None => self.close_tab(window, cx),
        }
    }

    /// ⌘W outside a pane: closes the chat tab in front, or leaves the new-chat screen for the
    /// last tab.
    fn close_tab(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.with_app(cx, |app, cx| match app.selection().clone() {
            Selection::Chat(id) => app.close_tab(&id, cx),
            Selection::NewChat(_) => {
                if let Some(last) = app.tabs().last().cloned() {
                    app.select(Selection::Chat(last), cx);
                }
            }
            _ => {}
        });
    }

    /// ⌃⌘S. When the sidebar is open but stepped aside for a wide dock, asking for it narrows the
    /// dock to make room (or hides it, when even its minimum would not leave room).
    fn toggle_sidebar(&mut self, _: &ToggleSidebar, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height - TITLE_STRIP_HEIGHT));
        let stepped_aside = !app.read(cx).sidebar_hidden()
            && self.current_session(cx).is_some_and(|session| {
                matches!(app.read(cx).selection(), Selection::Chat(_))
                    && solve(width, height, true, &session.read(cx).panes.layout).sidebar_stepped_aside
            });
        if stepped_aside {
            let room = width - f32::from(SIDEBAR_WIDTH) - TRANSCRIPT_MIN_WIDTH;
            self.change_layout(cx, |layout| {
                if room >= DOCK_MIN_WIDTH {
                    layout.set_size(Dock::Right, Some(room));
                } else {
                    layout.set_hidden(Dock::Right, true);
                }
            });
        } else {
            app.update(cx, |app, cx| app.toggle_sidebar(cx));
        }
    }

    /// A resize handle moved: the dock or split takes the size the mouse gives it, within the
    /// limits the solver keeps.
    fn resize(&mut self, event: &DragMoveEvent<Resizing>, window: &mut Window, cx: &mut Context<Self>) {
        let resizing = event.drag(cx).clone();
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height));
        let (x, y) = (f32::from(event.event.position.x), f32::from(event.event.position.y));
        match resizing {
            Resizing::Dock(Dock::Right) => {
                let size = (width - x).clamp(DOCK_MIN_WIDTH, solve::widest_dock(width).max(DOCK_MIN_WIDTH));
                self.change_layout(cx, |layout| layout.set_size(Dock::Right, Some(size.round())));
            }
            Resizing::Dock(Dock::Bottom) => {
                let content = height - f32::from(TITLE_STRIP_HEIGHT);
                let size = (height - y).clamp(DOCK_MIN_HEIGHT, solve::tallest_dock(content).max(DOCK_MIN_HEIGHT));
                self.change_layout(cx, |layout| layout.set_size(Dock::Bottom, Some(size.round())));
            }
            Resizing::Split { dock, path, index, axis, start, total } => {
                let (at, minimum) = match axis {
                    Axis::Row => (x, STACK_MIN_WIDTH),
                    Axis::Column => (y - f32::from(TITLE_STRIP_HEIGHT), STACK_MIN_HEIGHT),
                };
                if total < 2. * minimum {
                    return;
                }
                let before = (at - start).clamp(minimum, total - minimum);
                self.change_layout(cx, |layout| layout.resize_split(dock, &path, index, before, total - before));
            }
        }
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
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        let (sidebar_data, screen, strip) = match &self.source {
            Source::Live(app) => live_view(app.read(cx), cx),
            Source::Fixed(fixed) => fixed.as_ref().clone(),
            Source::Failed(message) => (SidebarData::default(), Screen::Failed(message.clone()), StripData::default()),
        };
        let sidebar_hidden = match &self.source {
            Source::Live(app) => app.read(cx).sidebar_hidden(),
            _ => self.scene_sidebar_hidden,
        };

        // The solver places everything below the title strip from the window's size.
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height - TITLE_STRIP_HEIGHT));
        let mouse = window.mouse_position();
        let (solution, dock_elements, words) = {
            let empty = ChatPanes::new(Layout::default(), Default::default());
            let panes = match (&screen, &self.source) {
                (Screen::Chat { .. }, Source::Live(app)) => match app.read(cx).current_chat() {
                    Some(session) => &session.read(cx).panes,
                    None => &empty,
                },
                (Screen::Chat { .. }, Source::Fixed(_)) => self.scene_panes.as_ref().unwrap_or(&empty),
                _ => &empty,
            };
            let solution = solve(width, height, !sidebar_hidden, &panes.layout);
            let left = if solution.sidebar { f32::from(SIDEBAR_WIDTH) } else { 0. };
            let frame = dock::Frame {
                panes,
                solution: &solution,
                left,
                dragging: self.dragging.map(|pane| (pane, point(mouse.x, mouse.y - TITLE_STRIP_HEIGHT))),
            };
            let docks = dock::docks(&frame, &theme, cx);
            let words = matches!(screen, Screen::Chat { .. }).then(|| PaneWords::read(panes, cx));
            (solution, docks, words)
        };
        let left = if solution.sidebar { SIDEBAR_WIDTH } else { px(0.) };
        let narrow = solution.transcript.width < NARROW_TRANSCRIPT;
        // The project's name yields to the tabs when the strip is short of room.
        let roomy_strip = width - f32::from(left) >= ROOMY_STRIP;
        let inset = if narrow { NARROW_INSET } else { CONTENT_INSET };

        let lights = div().flex().gap(px(8.)).pl(px(14.)).pt(px(14.)).when(self.traffic_light_stand_ins, |lights| {
            lights.children((0..3).map(|_| div().size(px(12.)).rounded_full().bg(theme.border_strong)))
        });
        let sidebar = solution.sidebar.then(|| {
            div()
                .flex_none()
                .w(SIDEBAR_WIDTH)
                .h_full()
                .flex()
                .flex_col()
                .bg(theme.sidebar)
                .child(drag_strip(
                    "sidebar-strip",
                    div().size_full().flex().justify_between().child(lights).child(
                        div().pt(px(9.)).pr(px(10.)).child(sidebar_toggle(&theme)),
                    ),
                ))
                .child(Sidebar::new(sidebar_data))
        });

        let (strip, body) = match screen {
            Screen::Welcome { notice } => (div(), welcome(notice, inset, &theme).into_any_element()),
            Screen::NewChat { project, composer, notice } => {
                (tab_strip(&strip, None, inset, &theme), new_chat(project, composer, notice, inset, &theme).into_any_element())
            }
            Screen::Chat { project, body, composer, .. } => (
                tab_strip(&strip, Some((roomy_strip.then_some(project), words.unwrap_or_default())), inset, &theme),
                chat(body, composer, inset, narrow, &theme).into_any_element(),
            ),
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
            Screen::Failed(message) => (div(), failed(message, inset, &theme).into_any_element()),
        };
        // With the sidebar away, the strip makes room for the traffic lights and offers it back.
        let strip = if solution.sidebar {
            strip
        } else {
            div()
                .size_full()
                .flex()
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .child(div().w(px(70.)).child(div().flex().gap(px(8.)).pl(px(14.)).pt(px(14.)).when(
                            self.traffic_light_stand_ins,
                            |lights| lights.children((0..3).map(|_| div().size(px(12.)).rounded_full().bg(theme.border_strong))),
                        )))
                        .child(div().pt(px(9.)).child(sidebar_toggle(&theme))),
                )
                .child(div().flex_1().min_w_0().child(strip))
        };

        let transcript = solution.transcript;
        let content = div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                div()
                    .absolute()
                    .left(px(transcript.x) - left)
                    .top(px(transcript.y))
                    .w(px(transcript.width))
                    .h(px(transcript.height))
                    .child(body),
            )
            .children(dock_elements);
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.window)
            .child(drag_strip("title-strip", strip))
            .child(content);

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
                .on_action(cx.listener(Self::toggle_sidebar))
                .on_action(cx.listener(|shell, _: &ToggleChanges, window, cx| shell.toggle_pane(PaneKind::Changes, window, cx)))
                .on_action(cx.listener(|shell, _: &ToggleTerminal, window, cx| shell.toggle_pane(PaneKind::Terminal, window, cx)))
                .on_action(cx.listener(|shell, _: &NewTerminal, window, cx| shell.new_terminal(None, window, cx)))
                .on_action(cx.listener(|shell, action: &NewTerminalWith, window, cx| shell.new_terminal(Some(action.pane), window, cx)))
                .on_action(cx.listener(Self::close_focused_pane))
                .on_action(cx.listener(|shell, action: &ClosePaneId, window, cx| shell.close_pane(action.pane, window, cx)))
                .on_action(cx.listener(|shell, action: &ActivatePane, window, cx| {
                    let pane = action.pane;
                    shell.change_layout(cx, |layout| layout.activate(pane));
                    shell.focus_pane(pane, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &MovePane, _, cx| {
                    let (pane, target) = (action.pane, action.target);
                    shell.dragging = None;
                    shell.change_layout(cx, |layout| layout.move_pane(pane, target));
                }))
                .on_action(cx.listener(|shell, action: &ResetDockSize, _, cx| {
                    let dock = action.dock;
                    shell.change_layout(cx, |layout| layout.set_size(dock, None));
                }))
                .on_action(cx.listener(|shell, action: &EqualizeSplit, _, cx| {
                    let (dock, path, index) = (action.dock, action.path.clone(), action.index);
                    shell.change_layout(cx, |layout| layout.resize_split(dock, &path, index, 1., 1.));
                }))
                .on_action(cx.listener(|shell, _: &CloseTab, window, cx| shell.close_tab(window, cx)))
                .on_action(cx.listener(|shell, action: &CloseChatTab, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.close_tab(&id, cx));
                }))
                .on_action(cx.listener(|shell, _: &NextTab, _, cx| shell.with_app(cx, |app, cx| app.step_tab(true, cx))))
                .on_action(cx.listener(|shell, _: &PreviousTab, _, cx| shell.with_app(cx, |app, cx| app.step_tab(false, cx))))
                .on_drag_move(cx.listener(Self::resize))
                .on_drag_move(cx.listener(|shell, event: &DragMoveEvent<DraggedTab>, _, cx| {
                    let pane = event.drag(cx).pane;
                    if shell.dragging != Some(pane) {
                        shell.dragging = Some(pane);
                        cx.notify();
                    }
                }))
                .size_full()
                .flex()
                .bg(theme.window)
                .text_color(theme.text)
                .typography(Type::Ui)
                .children(sidebar)
                .child(main),
        )
    }
}

/// The words at the right of a chat's title strip that show and hide its panes.
#[derive(Clone, Debug, Default)]
pub struct PaneWords {
    pub changes: bool,
    pub changed_files: Option<usize>,
    pub terminal: bool,
}

impl PaneWords {
    fn read(panes: &ChatPanes, cx: &App) -> Self {
        let visible = panes.layout.visible();
        let changed_files = panes.layout.first_of(PaneKind::Changes).and_then(|id| match panes.model(id) {
            Some(PaneModel::Changes(changes)) => changes.read(cx).count(),
            _ => None,
        });
        Self {
            changes: visible.iter().any(|pane| pane.kind == PaneKind::Changes),
            changed_files,
            terminal: visible.iter().any(|pane| pane.kind == PaneKind::Terminal),
        }
    }
}

/// One chat tab in the title strip.
#[derive(Clone, Debug)]
pub struct TabData {
    pub id: SharedString,
    pub title: SharedString,
    pub active: bool,
    pub state: ChatState,
}

/// What the title strip shows: the open chats as tabs, and a "New chat" tab while writing one.
#[derive(Clone, Debug, Default)]
pub struct StripData {
    pub tabs: Vec<TabData>,
    pub new_chat: bool,
}

/// The sidebar, screen and title strip for the app's current state.
fn live_view(app: &AppState, cx: &App) -> (SidebarData, Screen, StripData) {
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
                return (sidebar, Screen::Welcome { notice: None }, StripData::default());
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
    let strip = StripData {
        tabs: app
            .tabs()
            .iter()
            .filter_map(|id| {
                let chat = store.chat(id)?;
                let session = app.session(id).map(|session| session.read(cx));
                Some(TabData {
                    id: id.clone().into(),
                    title: chat.title.clone().into(),
                    active: *app.selection() == Selection::Chat(id.clone()),
                    state: match session {
                        Some(session) if session.running() => ChatState::Working,
                        Some(session) if session.unread => ChatState::Unread,
                        _ => ChatState::Idle,
                    },
                })
            })
            .collect(),
        new_chat: matches!(app.selection(), Selection::NewChat(_)),
    };
    (sidebar, screen, strip)
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

fn welcome(notice: Option<SharedString>, inset: Pixels, theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(inset)
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

fn new_chat(
    project: ProjectInfo,
    composer: ComposerData,
    notice: Option<SharedString>,
    inset: Pixels,
    theme: &Theme,
) -> impl IntoElement {
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
        .pl(inset)
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

fn sidebar_toggle(theme: &Theme) -> impl IntoElement {
    div()
        .id("toggle-sidebar")
        .occlude()
        .size(px(24.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .hover(|button| button.bg(theme.hover))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSidebar), cx))
        .child(Icon::Sidebar.view(px(15.), theme.text_dim))
        .tag("toggle-sidebar")
}

/// The open chats as words across the strip, the one in front in strong type with a short rule
/// under it. On a chat, its project and the pane words sit at the right.
fn tab_strip(strip: &StripData, chat: Option<(Option<ProjectInfo>, PaneWords)>, inset: Pixels, theme: &Theme) -> gpui::Div {
    let tab = |id: Option<SharedString>, title: SharedString, active: bool, state: ChatState| {
        let group: SharedString = format!("chat-tab-{}", id.as_deref().unwrap_or("new")).into();
        let close = match &id {
            Some(id) => Box::new(CloseChatTab { id: id.to_string() }) as Box<dyn gpui::Action>,
            None => Box::new(CloseTab),
        };
        let select = id.clone().map(|id| SelectChat { id: id.to_string() });
        div()
            .id(SharedString::from(format!("tab-{}", id.as_deref().unwrap_or("new"))))
            .group(group.clone())
            .occlude()
            .relative()
            .min_w(px(64.))
            .max_w(px(220.))
            .h_full()
            .pl(px(12.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .when_some(select, |tab, select| tab.on_click(move |_, window, cx| window.dispatch_action(Box::new(select.clone()), cx)))
            .when(state != ChatState::Idle, |tab| {
                tab.child(div().flex_none().size(px(6.)).rounded_full().bg(match state {
                    ChatState::Working => theme.accent,
                    _ => theme.accent_text,
                }))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .typography(if active { Type::UiStrong } else { Type::Ui })
                    .text_color(if active { theme.text } else { theme.text_dim })
                    .when(!active, |title| title.group_hover(group.clone(), |title| title.text_color(theme.text_soft)))
                    .child(title),
            )
            .child(
                div()
                    .id(SharedString::from(format!("close-{}", id.as_deref().unwrap_or("new"))))
                    .flex_none()
                    .size(px(18.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .invisible()
                    .group_hover(group.clone(), |close| close.visible())
                    .hover(|close| close.bg(theme.hover))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(close.boxed_clone(), cx);
                    })
                    .child(Icon::Close.view(px(11.), theme.text_dim)),
            )
            .when(active, |tab| {
                tab.child(div().absolute().bottom_0().left(px(12.)).right(px(22.)).h(px(2.)).rounded(px(1.)).bg(theme.accent_line))
            })
            .tag(if active { "chat-tab-active" } else { "chat-tab" })
    };
    let mut tabs: Vec<_> = strip.tabs.iter().map(|data| tab(Some(data.id.clone()), data.title.clone(), data.active, data.state)).collect();
    if strip.new_chat {
        tabs.push(tab(None, "New chat".into(), true, ChatState::Idle));
    }
    let word = |id: &'static str, label: &'static str, count: Option<usize>, on: bool, action: Box<dyn gpui::Action>| {
        div()
            .id(id)
            .occlude()
            .relative()
            .flex_none()
            .h_full()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(5.))
            .typography(if on { Type::UiStrong } else { Type::Ui })
            .text_color(if on { theme.text } else { theme.text_dim })
            .hover(|word| word.text_color(theme.text))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .child(label)
            .when_some(count.filter(|count| *count > 0), |word, count| {
                word.child(div().typography(Type::Small).text_color(theme.text_faint).child(count.to_string()))
            })
            .when(on, |word| word.child(div().absolute().bottom_0().left(px(8.)).right(px(8.)).h(px(2.)).rounded(px(1.)).bg(theme.accent_line)))
            .tag(id)
    };
    let right = chat.map(|(project, words)| {
        div()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .pr(px(10.))
            .when_some(project, |right, project| {
                right.child(
                    div()
                        .min_w_0()
                        .pr(px(10.))
                        .truncate()
                        .typography(Type::Small)
                        .text_color(theme.text_faint)
                        .child(match project.branch {
                            Some(branch) => format!("{} · {branch}", project.name),
                            None => project.name.to_string(),
                        }),
                )
            })
            .child(word("toggle-changes", "Changes", words.changed_files, words.changes, Box::new(ToggleChanges)))
            .child(word("toggle-terminal", "Terminal", None, words.terminal, Box::new(ToggleTerminal)))
    });
    div()
        .size_full()
        .pl((inset - px(12.)).max(px(4.)))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().min_w_0().h_full().flex().overflow_hidden().children(tabs))
        .children(right)
}

/// One transcript item in the column: inset from the left edge, no wider than a reading line.
fn column(child: impl IntoElement, inset: Pixels, narrow: bool) -> impl IntoElement {
    div()
        .w_full()
        .pl(inset)
        .pr(if narrow { px(24.) } else { px(48.) })
        .child(div().max_w(TRANSCRIPT_MAX_WIDTH).child(child))
}

fn chat(body: Body, composer: ComposerData, inset: Pixels, narrow: bool, theme: &Theme) -> impl IntoElement {
    let clear = Hsla { a: 0., ..theme.window };
    let transcript = match body {
        Body::Items(items) => div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .pb(DOCK_FADE)
            .child(column(Transcript::new(items), inset, narrow))
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
                column(transcript::item(current, previous, theme, &handlers), inset, narrow).into_any_element()
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
                .pl(inset - px(16.))
                .pr(if narrow { px(16.) } else { px(32.) })
                .pt(DOCK_FADE)
                .pb(px(18.))
                .bg(linear_gradient(180., linear_color_stop(clear, 0.), linear_color_stop(theme.window, 0.3)))
                .child(div().max_w(TRANSCRIPT_MAX_WIDTH + px(32.)).child(Composer::new(composer))),
        )
}

fn failed(message: SharedString, inset: Pixels, theme: &Theme) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .pl(inset)
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
