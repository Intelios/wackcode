//! The window's root view: the sidebar beside one screen.
//!
//! The app draws its own title strip, so the 40-point band across the top of both columns is what
//! moves the window. The traffic lights sit in the sidebar's band.
//!
//! The title strip and settings hang from one left edge (`CONTENT_INSET` into the main column).
//! The reading surfaces are centred, on the owner's call: a chat's transcript and composer sit
//! in a reading-width column centred in the space beside its panes, and the new-chat and
//! welcome screens centre their block like a title page, with the composer in the middle.
//!
//! Where things go comes from the pane solver (`panes::solve`): the sidebar, the transcript and a
//! chat's docks are placed from the window's size on every frame, so the transcript keeps its
//! minimum width at any size the window allows, and the sidebar steps aside before a dock shrinks.
//!
//! The window is a frame around two panes: the sidebar, and the stage holding the screen (the
//! transcript and its docks, a new chat, Settings). The title strip with the chat tabs runs across
//! the frame above the stage. Every edge keeps [`FRAME_GUTTER`]: the solver works in the frame's
//! inner space ([`solver_space`]), and anything that turns the mouse into a layout position goes
//! through [`to_solver`].
//!
//! Over an image backdrop the picture fills the window, blurred and dimmed as the user chose, and
//! both panes are frosted glass over it (`glass.rs`); text outside them (the tabs) sits on pills
//! of the same glass. The new-chat and welcome screens show the picture clear across the whole
//! stage, with glass only behind the heading (`backdrop.rs`); opening a chat eases it away to the
//! glass, and New chat eases it back (`clear`, `motion::Ease`). Without a picture the frame is the
//! sidebar's colour and the stage is the lit window.
//!
//! The shell draws from plain data ([`SidebarData`], [`Screen`]). In the app that data is read
//! from [`AppState`] on every frame and the shell handles the actions its views dispatch; a
//! gallery scene hands it fixed data instead and handles nothing.

use std::rc::Rc;

use gpui::{
    App, Context, DragMoveEvent, ElementId, Entity, FocusHandle, Focusable, Hsla, ListState, MouseButton, PathPromptOptions, Pixels,
    PromptLevel, SharedString, Size, Subscription, Window, WindowControlArea, div, list,
    point, prelude::*, px, relative,
};

use crate::{
    actions::{
        self, ActivatePane, AddFolder, Back, ChooseBackdrop, ChooseEffort, SlideEffort, ToggleTodos, ToggleWaiting, ChooseFocus, ClearCommandOutput, CloseCommandEditor, CycleAgentMode, SetAgentMode, PickOption, ShowQuestion, NextQuestion, PreviousQuestion, SubmitAnswers, CancelQuestion, WrapUpQuestion, ShowDoneTodos, DiscardPlan, KeepPlan, ConfirmDiscardPlan, ApprovePlan, SavePlan, CopyPlan, ToggleUnrestrictedPlanning, ToggleWebFetch, ToggleWebSearch, ToggleRouteFetch, CloseSearchEditor, SaveSearchProvider, TestSearchProvider, AskDeleteSearchProvider, ConfirmDeleteSearchProvider, CancelDeleteSearchProvider, ResetWebUsage, ChooseSearchProvider, AddSearchProvider, EditSearchProvider, ChooseSearchKey, ChooseSearchMethod, AddMcpServer, EditMcpServer, ToggleMcpServer, ToggleMcpTool, ExpandMcpServer, TestMcpServer, AskDeleteMcpServer, RemoveMcpEntry, ChooseMcpTransport, ChooseMcpScope, ToggleMcpProject, CloseMcpEditor, SaveMcpServer, TestMcpDraft, AddMcpEntry, ConfirmDeleteMcpServer, CancelDeleteMcpServer, ConfirmUnrestrictedPlanning, CancelUnrestrictedPlanning, ToggleMemory, SwitchProjectMemory, NewMemory, EditMemory, ChooseMemoryKind, SaveMemory, CloseMemoryEditor, AskDeleteMemory, AskRemoveMemoryFolder, ConfirmMemoryDelete, CancelMemoryDelete, RevealMemory, ToggleCommitAttribution, ToggleUsageRecording, RevealUsageFolder,
        ChooseBlur, ChooseDim, ChooseFormat, OpenCommandEditor, OpenConnectionForm, CloseConnectionForm, SwitchConnection, ChooseMode, ChooseModel, ChooseTexture, ChooseTheme, CloseChatTab, CloseMenu, ClosePane,
        ClosePaneId, CloseSettings, CloseTab, Compact, DeleteChat, DiscardConnection, EditConnection, ToggleConnectionKey, EqualizeSplit, FocusNext, FocusPrevious, MovePane, AddModelRow, RemoveModelRow, SaveCommandEditor, SaveConnection,
        NewChat, NewChatIn, NewTerminal, NewTerminalWith, NextTab, OpenEffortMenu, OpenModelMenu, OpenSettings,
        OpenSettingsSection, PinProject, PreviousTab, RemoveConnection, RemovePicture, ShowCommandOutput, ShowPicture, RemoveProject, ResetDockSize,
        PickCommand, SelectChat, Send, Stop, StopCommand, MoveToBackground, ToggleTasks, ToggleTasksPane, StopTask, ShowTask, ToggleAgentsPane, ShowAgent, StopAgent, ToggleChanges, ToggleProject, ToggleSidebar, ToggleTerminal, TriggerProjectCommand, Unqueue,
        ToggleAutoTitles, ToggleTitleModels, ChooseTitleModel, RenameChat, RenameChatAt, CancelRename, ToggleModelReasoning,
        StartSuggestion, DismissSuggestion, CopySuggestion, OpenChat,
        ToggleModelEffort, SwitchSkill, SwitchSkillFolder, RemoveSkillFolder, OpenSkill, CopySkill, ChooseSkillUse, NewSkill, SaveSkill,
        CloseSkill, AskDeleteSkill, ConfirmSkillDelete, CancelSkillDelete, AddSkillFolder, ImportSkills, SwitchSlashCommand, OpenSlashCommand,
        NewSlashCommand, SaveSlashCommand, CloseSlashCommand, AskDeleteSlashCommand, ConfirmSlashCommandDelete, CancelSlashCommandDelete,
        ChooseDelegation, ToggleHelperModels, ChooseHelperModel, SwitchAgent, SwitchAgentFolder, OpenAgent, NewAgent, DuplicateAgent,
        ChooseAgentModel, SaveAgent, CloseAgent, AskDeleteAgent, CancelAgentDelete, ConfirmAgentDelete,
    },
    app::{AppState, Menu, RenamePlace, Selection, shown_path},
    appearance_page::AppearanceData,
    backdrop::{self, Picture},
    edge_fade::faded,
    glass::{GLASS_BLUR, glass},
    chat::ChatSession,
    composer::{Choice, Composer, ComposerData, Input, MenuData, MenuGroup, MenuItem, Meter, Waiting},
    motion,
    panes::{
        Axis, ChatPanes, Dock, Layout, PaneId, PaneKind, PaneModel, Rect,
        dock::{self, DraggedTab, Resizing},
        solve::{self, DOCK_MIN_HEIGHT, DOCK_MIN_WIDTH, STACK_MIN_HEIGHT, STACK_MIN_WIDTH, TRANSCRIPT_MIN_WIDTH, solve},
    },
    run_button::{self, CommandButtonData, CommandEditorData},
    settings::{self, ProvidersData, Section, SettingsData},
    icon::Icon,
    sidebar::{ChatRow, ChatState, ProjectRow, SIDEBAR_WIDTH, Sidebar, SidebarData, TitleLook, rename_field},
    tags::{self, Tag},
    theme::Theme,
    transcript::{self, Handlers, Item, Transcript},
    typography::{Type, Typography},
};

/// Height of the strip across the top that holds the traffic lights and drags the window.
pub const TITLE_STRIP_HEIGHT: Pixels = px(44.);
/// The frame kept around the panes, and between the sidebar and the stage.
pub const FRAME_GUTTER: Pixels = px(8.);
/// The panes' corners.
pub const PANE_RADIUS: f32 = 14.;
/// Where macOS draws the traffic lights: inside the sidebar pane, centred on the title strip.
pub const TRAFFIC_LIGHTS: (f32, f32) = (20., 16.);

/// The pane solver's space for a window of `width` × `height`: the window less the frame's
/// gutters and the title strip. The sidebar and the stage tile it.
pub fn solver_space(width: f32, height: f32) -> (f32, f32) {
    let gutter = f32::from(FRAME_GUTTER);
    (width - 2. * gutter, height - f32::from(TITLE_STRIP_HEIGHT) - gutter)
}

/// A window position in the solver's space.
pub fn to_solver(x: f32, y: f32) -> (f32, f32) {
    (x - f32::from(FRAME_GUTTER), y - f32::from(TITLE_STRIP_HEIGHT))
}
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
/// Below this main-column width the Build/Run split buttons collapse to icons in the title strip.
const COMPACT_COMMANDS_BELOW: f32 = 640.;
/// The widest the centred new-chat and welcome blocks get.
const NEW_CHAT_WIDTH: f32 = 680.;
const WELCOME_WIDTH: f32 = 560.;
/// How far the transcript's text fades as it scrolls out under the title strip.
const TOP_FADE: Pixels = px(24.);

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
    /// Boxed: the page's data is far larger than any other screen's.
    Settings(Box<SettingsData>),
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
    /// A scene's backdrop; the app's comes from the store.
    scene_backdrop: Option<Picture>,
    _activation: Option<Subscription>,
    /// Holds focus while a screen has no text field, so its clicks and keys reach the shell.
    focus: Option<FocusHandle>,
    /// The duck's next paddle step is on its way.
    paddling: bool,
    /// How clear the picture on the stage is: 1 on the new-chat and welcome screens, easing to 0
    /// (the glass) once a chat opens. `None` until the first frame.
    clear: Option<motion::Ease>,
    /// Whether the Build/Run command editor popover was open on the last frame.
    had_command_editor: bool,
    /// Whether a Settings editor was open at the last frame: when one closes, however it closed
    /// (a button, ↩ in a field, a delete), its field is gone and the shell takes the keys again.
    had_settings_editor: bool,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = match AppState::open_store() {
            Ok(store) => {
                let app = cx.new(|cx| AppState::new(store, cx));
                cx.observe(&app, |_, _, cx| cx.notify()).detach();
                Source::Live(app)
            }
            Err(message) => Source::Failed(message.into()),
        };
        motion::follow_system(cx);
        crate::browser::attach(window);
        let activation = cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                motion::follow_system(cx);
            }
        });
        Self {
            source,
            traffic_light_stand_ins: false,
            focused: None,
            dragging: None,
            scene_panes: None,
            scene_sidebar_hidden: false,
            scene_backdrop: None,
            _activation: Some(activation),
            focus: Some(cx.focus_handle()),
            paddling: false,
            clear: None,
            had_command_editor: false,
            had_settings_editor: false,
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
            scene_backdrop: None,
            _activation: None,
            focus: None,
            paddling: false,
            clear: None,
            had_command_editor: false,
            had_settings_editor: false,
        }
    }

    /// Terminates active Build/Run process groups before the window or application closes.
    pub fn shutdown(&self, cx: &App) {
        if let Some(app) = self.app() {
            app.read(cx).shutdown_runs();
        }
    }

    /// A scene with its sidebar put away.
    pub fn without_sidebar(mut self) -> Self {
        self.scene_sidebar_hidden = true;
        self
    }

    /// A scene's chat panes, and whether its sidebar is hidden.
    pub fn with_panes(mut self, panes: ChatPanes, sidebar_hidden: bool) -> Self {
        self.scene_panes = Some(panes);
        self.scene_sidebar_hidden = sidebar_hidden;
        self
    }

    /// A scene drawn over a backdrop.
    pub fn with_backdrop(mut self, picture: Picture) -> Self {
        self.scene_backdrop = Some(picture);
        self
    }

    /// The browser tabs of the chat in front, for the automation socket.
    #[cfg(feature = "automation")]
    pub fn current_tabs(&self, cx: &App) -> Option<wack_browser::ChatTabs> {
        Some(self.app()?.read(cx).current_chat()?.read(cx).tabs.clone())
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

    fn begin_rename(&mut self, chat: &str, place: RenamePlace, window: &mut Window, cx: &mut Context<Self>) {
        self.with_app(cx, |app, cx| app.begin_rename(chat, place, window, cx));
    }

    /// File › Rename Chat…: the chat on screen, on its tab.
    fn rename_on_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.selected_chat(cx) {
            self.begin_rename(&chat, RenamePlace::Tab, window, cx);
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
            // With nothing to pick from, the place to go is where connections are added or switched on.
            if app.store().enabled_connections().next().is_none() {
                app.settings_section = Section::Providers;
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
        if app.read(cx).command_editor.is_some() {
            app.update(cx, |app, cx| app.close_command_editor(cx));
            self.focus_composer(window, cx);
            return;
        }
        if app.read(cx).menu.is_some() {
            app.update(cx, |app, cx| app.close_menu(cx));
            return;
        }
        if *app.read(cx).selection() == Selection::Settings {
            app.update(cx, |app, cx| app.close_settings(cx));
            return;
        }
        // An open question is dismissed before the run is stopped.
        if let Some(session) = self.current_session(cx)
            && session.read(cx).asking.is_some()
        {
            session.update(cx, |session, cx| session.cancel_question(cx));
            return;
        }
        self.stop(&Stop, window, cx);
    }

    /// Runs `change` on the chat on screen.
    fn with_session(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut ChatSession, &mut Context<ChatSession>)) {
        if let Some(session) = self.current_session(cx) {
            session.update(cx, change);
        }
    }

    /// The chat on screen, if any.
    fn current_session(&self, cx: &App) -> Option<Entity<ChatSession>> {
        self.app()?.read(cx).current_chat().cloned()
    }

    /// The panes on the active screen (`Chat` or `NewChat`).
    fn current_panes<'a>(&self, cx: &'a App) -> Option<&'a ChatPanes> {
        let app = self.app()?.read(cx);
        match app.selection() {
            Selection::Chat(id) => app.session(id).map(|session| &session.read(cx).panes),
            Selection::NewChat(_) => app.current_new_chat_panes(),
            Selection::Welcome | Selection::Settings => None,
        }
    }

    /// Changes the layout of the chat or new-chat screen on view.
    fn change_layout<R>(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Layout) -> R) -> Option<R> {
        let app = self.app()?;
        match app.read(cx).selection().clone() {
            Selection::Chat(id) => {
                let session = app.read(cx).session(&id).cloned()?;
                Some(session.update(cx, |session, cx| session.change_layout(cx, change)))
            }
            Selection::NewChat(_) => app.update(cx, |app, cx| app.change_new_chat_layout(cx, change)),
            Selection::Welcome | Selection::Settings => None,
        }
    }

    fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self
            .current_panes(cx)
            .and_then(|panes| panes.model(pane))
            .map(|model| model.focus_handle(cx));
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
        let panes = self.current_panes(cx)?;
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
        let (width, height) = solver_space(f32::from(viewport.width), f32::from(viewport.height));
        let stepped_aside = !app.read(cx).sidebar_hidden()
            && self.current_panes(cx).is_some_and(|panes| {
                solve(width, height, true, &panes.layout).sidebar_stepped_aside
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
        let (width, height) = solver_space(f32::from(viewport.width), f32::from(viewport.height));
        let (x, y) = to_solver(f32::from(event.event.position.x), f32::from(event.event.position.y));
        match resizing {
            Resizing::Dock(Dock::Right) => {
                let size = (width - x).clamp(DOCK_MIN_WIDTH, solve::widest_dock(width).max(DOCK_MIN_WIDTH));
                self.change_layout(cx, |layout| layout.set_size(Dock::Right, Some(size.round())));
            }
            Resizing::Dock(Dock::Bottom) => {
                let size = (height - y).clamp(DOCK_MIN_HEIGHT, solve::tallest_dock(height).max(DOCK_MIN_HEIGHT));
                self.change_layout(cx, |layout| layout.set_size(Dock::Bottom, Some(size.round())));
            }
            Resizing::Split { dock, path, index, axis, start, total } => {
                let (at, minimum) = match axis {
                    Axis::Row => (x, STACK_MIN_WIDTH),
                    Axis::Column => (y, STACK_MIN_HEIGHT),
                };
                if total < 2. * minimum {
                    return;
                }
                let before = (at - start).clamp(minimum, total - minimum);
                self.change_layout(cx, |layout| layout.resize_split(dock, &path, index, before, total - before));
            }
        }
    }

    /// Gives the current screen's text field focus once, when the screen changes. On a screen
    /// without one the shell itself takes focus: typing never lands in a field that is not
    /// drawn, and actions still start inside the shell (with nothing focused, GPUI dispatches
    /// from the window's root, above every handler).
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
        let handle = match app.active_field(cx) {
            Some(field) => Some(field.read(cx).focus_handle(cx)),
            None => self.focus.clone(),
        };
        if let Some(handle) = handle {
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
    }

    /// While an agent works, the duck takes a paddle step every [`motion::PADDLE_STEP`]; each
    /// step redraws the window, which schedules the next. Under Reduce Motion it stays at rest.
    fn paddle(&mut self, sidebar: &SidebarData, cx: &mut Context<Self>) {
        let working = sidebar.pinned.iter().chain(&sidebar.projects).flat_map(|project| &project.chats).any(|chat| chat.state == ChatState::Working);
        if !working || cx.reduce_motion() || !matches!(self.source, Source::Live(_)) {
            motion::paddle_rest();
            return;
        }
        if !self.paddling {
            self.paddling = true;
            cx.spawn(async move |shell, cx| {
                cx.background_executor().timer(motion::PADDLE_STEP).await;
                let _ = shell.update(cx, |shell, cx| {
                    shell.paddling = false;
                    motion::paddle_on();
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Opens Settings › Memory's editor and puts the caret in its title.
    fn open_memory_editor(&mut self, dir: &std::path::Path, name: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let title = app.update(cx, |app, cx| app.open_memory_editor(dir, name, cx));
        if let Some(title) = title {
            let handle = title.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
    }

    /// Puts focus in a field that has just appeared on the page.
    fn focus_new_field(&mut self, field: Option<Entity<crate::text_field::TextField>>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = field {
            let handle = field.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
    }

    /// Asks for a folder (or, with `files`, a `.md` or `.zip` too) and hands it to `then`.
    fn pick_path(&mut self, files: bool, prompt: &'static str, cx: &mut Context<Self>, then: fn(&mut AppState, &std::path::Path, &mut Context<AppState>)) {
        let Some(app) = self.app() else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions { files, directories: true, multiple: false, prompt: Some(prompt.into()) });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                app.update(cx, |app, cx| then(app, &path, cx));
            }
        })
        .detach();
    }

    /// Once an editor (memory, skill, command or MCP server) has closed, the shell takes the keys again.
    fn refocus_if_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.app().is_some_and(|app| settings_editor_open(app.read(cx)));
        if !open && let Some(focus) = self.focus.clone() {
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
    }

    fn open_settings_section(&mut self, action: &OpenSettingsSection, _: &mut Window, cx: &mut Context<Self>) {
        let section = action.section;
        self.with_app(cx, |app, cx| {
            app.open_settings_section(section, cx);
        });
        // The new section's field, if it has one, takes focus.
        self.focused = None;
    }

    fn choose_backdrop(&mut self, _: &ChooseBackdrop, _: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        if app.read(cx).importing {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                app.update(cx, |app, cx| app.import_backdrop(path, cx));
            }
        })
        .detach();
    }

    /// Changes the backdrop's settings, when there is one.
    fn change_backdrop(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut wack_core::Backdrop)) {
        self.with_app(cx, |app, cx| {
            app.set_appearance(
                |appearance| {
                    if let Some(backdrop) = appearance.backdrop_mut() {
                        change(backdrop);
                    }
                },
                cx,
            )
        });
    }
}

/// Whether one of Settings' editors (memory, skill, command, search provider or MCP server) is open.
fn settings_editor_open(app: &AppState) -> bool {
    app.memory.editor.is_some() || app.skills.editor.is_some() || app.templates.editor.is_some() || app.subagents.editor.is_some() || app.web.editor.is_some() || app.mcp_settings.editor.is_some()
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        self.focus_field(window, cx);
        let command_editor_open = self.app().is_some_and(|app| app.read(cx).command_editor.is_some());
        if self.had_command_editor && !command_editor_open {
            self.focus_composer(window, cx);
        }
        self.had_command_editor = command_editor_open;
        let settings_editor_open = self.app().is_some_and(|app| settings_editor_open(app.read(cx)));
        if self.had_settings_editor && !settings_editor_open {
            self.refocus_if_closed(window, cx);
        }
        self.had_settings_editor = settings_editor_open;
        // A question card takes focus as it arrives, and gives it back to the composer as it goes.
        if let Some(session) = self.current_session(cx)
            && let Some(handle) = session.update(cx, |session, _| session.focus_request.take())
        {
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        if !cx.has_active_drag() {
            self.dragging = None;
        }
        let (sidebar_data, screen, strip) = match &self.source {
            Source::Live(app) => live_view(app.read(cx), cx),
            Source::Fixed(fixed) => fixed.as_ref().clone(),
            Source::Failed(message) => (SidebarData::default(), Screen::Failed(message.clone()), StripData::default()),
        };
        self.paddle(&sidebar_data, cx);
        if let Source::Live(app) = &self.source
            && app.read(cx).title_swaps.values().any(motion::TitleSwap::live)
        {
            window.request_animation_frame();
        }
        let sidebar_hidden = match &self.source {
            Source::Live(app) => app.read(cx).sidebar_hidden(),
            _ => self.scene_sidebar_hidden,
        };

        // The solver places the sidebar and the stage inside the frame from the window's size.
        let viewport = window.viewport_size();
        let (window_width, window_height) = (f32::from(viewport.width), f32::from(viewport.height));
        let (width, height) = solver_space(window_width, window_height);
        let mouse = window.mouse_position();
        let (solution, dock_elements, words) = {
            let empty = ChatPanes::new(Layout::default(), Default::default());
            let panes = match (&screen, &self.source) {
                (Screen::Chat { .. }, Source::Live(app)) => match app.read(cx).current_chat() {
                    Some(session) => &session.read(cx).panes,
                    None => &empty,
                },
                (Screen::NewChat { .. }, Source::Live(app)) => app.read(cx).current_new_chat_panes().unwrap_or(&empty),
                (Screen::Chat { .. } | Screen::NewChat { .. }, Source::Fixed(_)) => self.scene_panes.as_ref().unwrap_or(&empty),
                _ => &empty,
            };
            let solution = solve(width, height, !sidebar_hidden, &panes.layout);
            let left = if solution.sidebar { f32::from(SIDEBAR_WIDTH) } else { 0. };
            let frame = dock::Frame {
                panes,
                solution: &solution,
                left,
                width,
                height,
                dragging: self.dragging.map(|pane| {
                    let (x, y) = to_solver(f32::from(mouse.x), f32::from(mouse.y));
                    (pane, point(px(x), px(y)))
                }),
            };
            let docks = dock::docks(&frame, &theme, cx);
            let words = matches!(screen, Screen::Chat { .. }).then(|| PaneWords::read(panes, cx));
            (solution, docks, words)
        };
        let left = if solution.sidebar { SIDEBAR_WIDTH } else { px(0.) };
        let narrow = solution.transcript.width < NARROW_TRANSCRIPT;
        let transcript_height = solution.transcript.height;
        // The project's name yields to the tabs when the strip is short of room.
        let main_width = width - f32::from(left);
        let roomy_strip = main_width >= ROOMY_STRIP;
        let compact_commands = main_width < COMPACT_COMMANDS_BELOW;
        let inset = if narrow { NARROW_INSET } else { CONTENT_INSET };

        // The picture, when there is one: blurred and dimmed behind everything, and clear across
        // the stage on the new-chat and welcome screens, frosting over as a chat opens.
        let picture = match &self.source {
            Source::Live(app) => app.read(cx).backdrop(),
            _ => self.scene_backdrop.clone(),
        };
        let over_picture = picture.is_some();
        let hero_screen = matches!(screen, Screen::Welcome { .. } | Screen::NewChat { .. });
        let target = if over_picture && hero_screen { 1. } else { 0. };
        let still = cx.reduce_motion();
        let clear = self.clear.get_or_insert(motion::Ease::at(target));
        clear.toward(target, still);
        let (clearness, clearing) = (clear.value(), clear.moving());
        if clearing {
            window.request_animation_frame();
        }
        let whole = Rect { x: 0., y: 0., width: window_width, height: window_height };
        let (behind, hero) = match &picture {
            Some(picture) => {
                let behind = backdrop::behind(picture, whole, theme.window, theme.veil);
                let light = theme.appearance == crate::theme::Appearance::Light;
                let artwork = (clearness > 0.001).then(|| backdrop::artwork(picture, light, cx)).flatten();
                let hero = artwork.map(|artwork| {
                    div().absolute().left_0().top_0().size_full().opacity(clearness).child(motion::fade_in(
                        "hero",
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .size_full()
                            .child(backdrop::hero(artwork, picture, whole, PANE_RADIUS)),
                    ))
                });
                (Some(behind), hero)
            }
            None => (None, None),
        };
        // The panes: frosted glass over a picture; without one, the sidebar lies flat on the frame
        // beside the lit stage.
        let pane = |child: gpui::AnyElement, stage: bool| -> gpui::AnyElement {
            if over_picture {
                glass(PANE_RADIUS, GLASS_BLUR, Hsla { a: theme.glass, ..theme.window }, theme.border, child).into_any_element()
            } else if stage {
                glass(PANE_RADIUS, 0., theme.window, theme.border, child).into_any_element()
            } else {
                child
            }
        };
        let gutter = FRAME_GUTTER;
        // While Settings is open the sidebar's pane holds its navigation.
        let settings_nav = match &screen {
            Screen::Settings(data) => Some((data.section, data.providers.connections.len())),
            _ => None,
        };
        let sidebar = solution.sidebar.then(|| {
            div().flex_none().w(gutter + SIDEBAR_WIDTH).h_full().p(gutter).flex().flex_col().child(pane(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(drag_strip(
                        "sidebar-strip",
                        TITLE_STRIP_HEIGHT - gutter,
                        div().size_full().flex().justify_end().pt(px(2.)).pr(px(10.)).child(sidebar_toggle(&theme)),
                    ))
                    .child(match settings_nav {
                        Some((section, connections)) => settings::nav(section, connections, &theme).into_any_element(),
                        None => Sidebar::new(sidebar_data).into_any_element(),
                    })
                    .into_any_element(),
                false,
            ))
        });

        // A screen arrives as it is opened; a chat does not, so switching tabs never moves text.
        let arrive = |id: String, body: gpui::AnyElement| {
            motion::enter(ElementId::Name(id.into()), div().size_full().child(body)).into_any_element()
        };
        let strip_ground = over_picture.then_some(Hsla { a: theme.glass, ..theme.window });
        let (strip, body) = match screen {
            Screen::Welcome { notice } => {
                let ground = (clearness > 0.).then_some(Hsla { a: theme.cover(), ..theme.window });
                (div(), arrive("arrive-welcome".into(), welcome(notice, ground, &theme).into_any_element()))
            }
            Screen::NewChat { project, composer, notice } => {
                let id = format!("arrive-new-chat-{}", project.path);
                let ground = (clearness > 0.).then_some(Hsla { a: theme.cover(), ..theme.window });
                let body = new_chat(project, composer, &strip, viewport, notice, ground, &theme).into_any_element();
                (tab_strip(&strip, None, compact_commands, viewport, strip_ground, &theme), arrive(id, body))
            }
            Screen::Chat { project, body, composer, .. } => (
                tab_strip(
                    &strip,
                    Some((roomy_strip.then_some(project), words.unwrap_or_default())),
                    compact_commands,
                    viewport,
                    strip_ground,
                    &theme,
                ),
                chat(body, composer, narrow, transcript_height).into_any_element(),
            ),
            Screen::Settings(data) => {
                let id = format!("arrive-settings-{}", data.section.name());
                (
                    div().size_full().child(settings::strip(data.section, inset, strip_ground, &theme)),
                    div()
                        .size_full()
                        .key_context(actions::FORM_CONTEXT)
                        .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
                        .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
                        .child(arrive(id, settings::page(*data, !solution.sidebar, narrow, solution.transcript.width < settings::FORM_STACK_BELOW, &theme).into_any_element()))
                        .into_any_element(),
                )
            }
            Screen::Failed(message) => (div(), failed(message, &theme).into_any_element()),
        };
        // With the sidebar away, the strip makes room for the traffic lights and offers it back.
        let strip = if solution.sidebar {
            strip
        } else {
            div()
                .size_full()
                .flex()
                .child(div().flex_none().w(px(TRAFFIC_LIGHTS.0 + 56.) - gutter))
                .child(div().flex_none().pt(px(10.)).child(sidebar_toggle(&theme)))
                .child(div().flex_1().min_w_0().child(strip))
        };

        let transcript = solution.transcript;
        let content = div()
            .size_full()
            .relative()
            .children(hero)
            .child(
                div()
                    .absolute()
                    .left(px(transcript.x) - left)
                    .top(px(transcript.y))
                    .w(px(transcript.width))
                    .h(px(transcript.height))
                    .overflow_hidden()
                    .child(body),
            )
            .children(dock_elements);
        let stage = pane(div().flex_1().min_h_0().w_full().child(content).into_any_element(), true);
        let main = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pr(gutter)
            .pb(gutter)
            .when(!solution.sidebar, |main| main.pl(gutter))
            .child(drag_strip("title-strip", TITLE_STRIP_HEIGHT, strip))
            .child(stage);
        // The gallery has no window controls: neutral circles stand where macOS draws them.
        let lights = self.traffic_light_stand_ins.then(|| {
            div()
                .absolute()
                .left(px(TRAFFIC_LIGHTS.0))
                .top(px(TRAFFIC_LIGHTS.1))
                .flex()
                .gap(px(8.))
                .children((0..3).map(|_| div().size(px(12.)).rounded_full().bg(theme.border_strong)))
        });

        tags::root(
            div()
                .key_context(actions::context())
                .when_some(self.focus.as_ref(), |root, focus| root.track_focus(focus))
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
                .on_action(cx.listener(|shell, action: &PickOption, _, cx| {
                    let index = action.index;
                    shell.with_session(cx, |session, cx| session.pick_option(index, cx));
                }))
                .on_action(cx.listener(|shell, action: &ShowQuestion, _, cx| {
                    let index = action.index;
                    shell.with_session(cx, |session, cx| session.show_question(index, cx));
                }))
                .on_action(cx.listener(|shell, _: &NextQuestion, _, cx| shell.with_session(cx, |session, cx| session.next_question(cx))))
                .on_action(cx.listener(|shell, _: &PreviousQuestion, _, cx| {
                    shell.with_session(cx, |session, cx| session.previous_question(cx))
                }))
                .on_action(cx.listener(|shell, _: &SubmitAnswers, _, cx| shell.with_session(cx, |session, cx| session.submit_answers(cx))))
                .on_action(cx.listener(|shell, _: &CancelQuestion, _, cx| shell.with_session(cx, |session, cx| session.cancel_question(cx))))
                .on_action(cx.listener(|shell, _: &WrapUpQuestion, _, cx| shell.with_session(cx, |session, cx| session.wrap_up(cx))))
                .on_action(cx.listener(|shell, _: &ToggleUnrestrictedPlanning, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_unrestricted_planning(cx))
                }))
                .on_action(cx.listener(|shell, _: &ConfirmUnrestrictedPlanning, _, cx| {
                    shell.with_app(cx, |app, cx| app.set_unrestricted_planning(true, cx))
                }))
                .on_action(cx.listener(|shell, _: &CancelUnrestrictedPlanning, _, cx| {
                    shell.with_app(cx, |app, cx| {
                        app.confirming_unrestricted = false;
                        cx.notify();
                    })
                }))
                .on_action(cx.listener(|shell, _: &ToggleMemory, _, cx| shell.with_app(cx, |app, cx| app.toggle_memory(cx))))
                .on_action(cx.listener(|shell, _: &ToggleAutoTitles, _, cx| shell.with_app(cx, |app, cx| app.toggle_auto_titles(cx))))
                .on_action(cx.listener(|shell, _: &ToggleTitleModels, _, cx| shell.with_app(cx, |app, cx| app.toggle_title_models(cx))))
                .on_action(cx.listener(|shell, action: &ChooseTitleModel, _, cx| {
                    let (connection, model) = (action.connection.clone(), action.model.clone());
                    shell.with_app(cx, |app, cx| app.choose_title_model(&connection, &model, cx));
                }))
                .on_action(cx.listener(|shell, _: &RenameChat, window, cx| shell.rename_on_screen(window, cx)))
                .on_action(cx.listener(|shell, action: &RenameChatAt, window, cx| {
                    let (chat, place) = (action.chat.clone(), if action.sidebar { RenamePlace::Sidebar } else { RenamePlace::Tab });
                    shell.begin_rename(&chat, place, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CancelRename, window, cx| {
                    shell.with_app(cx, |app, cx| app.finish_rename(None, None, Some(window), cx));
                }))
                .on_action(cx.listener(|shell, action: &SwitchProjectMemory, _, cx| {
                    let (key, on) = (action.key.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_project_memory(&key, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &NewMemory, window, cx| {
                    let dir = action.dir.clone();
                    shell.open_memory_editor(&dir, None, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &EditMemory, window, cx| {
                    let (dir, name) = (action.dir.clone(), action.name.clone());
                    shell.open_memory_editor(&dir, Some(&name), window, cx);
                }))
                .on_action(cx.listener(|shell, action: &ChooseMemoryKind, _, cx| {
                    let kind = action.kind;
                    shell.with_app(cx, |app, cx| app.choose_memory_kind(kind, cx));
                }))
                .on_action(cx.listener(|shell, _: &SaveMemory, window, cx| {
                    shell.with_app(cx, |app, cx| app.save_memory(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseMemoryEditor, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_memory_editor(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, action: &AskDeleteMemory, _, cx| {
                    let (dir, name) = (action.dir.clone(), action.name.clone());
                    shell.with_app(cx, |app, cx| app.ask_delete_memory(&dir, &name, cx));
                }))
                .on_action(cx.listener(|shell, action: &AskRemoveMemoryFolder, _, cx| {
                    let key = action.key.clone();
                    shell.with_app(cx, |app, cx| app.ask_remove_memory_folder(&key, cx));
                }))
                .on_action(cx.listener(|shell, _: &ConfirmMemoryDelete, _, cx| shell.with_app(cx, |app, cx| app.confirm_memory_delete(cx))))
                .on_action(cx.listener(|shell, _: &CancelMemoryDelete, _, cx| shell.with_app(cx, |app, cx| app.cancel_memory_delete(cx))))
                .on_action(cx.listener(|_, action: &RevealMemory, _, _| crate::memory_page::reveal(&action.path)))
                .on_action(cx.listener(|shell, _: &ToggleWebFetch, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_web_fetch(cx))
                }))
                .on_action(cx.listener(|shell, _: &ToggleWebSearch, _, cx| shell.with_app(cx, |app, cx| app.toggle_web_search(cx))))
                .on_action(cx.listener(|shell, _: &ToggleRouteFetch, _, cx| shell.with_app(cx, |app, cx| app.toggle_route_fetch(cx))))
                .on_action(cx.listener(|shell, action: &ChooseSearchProvider, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.choose_search_provider(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &AddSearchProvider, window, cx| {
                    let kind = action.kind;
                    let field = shell.app().map(|app| app.update(cx, |app, cx| app.add_search_provider(kind, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &EditSearchProvider, window, cx| {
                    let id = action.id.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.edit_search_provider(&id, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseSearchEditor, _, cx| shell.with_app(cx, |app, cx| app.close_search_editor(cx))))
                .on_action(cx.listener(|shell, action: &ChooseSearchKey, _, cx| {
                    let connection = action.connection.clone();
                    shell.with_app(cx, |app, cx| app.choose_search_key(connection, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseSearchMethod, _, cx| {
                    let method = action.method;
                    shell.with_app(cx, |app, cx| app.choose_search_method(method, cx));
                }))
                .on_action(cx.listener(|shell, _: &SaveSearchProvider, _, cx| shell.with_app(cx, |app, cx| app.save_search_provider(cx))))
                .on_action(cx.listener(|shell, _: &TestSearchProvider, _, cx| shell.with_app(cx, |app, cx| app.test_search_provider(cx))))
                .on_action(cx.listener(|shell, _: &AskDeleteSearchProvider, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_search_provider(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelDeleteSearchProvider, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_search_provider(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmDeleteSearchProvider, _, cx| shell.with_app(cx, |app, cx| app.confirm_delete_search_provider(cx))))
                .on_action(cx.listener(|shell, _: &ResetWebUsage, _, cx| shell.with_app(cx, |app, cx| app.reset_web_usage(cx))))
                .on_action(cx.listener(|shell, action: &AddMcpServer, window, cx| {
                    let transport = action.transport;
                    let field = shell.app().map(|app| app.update(cx, |app, cx| app.add_mcp_server(transport, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &EditMcpServer, window, cx| {
                    let id = action.id.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.edit_mcp_server(&id, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseMcpEditor, _, cx| shell.with_app(cx, |app, cx| app.close_mcp_editor(cx))))
                .on_action(cx.listener(|shell, _: &SaveMcpServer, _, cx| shell.with_app(cx, |app, cx| app.save_mcp_server(cx))))
                .on_action(cx.listener(|shell, _: &TestMcpDraft, _, cx| shell.with_app(cx, |app, cx| app.test_mcp_draft(cx))))
                .on_action(cx.listener(|shell, _: &AddMcpEntry, window, cx| {
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.add_mcp_entry(cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &RemoveMcpEntry, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.remove_mcp_entry(index, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseMcpTransport, _, cx| {
                    let transport = action.transport;
                    shell.with_app(cx, |app, cx| app.choose_mcp_transport(transport, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseMcpScope, _, cx| {
                    let only = action.only;
                    shell.with_app(cx, |app, cx| app.choose_mcp_scope(only, cx));
                }))
                .on_action(cx.listener(|shell, action: &ToggleMcpProject, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.toggle_mcp_project(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &ToggleMcpServer, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.toggle_mcp_server(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &ToggleMcpTool, _, cx| {
                    let (id, tool) = (action.id.clone(), action.tool.clone());
                    shell.with_app(cx, |app, cx| app.toggle_mcp_tool(&id, &tool, cx));
                }))
                .on_action(cx.listener(|shell, action: &ExpandMcpServer, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.expand_mcp_server(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &TestMcpServer, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.test_mcp_server(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &AskDeleteMcpServer, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.ask_delete_mcp_server(Some(id), cx));
                }))
                .on_action(cx.listener(|shell, _: &CancelDeleteMcpServer, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_mcp_server(None, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmDeleteMcpServer, _, cx| shell.with_app(cx, |app, cx| app.confirm_delete_mcp_server(cx))))
                .on_action(cx.listener(|shell, _: &ToggleUsageRecording, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_usage_recording(cx))
                }))
                .on_action(cx.listener(|shell, _: &ToggleCommitAttribution, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_commit_attribution(cx))
                }))
                .on_action(cx.listener(|_, action: &RevealUsageFolder, _, _| crate::memory_page::reveal(&action.path)))
                .on_action(cx.listener(|shell, action: &SwitchSkill, _, cx| {
                    let (key, on) = (action.key.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_skill(&key, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &SwitchSkillFolder, _, cx| {
                    let (id, on) = (action.id.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_skill_folder(&id, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &RemoveSkillFolder, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.remove_skill_folder(&id, cx));
                }))
                .on_action(cx.listener(|shell, _: &AddSkillFolder, _, cx| shell.pick_path(false, "Add", cx, AppState::add_skill_folder)))
                .on_action(cx.listener(|shell, _: &ImportSkills, _, cx| shell.pick_path(true, "Import", cx, AppState::import_skills)))
                .on_action(cx.listener(|shell, action: &OpenSkill, window, cx| {
                    let file = action.file.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_skill(&file, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &CopySkill, window, cx| {
                    let file = action.file.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.copy_skill(&file, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &NewSkill, window, cx| {
                    let field = shell.app().map(|app| app.update(cx, |app, cx| app.new_skill(cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &ChooseSkillUse, _, cx| {
                    let manual = action.manual;
                    shell.with_app(cx, |app, cx| app.choose_skill_use(manual, cx));
                }))
                .on_action(cx.listener(|shell, _: &SaveSkill, window, cx| {
                    shell.with_app(cx, |app, cx| app.save_skill(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseSkill, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_skill(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, action: &SwitchSlashCommand, _, cx| {
                    let (key, on) = (action.key.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_slash_command(&key, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &OpenSlashCommand, window, cx| {
                    let name = action.name.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_slash_command(Some(&name), cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &NewSlashCommand, window, cx| {
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_slash_command(None, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &SaveSlashCommand, window, cx| {
                    shell.with_app(cx, |app, cx| app.save_slash_command(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseSlashCommand, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_slash_command(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &AskDeleteSlashCommand, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_slash_command(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelSlashCommandDelete, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_slash_command(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmSlashCommandDelete, window, cx| {
                    shell.with_app(cx, |app, cx| app.confirm_slash_command_delete(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, action: &ChooseDelegation, _, cx| {
                    let delegation = action.delegation;
                    shell.with_app(cx, |app, cx| app.choose_delegation(delegation, cx));
                }))
                .on_action(cx.listener(|shell, _: &ToggleHelperModels, _, cx| shell.with_app(cx, |app, cx| app.toggle_helper_models(cx))))
                .on_action(cx.listener(|shell, action: &ChooseHelperModel, _, cx| {
                    let (connection, model) = (action.connection.clone(), action.model.clone());
                    shell.with_app(cx, |app, cx| app.choose_helper_model(connection, model, cx));
                }))
                .on_action(cx.listener(|shell, action: &SwitchAgent, _, cx| {
                    let (name, on) = (action.name.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_agent(&name, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &SwitchAgentFolder, _, cx| {
                    let (id, on) = (action.id.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.switch_agent_folder(&id, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &OpenAgent, window, cx| {
                    let name = action.name.clone();
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_agent(Some(&name), cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &NewAgent, window, cx| {
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_agent(None, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &DuplicateAgent, window, cx| {
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.duplicate_agent(cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, action: &ChooseAgentModel, _, cx| {
                    let helper = action.helper;
                    shell.with_app(cx, |app, cx| app.choose_agent_model(helper, cx));
                }))
                .on_action(cx.listener(|shell, _: &SaveAgent, window, cx| {
                    shell.with_app(cx, |app, cx| app.save_agent(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &CloseAgent, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_agent(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &AskDeleteAgent, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_agent(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelAgentDelete, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_agent(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmAgentDelete, window, cx| {
                    shell.with_app(cx, |app, cx| app.confirm_agent_delete(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &AskDeleteSkill, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_skill(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelSkillDelete, _, cx| shell.with_app(cx, |app, cx| app.ask_delete_skill(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmSkillDelete, window, cx| {
                    shell.with_app(cx, |app, cx| app.confirm_skill_delete(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &ShowDoneTodos, _, cx| shell.with_session(cx, |session, cx| session.show_done_todos(cx))))
                .on_action(cx.listener(|shell, _: &DiscardPlan, _, cx| shell.with_session(cx, |session, cx| session.ask_discard_plan(cx))))
                .on_action(cx.listener(|shell, _: &KeepPlan, _, cx| shell.with_session(cx, |session, cx| session.keep_plan(cx))))
                .on_action(cx.listener(|shell, _: &ConfirmDiscardPlan, _, cx| shell.with_app(cx, |app, cx| app.discard_plan(cx))))
                .on_action(cx.listener(|shell, _: &ApprovePlan, _, cx| shell.with_app(cx, |app, cx| app.approve_plan(cx))))
                .on_action(cx.listener(|shell, _: &SavePlan, _, cx| shell.with_app(cx, |app, cx| app.save_plan(cx))))
                .on_action(cx.listener(|shell, _: &CopyPlan, _, cx| {
                    if let Some(session) = shell.current_session(cx)
                        && let Some(plan) = session.read(cx).awaiting_plan()
                    {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(plan));
                        session.update(cx, |session, cx| session.plan_notice("Plan copied to the clipboard.".into(), cx));
                    }
                }))
                .on_action(cx.listener(|shell, action: &StartSuggestion, _, cx| {
                    let (id, target, mode) = (action.id.clone(), action.target, action.mode);
                    shell.with_app(cx, |app, cx| app.start_suggestion(&id, target, mode, cx));
                }))
                .on_action(cx.listener(|shell, action: &DismissSuggestion, _, cx| {
                    let (id, dismissed) = (action.id.clone(), action.dismissed);
                    shell.with_app(cx, |app, cx| app.dismiss_suggestion(&id, dismissed, cx));
                }))
                .on_action(cx.listener(|shell, action: &CopySuggestion, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.copy_suggestion(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &OpenChat, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.select(Selection::Chat(id), cx));
                }))
                .on_action(cx.listener(|shell, action: &SetAgentMode, _, cx| {
                    let mode = action.mode;
                    shell.with_app(cx, |app, cx| app.set_agent_mode(mode, cx));
                }))
                .on_action(cx.listener(|shell, _: &CycleAgentMode, _, cx| shell.with_app(cx, |app, cx| app.cycle_agent_mode(cx))))
                .on_action(cx.listener(|shell, action: &ChooseEffort, _, cx| {
                    let effort = action.effort;
                    shell.with_app(cx, |app, cx| app.choose(None, None, Some(effort), cx));
                }))
                .on_action(cx.listener(|shell, action: &SlideEffort, _, cx| {
                    let effort = action.effort;
                    shell.with_app(cx, |app, cx| {
                        app.choose(None, None, Some(effort), cx);
                        // The slider stays open as it moves.
                        app.menu = Some(Menu::Effort);
                    });
                }))
                .on_action(cx.listener(|shell, _: &ToggleTodos, _, cx| shell.with_session(cx, |session, cx| session.toggle_todos(cx))))
                .on_action(cx.listener(|shell, _: &ToggleWaiting, _, cx| shell.with_session(cx, |session, cx| session.toggle_waiting(cx))))
                .on_action(cx.listener(|shell, _: &ToggleTasks, _, cx| shell.with_session(cx, |session, cx| session.toggle_tasks(cx))))
                .on_action(cx.listener(|shell, _: &MoveToBackground, _, cx| {
                    shell.with_session(cx, |session, cx| {
                        session.background_command(cx);
                    })
                }))
                .on_action(cx.listener(|shell, action: &StopTask, _, cx| {
                    let id = action.id.to_string();
                    shell.with_session(cx, |session, cx| session.stop_task(id, cx));
                }))
                .on_action(cx.listener(|shell, _: &ToggleTasksPane, window, cx| shell.toggle_pane(PaneKind::Tasks, window, cx)))
                .on_action(cx.listener(|shell, _: &ToggleAgentsPane, window, cx| shell.toggle_pane(PaneKind::Agents, window, cx)))
                .on_action(cx.listener(|shell, action: &ShowAgent, _, cx| {
                    let id = action.id.to_string();
                    shell.change_layout(cx, |layout| layout.open(PaneKind::Agents));
                    shell.with_session(cx, |session, cx| session.panes.select_agent(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &StopAgent, _, cx| {
                    let id = action.id.to_string();
                    shell.with_session(cx, |session, cx| session.stop_agent(&id, cx));
                }))
                .on_action(cx.listener(|shell, action: &ShowTask, _, cx| {
                    let id = action.id.to_string();
                    shell.change_layout(cx, |layout| layout.open(PaneKind::Tasks));
                    // The pane shows what the list did, so the list closes.
                    shell.with_session(cx, |session, cx| {
                        session.tasks_open = false;
                        session.panes.select_task(&id, cx);
                    });
                }))
                .on_action(cx.listener(|shell, _: &SaveConnection, _, cx| shell.with_app(cx, |app, cx| app.save_connection(cx))))
                .on_action(cx.listener(|shell, _: &OpenConnectionForm, window, cx| {
                    // The editor opens ready to type its name.
                    let Some(app) = shell.app() else {
                        return;
                    };
                    let name = app.update(cx, |app, cx| {
                        app.open_connection_form(cx);
                        app.form.name.clone()
                    });
                    let handle = name.read(cx).focus_handle(cx);
                    window.defer(cx, move |window, cx| window.focus(&handle, cx));
                }))
                .on_action(cx.listener(|shell, action: &EditConnection, window, cx| {
                    let Some(app) = shell.app() else {
                        return;
                    };
                    let id = action.id.clone();
                    let name = app.update(cx, |app, cx| {
                        app.edit_connection(&id, cx);
                        app.form.name.clone()
                    });
                    let handle = name.read(cx).focus_handle(cx);
                    window.defer(cx, move |window, cx| window.focus(&handle, cx));
                }))
                .on_action(cx.listener(|shell, _: &CloseConnectionForm, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_connection_form(cx));
                    // The editor's fields are gone; the shell takes the keys again.
                    if let Some(focus) = shell.focus.clone() {
                        window.defer(cx, move |window, cx| window.focus(&focus, cx));
                    }
                }))
                .on_action(cx.listener(|shell, _: &DiscardConnection, _, cx| {
                    shell.with_app(cx, |app, cx| app.discard_connection_changes(cx));
                }))
                .on_action(cx.listener(|shell, _: &ToggleConnectionKey, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_connection_key(cx));
                }))
                .on_action(cx.listener(|shell, _: &AddModelRow, window, cx| {
                    let Some(app) = shell.app() else {
                        return;
                    };
                    let field = app.update(cx, |app, cx| {
                        app.add_model_row(cx);
                        app.form.models.last().map(|row| row.id.clone())
                    });
                    if let Some(field) = field {
                        let handle = field.read(cx).focus_handle(cx);
                        window.defer(cx, move |window, cx| window.focus(&handle, cx));
                    }
                }))
                .on_action(cx.listener(|shell, action: &ToggleModelReasoning, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.toggle_model_reasoning(index, cx));
                }))
                .on_action(cx.listener(|shell, action: &ToggleModelEffort, _, cx| {
                    let (index, effort) = (action.index, action.effort);
                    shell.with_app(cx, |app, cx| app.toggle_model_effort(index, effort, cx));
                }))
                .on_action(cx.listener(|shell, action: &RemoveModelRow, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.remove_model_row(index, cx));
                }))
                .on_action(cx.listener(|shell, action: &SwitchConnection, _, cx| {
                    let (id, on) = (action.id.clone(), action.on);
                    shell.with_app(cx, |app, cx| app.set_connection_enabled(&id, on, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseFormat, _, cx| {
                    let format = action.format;
                    shell.with_app(cx, |app, cx| app.set_format(format, cx));
                }))
                .on_action(cx.listener(Self::remove_connection))
                .on_action(cx.listener(Self::open_settings_section))
                .on_action(cx.listener(|shell, action: &ChooseMode, _, cx| {
                    let mode = action.mode;
                    shell.with_app(cx, |app, cx| app.set_appearance(|appearance| appearance.mode = mode, cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseTheme, _, cx| {
                    let theme = action.theme;
                    shell.with_app(cx, |app, cx| app.set_appearance(|appearance| appearance.choose_theme(theme), cx));
                }))
                .on_action(cx.listener(Self::choose_backdrop))
                .on_action(cx.listener(|shell, action: &ShowPicture, _, cx| {
                    let file = action.file.clone();
                    shell.with_app(cx, |app, cx| app.set_appearance(|appearance| appearance.show(file.as_deref()), cx));
                }))
                .on_action(cx.listener(|shell, action: &RemovePicture, _, cx| {
                    let file = action.file.clone();
                    shell.with_app(cx, |app, cx| app.set_appearance(|appearance| appearance.remove(&file), cx));
                }))
                .on_action(cx.listener(|shell, action: &ChooseDim, _, cx| {
                    let dim = action.dim;
                    shell.change_backdrop(cx, |backdrop| backdrop.dim = dim);
                }))
                .on_action(cx.listener(|shell, action: &ChooseBlur, _, cx| {
                    let blur = action.blur;
                    shell.change_backdrop(cx, |backdrop| backdrop.blur = blur);
                }))
                .on_action(cx.listener(|shell, action: &ChooseTexture, _, cx| {
                    let texture = action.texture;
                    shell.change_backdrop(cx, |backdrop| backdrop.texture = texture);
                }))
                .on_action(cx.listener(|shell, action: &ChooseFocus, _, cx| {
                    let (across, down) = (action.across.min(2) as f32 / 2., action.down.min(2) as f32 / 2.);
                    shell.change_backdrop(cx, |backdrop| (backdrop.focus_x, backdrop.focus_y) = (across, down));
                }))
                .on_action(cx.listener(|shell, action: &PickCommand, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.pick_command(index, cx));
                }))
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
                .on_action(cx.listener(|shell, action: &TriggerProjectCommand, window, cx| {
                    let kind = action.kind;
                    if let Some(app) = shell.app()
                        && let Some(input) = app.update(cx, |app, cx| app.trigger_command(kind, cx))
                    {
                        let handle = input.read(cx).focus_handle(cx);
                        window.defer(cx, move |window, cx| window.focus(&handle, cx));
                    }
                }))
                .on_action(cx.listener(|shell, action: &OpenCommandEditor, window, cx| {
                    let kind = action.kind;
                    if let Some(app) = shell.app() {
                        match app.update(cx, |app, cx| app.open_command_editor(kind, cx)) {
                            Some(input) => {
                                let handle = input.read(cx).focus_handle(cx);
                                window.defer(cx, move |window, cx| window.focus(&handle, cx));
                            }
                            None => shell.focus_composer(window, cx),
                        }
                    }
                }))
                .on_action(cx.listener(|shell, _: &CloseCommandEditor, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_command_editor(cx));
                    shell.focus_composer(window, cx);
                }))
                .on_action(cx.listener(|shell, action: &SaveCommandEditor, window, cx| {
                    let run_after = action.run_after;
                    if let Some(app) = shell.app()
                        && app.update(cx, |app, cx| app.save_command_editor(run_after, cx))
                    {
                        shell.focus_composer(window, cx);
                    }
                }))
                .on_action(cx.listener(|shell, action: &ShowCommandOutput, window, cx| {
                    let kind = action.kind;
                    shell.with_app(cx, |app, cx| app.show_command_output(kind, cx));
                    shell.focus_composer(window, cx);
                }))
                .on_action(cx.listener(|shell, action: &ClearCommandOutput, _, cx| {
                    let kind = action.kind;
                    shell.with_app(cx, |app, cx| app.clear_command_output(kind, cx));
                }))
                .on_action(cx.listener(|shell, action: &StopCommand, _, cx| {
                    let kind = action.kind;
                    shell.with_app(cx, |app, cx| app.stop_command(kind, cx));
                }))
                .on_drag_move(cx.listener(Self::resize))
                .on_drag_move(cx.listener(|shell, event: &DragMoveEvent<DraggedTab>, _, cx| {
                    let pane = event.drag(cx).pane;
                    if shell.dragging != Some(pane) {
                        shell.dragging = Some(pane);
                        cx.notify();
                    }
                }))
                .relative()
                .size_full()
                .flex()
                .bg(if over_picture { theme.window } else { theme.sidebar })
                .text_color(theme.text)
                .typography(Type::Ui)
                .children(behind)
                .children(sidebar)
                .child(main)
                .children(lights),
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
    pub look: TitleLook,
    pub active: bool,
    pub state: ChatState,
}

/// What the title strip shows: the open chats as tabs, and a "New chat" tab while writing one.
#[derive(Clone, Default)]
pub struct StripData {
    pub tabs: Vec<TabData>,
    pub new_chat: bool,
    pub commands: Option<[CommandButtonData; 2]>,
    pub command_editor: Option<CommandEditorData>,
}

/// The sidebar, screen and title strip for the app's current state.
fn live_view(app: &AppState, cx: &App) -> (SidebarData, Screen, StripData) {
    let store = app.store();
    let still = cx.reduce_motion();
    // How a chat's title is drawn in `place`: the rename field where it is being renamed, the
    // model's title arriving, or plain.
    let look = |chat: &str, place: RenamePlace| match &app.renaming {
        Some(renaming) if renaming.chat == chat && renaming.place == place => TitleLook::Editing(renaming.field.clone()),
        _ => app.title_swaps.get(chat).and_then(|swap| swap.look(still)).map_or(TitleLook::Plain, TitleLook::Swap),
    };
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
                    look: look(&chat.id, RenamePlace::Sidebar),
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
        Selection::Settings => {
            let appearance = store.appearance();
            Screen::Settings(Box::new(SettingsData {
                section: app.settings_section,
                providers: ProvidersData::new(store.connections(), |id| store.has_key(id), &app.form, cx),
                appearance: AppearanceData {
                    mode: appearance.mode,
                    theme: appearance.theme,
                    backdrop: app.backdrop(),
                    pictures: app.pictures(),
                    importing: app.importing,
                    notice: app.appearance_notice.clone(),
                },
                tools: crate::tools_page::ToolsData {
                    unrestricted_planning: store.planning_unrestricted(),
                    confirming: app.confirming_unrestricted,
                    web_fetch: store.web_fetch_enabled(),
                },
                memory: app.memory.data(cx),
                skills: app.skills.data(cx),
                commands: app.templates.data(cx),
                subagents: app.subagents.data(
                    store.agents_config(),
                    store.connections(),
                    crate::titles_card::TitlesData::new(store.auto_title(), store.connections(), app.title_models_open, app.title_error.clone()),
                    cx,
                ),
                git: crate::git_page::GitData {
                    commit_attribution: store.git_config().commit_attribution,
                    error: app.git_error.clone(),
                },
                integrations: crate::integrations_page::IntegrationsData { status: store.usage().status() },
                web: app.web_data(cx),
                mcp: app.mcp_data(),
            }))
        }
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
                        ultra_effort: is_ultra(resolved.as_ref()),
                        mode: app.new_chat_mode,
                        key: "new-chat".into(),
                        meter: None,
                        working: false,
                        waiting: Vec::new(),
                        roomy: true,
                        menu: menu_data(app, resolved.as_ref()),
                        slash: app.slash_data(None),
                        hint: app.command_hint(None, cx),
                        question: None,
                        todos: None,
                        activity: None,
                        todos_open: false,
                        waiting_open: false,
                        tasks: Vec::new(),
                        tasks_open: false,
                        foreground: false,
                        agents: 0,
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
                    ultra_effort: is_ultra(resolved.as_ref()),
                    mode: chat.mode,
                    key: chat.id.clone().into(),
                    meter,
                    working: state.running(),
                    waiting,
                    roomy: false,
                    menu: menu_data(app, resolved.as_ref()),
                    slash: app.slash_data(Some(id)),
                    hint: app.command_hint(Some(id), cx),
                    question: state.asking.as_ref().map(|asking| crate::question::QuestionData {
                        questions: asking.questions.clone(),
                        tab: asking.tab,
                        drafts: asking.drafts(cx),
                        wrap_up: asking.wrap_up,
                        other: Input::Field(asking.others[asking.tab.min(asking.others.len() - 1)].clone()),
                        focus: Some(asking.focus.clone()),
                        nods: asking.nods,
                    }),
                    todos: {
                        let view = crate::todo_panel::view(state.todos(), &state.todo_faded, state.todo_show_done);
                        (view.total > 0).then(|| crate::todo_panel::TodoData { view, busy: state.running() })
                    },
                    activity: state.running().then(|| state.activity()).flatten(),
                    todos_open: state.todos_open,
                    waiting_open: state.waiting_open,
                    tasks: task_lines(&state.tasks),
                    tasks_open: state.tasks_open,
                    foreground: state.tasks.has_foreground() || state.agents.active().iter().any(|info| !info.background),
                    agents: state.agents.active().len(),
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
                    look: look(id, RenamePlace::Tab),
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
        commands: app.command_buttons(),
        command_editor: app.command_editor_data(cx),
    };
    (sidebar, screen, strip)
}

/// The open menu's choices, with the one in use marked.
fn menu_data(app: &AppState, resolved: Option<&wack_core::Resolved>) -> Option<MenuData> {
    let kind = app.menu?;
    let groups = match kind {
        Menu::Model => app
            .store()
            .enabled_connections()
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
        Menu::Effort => match resolved {
            Some(resolved) => effort_groups(&resolved.model.choices(), resolved.effort),
            None => effort_groups(&[wack_core::Reasoning::Off], wack_core::Reasoning::Off),
        },
    };
    Some(MenuData { kind, groups })
}

/// The effort menu: the levels the model lists in Settings › Providers (and Ultra above them),
/// with `current` ticked.
pub fn effort_groups(choices: &[wack_core::Reasoning], current: wack_core::Reasoning) -> Vec<MenuGroup> {
    use wack_core::Reasoning;
    let detail = |effort: Reasoning| match effort {
        Reasoning::Off => Some("no reasoning"),
        Reasoning::Ultra => Some("the most thorough work"),
        _ => None,
    };
    vec![MenuGroup {
        title: Some("Reasoning effort".into()),
        items: choices
            .iter()
            .map(|&effort| MenuItem {
                label: effort.label().into(),
                detail: detail(effort).map(Into::into),
                selected: current == effort,
                choice: Choice::Effort(Some(effort)),
            })
            .collect(),
    }]
}

/// The commands running in a chat's background, then the three that ended last.
pub(crate) fn task_lines(tasks: &wack_core::tasks::Tasks) -> Vec<crate::composer::TaskLine> {
    let now = wack_core::tasks::now_millis();
    let list: Vec<_> = tasks.list().into_iter().filter(|info| info.in_background()).collect();
    let line = |info: &wack_core::tasks::TaskInfo| crate::composer::TaskLine {
        id: info.id.clone().into(),
        label: info.label().to_string().into(),
        words: crate::panes::tasks::status_words(info, now).into(),
        status: info.status.clone(),
        last: crate::panes::tasks::plain(&info.last_line).into(),
    };
    let mut lines: Vec<_> = list.iter().filter(|info| info.status.running()).map(line).collect();
    lines.extend(list.iter().rev().filter(|info| !info.status.running()).take(3).map(line));
    lines
}

fn first_line(text: &str) -> SharedString {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim().to_string().into()
}

/// The words the composer shows for the model and its effort.
fn model_words(resolved: Option<&wack_core::Resolved>) -> (SharedString, Option<SharedString>) {
    match resolved {
        Some(resolved) => (
            model_name(&resolved.model.id).into(),
            Some(resolved.effort.label().into()),
        ),
        None => ("No model".into(), None),
    }
}

fn is_ultra(resolved: Option<&wack_core::Resolved>) -> bool {
    resolved.is_some_and(|resolved| resolved.effort.is_ultra())
}

/// `zai-org/GLM-5.3-Flash` reads as `GLM-5.3-Flash`.
pub fn model_name(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

/// "14%" of the window, or "41.2k tokens" when the model's window is unknown (the composer
/// says what it measures in a tooltip). Compact is offered
/// once there is enough to be worth summarising.
fn meter(tokens: u64, window: Option<u64>, has_messages: bool) -> Meter {
    match window.filter(|window| *window > 0) {
        Some(window) => {
            let percent = tokens as f32 / window as f32 * 100.;
            let label = if percent > 0. && percent < 1. {
                "<1%".to_string()
            } else {
                format!("{}%", percent.round().min(100.))
            };
            Meter { label: label.into(), high: percent >= 80., can_compact: has_messages && percent >= 25. }
        }
        None => Meter {
            label: format!("{} tokens", crate::work::count(tokens)).into(),
            high: false,
            can_compact: has_messages && tokens >= 50_000,
        },
    }
}

fn drag_strip(name: &'static str, height: Pixels, content: impl IntoElement) -> impl IntoElement {
    div()
        .id(name)
        .flex_none()
        .w_full()
        .h(height)
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
fn kicker(text: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Icon::Duck.view(px(15.), theme.text_soft))
        .child(div().typography(Type::Label).text_color(theme.text_dim).child(text))
}

/// `child` as it is, or over the clear picture (`ground`) on a frosted pill as thick as the panes'
/// glass and dim together, so its ink reads as it would on them.
fn on_ground(child: gpui::Div, ground: Option<Hsla>, theme: &Theme) -> gpui::AnyElement {
    match ground {
        Some(ground) => glass(11., GLASS_BLUR, ground, theme.border, child.px(px(12.)).py(px(3.))).into_any_element(),
        None => child.into_any_element(),
    }
}

/// A screen's display line. Over the clear picture (`halo`) it is ringed by faint copies of
/// itself in the window's colour, a halo that keeps it legible on any part of the picture without
/// laying anything over it (GPUI draws no text shadow).
fn display_line(text: SharedString, halo: bool, theme: &Theme) -> gpui::Div {
    // Rings of copies, fading outwards, so the halo has no edge of its own.
    const RINGS: [(f32, f32); 3] = [(0.9, 0.32), (1.9, 0.18), (3.0, 0.08)];
    const STEPS: usize = 12;
    let copies = halo.then(|| {
        RINGS.iter().flat_map(|&(radius, alpha)| {
            (0..STEPS).map(move |step| {
                let angle = step as f32 * std::f32::consts::TAU / STEPS as f32;
                (radius * angle.cos(), radius * angle.sin(), alpha)
            })
        })
    });
    div()
        .relative()
        .typography(Type::Display)
        .children(copies.into_iter().flatten().map(|(dx, dy, alpha)| {
            div()
                .absolute()
                .left(px(dx))
                .top(px(dy))
                .size_full()
                .text_color(Hsla { a: alpha, ..theme.window })
                .child(text.clone())
        }))
        .child(div().relative().text_color(theme.text).child(text))
}

/// A problem under a screen's block, on `ground` over the clear picture ([`on_ground`]).
fn notice_line(notice: Option<SharedString>, ground: Option<Hsla>, theme: &Theme) -> impl IntoElement {
    div().when_some(notice, |line, notice| {
        line.pt(px(14.)).child(on_ground(div().typography(Type::Ui).text_color(theme.danger).child(notice), ground, theme))
    })
}

/// A block in the middle of the column, a little above centre.
fn centred(block: impl IntoElement) -> gpui::Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(32.))
        .child(block)
        .child(div().flex_none().h(relative(0.08)))
}

/// The first-launch screen. Over the clear picture its smaller words sit on frosted pills
/// (`ground`, [`on_ground`]); the display line stands on the picture.
fn welcome(notice: Option<SharedString>, ground: Option<Hsla>, theme: &Theme) -> impl IntoElement {
    centred(
        div()
            .relative()
            .max_w(px(WELCOME_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .child(on_ground(kicker("Welcome to WackCode", theme), ground, theme))
            .child(div().pt(px(10.)).child(display_line("Open a folder to begin.".into(), ground.is_some(), theme)))
            .child(div().pt(px(14.)).child(on_ground(
                div().text_center().typography(Type::Body).text_color(theme.text_soft).child(
                    "WackCode works inside a folder on your Mac: it reads, edits and runs \
                     what is there. Add one, then start a chat in it.",
                ),
                ground,
                theme,
            )))
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
                            .bg(theme.accent_fill())
                            .hover(|button| button.bg(theme.accent_fill_hover()))
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(AddFolder), cx))
                            .child(Icon::Folder.view(px(15.), theme.on_accent))
                            .child(div().typography(Type::UiStrong).text_color(theme.on_accent).child("Add folder…"))
                            .tag("add-folder-button"),
                    )
                    .child(on_ground(div().typography(Type::Small).text_color(theme.text_faint).child("⌘O"), ground, theme)),
            )
            .child(notice_line(notice, None, theme)),
    )
}

/// The new-chat screen. Over the clear picture its smaller words sit on frosted pills (`ground`,
/// [`on_ground`]); the project's name stands on the picture.
fn new_chat(
    project: ProjectInfo,
    composer: ComposerData,
    strip: &StripData,
    viewport: Size<Pixels>,
    notice: Option<SharedString>,
    ground: Option<Hsla>,
    theme: &Theme,
) -> impl IntoElement {
    let hint = |keys: &'static str, what: &'static str| {
        div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.text_dim).child(keys))
            .child(div().text_color(theme.text_faint).child(what))
    };
    centred(
        div()
            .relative()
            .w_full()
            .max_w(px(NEW_CHAT_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .child(on_ground(kicker("New chat in", theme), ground, theme))
            .child(div().pt(px(6.)).child(display_line(project.name.clone(), false, theme)))
            // Held to the block's width, so a long path ends in an ellipsis instead of spilling out.
            .child(div().max_w_full().pt(px(8.)).child(on_ground(project_line(&project, theme).max_w_full(), ground, theme)))
            .when_some(strip.commands.as_ref(), |block, commands| {
                block.child(
                    div()
                        .pt(px(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .children(commands.iter().map(|button| {
                            run_button::split_button(button, strip.command_editor.as_ref(), false, ground, viewport, theme)
                        })),
                )
            })
            .child(div().w_full().pt(px(22.)).child(Composer::new(composer)))
            .child(
                div().pt(px(12.)).child(on_ground(
                    div().flex().gap(px(20.)).typography(Type::Small).child(hint("↩", "send")).child(hint("⇧↩", "new line")),
                    ground,
                    theme,
                )),
            )
            .child(notice_line(notice, ground, theme)),
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
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .hover(|button| button.bg(theme.hover))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSidebar), cx))
        .child(Icon::Sidebar.view(px(15.), theme.text_dim))
        .tag("toggle-sidebar")
}

/// The open chats as pills across the strip, the one in front lit and wearing its close button,
/// then a round button for a new chat. On a chat, its project and the
/// pane words sit at the right. Over a picture everything sits on `ground`, the panes' glass, so
/// it reads on the frame.
fn tab_strip(
    strip: &StripData,
    chat: Option<(Option<ProjectInfo>, PaneWords)>,
    compact_commands: bool,
    viewport: Size<Pixels>,
    ground: Option<Hsla>,
    theme: &Theme,
) -> gpui::Div {
    let tab = |id: Option<SharedString>, title: SharedString, look: TitleLook, active: bool, state: ChatState| {
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
            .flex_none()
            .min_w(px(64.))
            .max_w(px(220.))
            .h(px(28.))
            .pl(px(12.))
            .pr(px(5.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .when_some(ground, |tab, ground| tab.bg(ground))
            .when(active, |tab| tab.bg(ground.unwrap_or(theme.window)).border_1().border_color(theme.border_strong))
            .when(!active, |tab| tab.hover(|tab| tab.bg(theme.hover)))
            .when_some(select, |tab, select| {
                tab.on_click(move |event, window, cx| {
                    let action: Box<dyn gpui::Action> = if event.click_count() == 2 {
                        Box::new(RenameChatAt { chat: select.id.clone(), sidebar: false })
                    } else {
                        Box::new(select.clone())
                    };
                    window.dispatch_action(action, cx);
                })
            })
            // The same marks as the sidebar: the paddling duck while it works, a dot once it has
            // finished unseen.
            .map(|tab| match state {
                ChatState::Working => tab.child(motion::paddling(div().flex_none().child(Icon::Duck.view(px(12.), theme.accent_text)))),
                ChatState::Unread => tab.child(div().flex_none().size(px(6.)).rounded_full().bg(theme.accent_text)),
                ChatState::Idle => tab,
            })
            .child({
                let colour = if active { theme.text } else { theme.text_dim };
                let words = |text: gpui::AnyElement| {
                    div()
                        .min_w_0()
                        .truncate()
                        .typography(if active { Type::UiStrong } else { Type::Ui })
                        .text_color(colour)
                        .when(!active, |title| title.group_hover(group.clone(), |title| title.text_color(theme.text_soft)))
                        .child(text)
                };
                match look {
                    TitleLook::Editing(field) => rename_field(field, theme).into_any_element(),
                    // The model's title: the old one leaves to the left as the new one arrives from
                    // the right, then the accent glints across it.
                    TitleLook::Swap(swap) => {
                        let (offset, opacity) = swap.tab.new;
                        let arriving = match swap.tab.glint {
                            Some(head) => motion::glint_text(title, head, colour, theme.accent).into_any_element(),
                            None => title.into_any_element(),
                        };
                        div()
                            .min_w_0()
                            .relative()
                            .overflow_hidden()
                            .child(words(arriving).relative().left(px(offset)).opacity(opacity))
                            .when_some(swap.tab.old, |slot, (offset, opacity)| {
                                slot.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(px(offset))
                                        .w(px(220.))
                                        .child(words(swap.from.into_any_element()).opacity(opacity)),
                                )
                            })
                            .into_any_element()
                    }
                    TitleLook::Plain => words(title.into_any_element()).into_any_element(),
                }
            })
            .child(
                div()
                    .id(SharedString::from(format!("close-{}", id.as_deref().unwrap_or("new"))))
                    .flex_none()
                    .size(px(18.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!active, |close| close.invisible().group_hover(group.clone(), |close| close.visible()))
                    .hover(|close| close.bg(theme.hover))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(close.boxed_clone(), cx);
                    })
                    .child(Icon::Close.view(px(11.), theme.text_dim)),
            )
            .tag(if active { "chat-tab-active" } else { "chat-tab" })
    };
    let mut tabs: Vec<_> =
        strip.tabs.iter().map(|data| tab(Some(data.id.clone()), data.title.clone(), data.look.clone(), data.active, data.state)).collect();
    if strip.new_chat {
        tabs.push(tab(None, "New chat".into(), TitleLook::Plain, true, ChatState::Idle));
    }
    // While a new chat is open its own tab stands for this. Outside the clipped row, so it shows
    // however many tabs there are.
    let new_tab = (!strip.new_chat).then(|| {
        div()
            .id("new-tab")
            .occlude()
            .flex_none()
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when_some(ground, |button, ground| button.bg(ground))
            .hover(|button| button.bg(theme.hover))
            .on_click(|_, window, cx| window.dispatch_action(Box::new(NewChat), cx))
            .child(Icon::Plus.view(px(14.), theme.text_dim))
            .tag("new-tab")
    });
    let word = |id: &'static str, label: &'static str, count: Option<usize>, on: bool, action: Box<dyn gpui::Action>| {
        div()
            .id(id)
            .occlude()
            .relative()
            .flex_none()
            .h(px(22.))
            .px(px(9.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(5.))
            .typography(if on { Type::UiStrong } else { Type::Ui })
            .text_color(if on { theme.text } else { theme.text_dim })
            .when(on, |word| word.bg(theme.hover))
            .hover(|word| word.text_color(theme.text))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .child(label)
            .when_some(count.filter(|count| *count > 0), |word, count| {
                word.child(div().typography(Type::Small).text_color(theme.text_faint).child(count.to_string()))
            })
            .tag(id)
    };
    let right = chat.map(|(project, words)| {
        div()
            .flex_none()
            .h(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .pl(px(6.))
            .pr(px(4.))
            .when_some(ground, |right, ground| right.bg(ground))
            .when_some(project, |right, project| {
                right.child(
                    div()
                        .max_w(px(200.))
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
            .when_some(strip.commands.as_ref(), |right, commands| {
                right
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .children(commands.iter().map(|button| {
                                run_button::split_button(button, strip.command_editor.as_ref(), compact_commands, None, viewport, theme)
                            })),
                    )
                    .child(div().flex_none().w(px(1.)).h(px(12.)).mx(px(4.)).bg(theme.border))
            })
            .child(word("toggle-changes", "Changes", words.changed_files, words.changes, Box::new(ToggleChanges)))
            .child(word("toggle-terminal", "Terminal", None, words.terminal, Box::new(ToggleTerminal)))
    });
    div()
        .size_full()
        .pl(px(4.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_initial().min_w_0().h_full().flex().items_center().gap(px(4.)).overflow_hidden().children(tabs))
        .children(new_tab)
        .child(div().flex_1())
        .children(right)
}

/// The space kept either side of the centred transcript.
fn gutter(narrow: bool) -> Pixels {
    if narrow { px(24.) } else { px(48.) }
}

/// One transcript item in the column: centred in the space beside the panes, no wider than a
/// reading line.
fn column(child: impl IntoElement, narrow: bool) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .justify_center()
        .px(gutter(narrow))
        .child(div().w_full().max_w(TRANSCRIPT_MAX_WIDTH).child(child))
}

/// How much of the transcript stays in view however much the composer column holds.
const TRANSCRIPT_KEPT: f32 = 72.;

fn chat(body: Body, composer: ComposerData, narrow: bool, height: f32) -> impl IntoElement {
    // An open question may take more of the chat: answering it is what the chat waits on.
    let kept = if composer.question.is_some() { TRANSCRIPT_KEPT / 3. } else { TRANSCRIPT_KEPT };
    let transcript = match body {
        Body::Items(items) => div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .pb(DOCK_FADE)
            .child(column(Transcript::new(items), narrow))
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
                column(transcript::item(current, previous, theme, &handlers), narrow).into_any_element()
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
        // Text fades out as it scrolls under the title strip and the composer, rather than
        // being cut off; the fade is the text's own, so a backdrop shows through untouched.
        .child(div().flex_1().min_h_0().child(faded(transcript, TOP_FADE, DOCK_FADE)))
        .child(
            div()
                .flex_none()
                // Reaches up over the transcript's foot, where its text has faded out.
                .mt(-DOCK_FADE)
                .flex()
                .justify_center()
                // 16 points wider each side than the column, so its text lines up with the
                // transcript's.
                .px(gutter(narrow) - px(16.))
                .pt(DOCK_FADE)
                .pb(px(18.))
                .child(
                    div()
                        .w_full()
                        .max_w(TRANSCRIPT_MAX_WIDTH + px(32.))
                        // The column (question, waiting messages, composer) never takes the whole
                        // chat: what it holds scrolls inside itself instead.
                        .max_h(px((height - kept - f32::from(DOCK_FADE) - 18.).max(140.)))
                        .flex()
                        .flex_col()
                        .child(Composer::new(composer)),
                ),
        )
}

fn failed(message: SharedString, theme: &Theme) -> impl IntoElement {
    centred(
        div()
            .max_w(px(WELCOME_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .child(kicker("WackCode could not start", theme))
            .child(div().pt(px(10.)).text_center().typography(Type::Body).text_color(theme.text_soft).child(message)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_reads_as_a_share_and_offers_compact_from_a_quarter() {
        let small = meter(4_000, Some(1_000_000), true);
        assert_eq!(small.label.as_ref(), "<1%");
        assert!(!small.can_compact && !small.high);

        let quarter = meter(250_000, Some(1_000_000), true);
        assert_eq!(quarter.label.as_ref(), "25%");
        assert!(quarter.can_compact);
        assert!(!meter(250_000, Some(1_000_000), false).can_compact, "nothing to compact in an empty chat");

        assert!(meter(850_000, Some(1_000_000), true).high);
        assert_eq!(meter(0, Some(1_000), true).label.as_ref(), "0%");
        assert_eq!(meter(2_000_000, Some(1_000_000), true).label.as_ref(), "100%");
    }

    #[test]
    fn without_a_window_the_meter_counts_tokens() {
        let unknown = meter(41_200, None, true);
        assert_eq!(unknown.label.as_ref(), "41.2k tokens");
        assert!(!unknown.can_compact);
        assert!(meter(60_000, Some(0), true).can_compact, "a zero window counts as unknown");
    }

    #[test]
    fn model_names_drop_the_organisation() {
        assert_eq!(model_name("zai-org/GLM-5.3-Flash"), "GLM-5.3-Flash");
        assert_eq!(model_name("claude-opus-5-5"), "claude-opus-5-5");
    }
}
