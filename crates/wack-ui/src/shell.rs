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
//! transcript and its docks, a new chat, Settings). The title strip runs across the frame above
//! the stage, holding the chat tabs in a glass tray (`tab_strip`): the open tab is marked by a
//! bead that slides to whichever tab opens next (`motion::Bead`). A chat's stage opens with its
//! header (`chat_header`): the title, the project and branch, Build and Run, and the toggles
//! that show and hide its panes, as version 2's chat header did.
//!
//! Every edge keeps [`FRAME_GUTTER`]: the solver works in the frame's inner space
//! ([`solver_space`]), and anything that turns the mouse into a layout position goes through
//! [`to_solver`].
//!
//! Over an image backdrop the picture fills the window, blurred and dimmed as the user chose, and
//! both panes are frosted glass over it (`glass.rs`); text outside them (the tabs) sits on the
//! same glass. The new-chat and welcome screens show the picture clear across the whole
//! stage, with glass only behind the heading (`backdrop.rs`); opening a chat eases it away to the
//! glass, and New chat eases it back (`clear`, `motion::Ease`). Without a picture the frame is the
//! sidebar's colour and the stage is the lit window.
//!
//! The shell draws from plain data ([`SidebarData`], [`Screen`]). In the app that data is read
//! from [`AppState`] on every frame and the shell handles the actions its views dispatch; a
//! gallery scene hands it fixed data instead and handles nothing.

use std::{cell::RefCell, rc::Rc};

use wack_core::{AgentMode, ChatKind};
use gpui::{AnimationExt as _, 
    App, Context, DragMoveEvent, ElementId, Entity, FocusHandle, Focusable, Hsla, ListState, MouseButton, PathPromptOptions, Pixels,
    PromptLevel, SharedString, Size, Subscription, Window, WindowControlArea, div, list,
    point, prelude::*, px, relative,
};

use crate::{
    actions::{
        self, ActivatePane, AddFolder, Back, ChooseBackdrop, ChooseEffort, SlideEffort, ToggleTodos, PreviewCrop, CommitCrop, ChooseZoom, ResetCrop, ClearCommandOutput, CloseCommandEditor, CycleAgentMode, SetAgentMode, PickOption, ShowQuestion, NextQuestion, PreviousQuestion, SubmitAnswers, CancelQuestion, WrapUpQuestion, ShowDoneTodos, DiscardPlan, KeepPlan, ConfirmDiscardPlan, ApprovePlan, SavePlan, CopyPlan, ToggleUnrestrictedPlanning, ToggleWebFetch, ToggleWebSearch, ToggleRouteFetch, CloseSearchEditor, SaveSearchProvider, TestSearchProvider, AskDeleteSearchProvider, ConfirmDeleteSearchProvider, CancelDeleteSearchProvider, ResetWebUsage, ChooseSearchProvider, AddSearchProvider, EditSearchProvider, ChooseSearchKey, ChooseSearchMethod, AddMcpServer, EditMcpServer, ToggleMcpServer, ToggleMcpTool, ExpandMcpServer, TestMcpServer, AskDeleteMcpServer, RemoveMcpEntry, ChooseMcpTransport, ChooseMcpScope, ToggleMcpProject, CloseMcpEditor, SaveMcpServer, TestMcpDraft, AddMcpEntry, ConfirmDeleteMcpServer, CancelDeleteMcpServer, ConfirmUnrestrictedPlanning, CancelUnrestrictedPlanning, ToggleMemory, SwitchProjectMemory, NewMemory, EditMemory, ChooseMemoryKind, SaveMemory, CloseMemoryEditor, AskDeleteMemory, AskRemoveMemoryFolder, ConfirmMemoryDelete, CancelMemoryDelete, RevealMemory, ToggleCommitAttribution, ToggleUsageRecording, RevealUsageFolder,
        ChooseBlur, ChooseDim, ChooseFormat, OpenCommandEditor, OpenConnectionForm, CloseConnectionForm, SwitchConnection, ChooseMode, ChooseModel, ChooseTexture, ChooseTheme, CloseChatTab, CloseMenu, ClosePane,
        ClosePaneId, CloseSettings, CloseTab, Compact, DeleteChat, DiscardConnection, EditConnection, ToggleConnectionKey, EqualizeSplit, FocusNext, FocusPrevious, MovePane, AddModelRow, RemoveModelRow, SaveCommandEditor, SaveConnection,
        NewChat, NewChatIn, SelectDraft, CloseDraft, NewTerminal, NewTerminalWith, TogglePaneMenu, ClosePaneMenu, HideSidePanel, ToggleSidePanel, RevealProject, OpenPaneKind, NextTab, OpenEffortMenu, OpenModelMenu, OpenSettings,
        OpenSettingsSection, PinProject, PreviousTab, RemoveConnection, RemovePicture, ShowCommandOutput, ShowPicture, RemoveProject, ResetDockSize,
        PickCommand, SelectChat, Send, Stop, StopCommand, MoveToBackground, ToggleTasks, ToggleTasksPane, StopTask, ShowTask, ToggleAgentsPane, ToggleBrowserPane, ToggleCanvasPane, ShowCanvas, WriteInComposer, ShowAgent, StopAgent, ToggleChanges, ToggleProject, ToggleSidebar, ToggleTerminal, TriggerProjectCommand, Unqueue,
        ToggleAutoTitles, ToggleTitleModels, ChooseTitleModel, RenameChat, RenameChatAt, CancelRename, ToggleModelReasoning, ToggleModelRow, ToggleModelVision, AttachPictures, AttachFiles, RemoveAttachment, InsertPasted, RemovePasted, SteerQueued, RemoveQueued, OpenPicture, ClosePicture, StepPicture, ZoomPicture,
        StartSuggestion, DismissSuggestion, CopySuggestion, OpenChat,
        ArchiveChat, ArchiveCurrentChat, ClearSearch, RevealScratchpadFile, DeleteAllArchived, DeleteArchivedChat, ToggleArchived, UnarchiveChat,
        ToggleModelEffort, SwitchSkill, SwitchSkillFolder, RemoveSkillFolder, OpenSkill, CopySkill, ChooseSkillUse, NewSkill, SaveSkill,
        CloseSkill, AskDeleteSkill, ConfirmSkillDelete, CancelSkillDelete, AddSkillFolder, ImportSkills, SwitchSlashCommand, OpenSlashCommand,
        NewSlashCommand, SaveSlashCommand, CloseSlashCommand, AskDeleteSlashCommand, ConfirmSlashCommandDelete, CancelSlashCommandDelete,
        ChooseDelegation, ToggleHelperModels, ChooseHelperModel, SwitchAgent, SwitchAgentFolder, OpenAgent, NewAgent, DuplicateAgent,
        ChooseAgentModel, SaveAgent, CloseAgent, AskDeleteAgent, CancelAgentDelete, ConfirmAgentDelete,
        OpenPrompt, SavePrompt, ClosePrompt, AskRestorePrompt, CancelRestorePrompt, ConfirmRestorePrompt,
        TogglePromptDefault, AskRestoreAllPrompts, CancelRestoreAllPrompts, ConfirmRestoreAllPrompts,
        EnterCodeArea, EnterChatArea, UseIdea, RevealScratchpad, CopyScratchpadPath,
    },
    app::{AppState, Menu, RenamePlace, Selection, shown_path},
    appearance_page::AppearanceData,
    backdrop::{self, Picture},
    edge_fade::faded,
    glass::{GLASS_BLUR, Lit, Under, glass},
    chat::ChatSession,
    composer::{Choice, Composer, ComposerData, CrewPill, Input, MenuData, MenuGroup, MenuItem, Meter, TeamRow, Waiting},
    motion,
    panes::{
        Axis, ChatPanes, Dock, Layout, PaneId, PaneKind, PaneModel, Rect,
        dock::{self, DraggedTab, Resizing},
        solve::{self, DOCK_MIN_HEIGHT, DOCK_MIN_WIDTH, STACK_MIN_HEIGHT, STACK_MIN_WIDTH, Solution, solve},
    },
    run_button::{self, CommandButtonData, CommandEditorData},
    settings::{self, ProvidersData, Section, SettingsData},
    icon::Icon,
    sidebar::{
        ArchivedData, ArchivedRow, AreaSwitch, ChatGroup, ChatRow, ChatState, ProjectRow, RowConfirm, SIDEBAR_WIDTH, Sidebar, SidebarData, TitleLook, rename_field,
    },
    tags::{self, Tag},
    theme::Theme,
    transcript::{self, Handlers, Item, Transcript},
    typography::{Type, Typography},
};

/// Height of the strip across the top that holds the traffic lights and drags the window.
pub const TITLE_STRIP_HEIGHT: Pixels = px(solve::GAP + solve::STRIP_HEIGHT);
/// The frame kept around the panes, and between the sidebar, the stage and the side panel.
pub const FRAME_GUTTER: Pixels = px(solve::GAP);
/// The panes' corners.
pub const PANE_RADIUS: f32 = 14.;
/// Where macOS draws the traffic lights: inside the sidebar pane, centred on the title strip.
pub const TRAFFIC_LIGHTS: (f32, f32) = (20., 16.);
/// From the traffic lights' left edge to the sidebar toggle beside them, with the sidebar away:
/// the three lights take 52 points, and the toggle stands well clear of the green one.
const TOGGLE_CLEAR: f32 = 80.;
/// The sidebar toggle's width.
const SIDEBAR_TOGGLE: f32 = 24.;

/// The pane solver's space for a window of `width` × `height`: the window less the frame's
/// gutters. The sidebar, the title strip, the stage and the side panel tile it.
pub fn solver_space(width: f32, height: f32) -> (f32, f32) {
    let gutter = f32::from(FRAME_GUTTER);
    (width - 2. * gutter, height - 2. * gutter)
}

/// A window position in the solver's space.
pub fn to_solver(x: f32, y: f32) -> (f32, f32) {
    (x - f32::from(FRAME_GUTTER), y - f32::from(FRAME_GUTTER))
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
/// Below this transcript width the Build/Run split buttons collapse to icons in the chat's header.
const COMPACT_COMMANDS_BELOW: f32 = 640.;
/// The widest the centred new-chat and welcome blocks get.
const NEW_CHAT_WIDTH: f32 = 680.;
const WELCOME_WIDTH: f32 = 560.;
/// A chat this new grows into the sidebar's list instead of appearing.
const FRESH_ROW_MS: u64 = 2_500;
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
    /// The Chat area's new chat: a greeting for the hour ("Good evening"), a few ideas to start
    /// from (indexes into `chat_area::IDEAS`), and the composer.
    ChatHero { greeting: SharedString, ideas: Vec<usize>, composer: ComposerData, notice: Option<SharedString> },
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
    /// The side panel's "+" menu is open.
    pane_menu: bool,
    /// How open the sidebar is (0–1), easing as it opens and closes; unset until the first frame.
    sidebar_open: Option<motion::Ease>,
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
    /// The area on the last frame, and how many times the area has changed: each change is a
    /// hop, and the sidebar's page and the stage arrive from the side of the switch's segment.
    area: Option<ChatKind>,
    hops: usize,
    /// How many times Settings has opened or closed, each a rebuild of the window
    /// (`motion::rebuild`), and whether the last frame showed it.
    rebuilds: usize,
    was_settings: Option<bool>,
    /// The open tab's bead in the tab tray, and where each tab was laid out on the last frame.
    bead: Rc<RefCell<motion::Bead>>,
    tab_bounds: Rc<RefCell<Vec<gpui::Bounds<Pixels>>>>,
    /// Whether the side panel showed last frame, and how many times it has come into view, so
    /// each arrival slides it in once.
    /// How far in the side panel is (0–1), easing as it shows and hides, and its width when it
    /// last showed, for sliding it out.
    side_open: Option<motion::Ease>,
    side_width: f32,
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
        crate::browser::attach(window, cx);
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
            pane_menu: false,
            sidebar_open: None,
            scene_panes: None,
            scene_sidebar_hidden: false,
            scene_backdrop: None,
            _activation: Some(activation),
            focus: Some(cx.focus_handle()),
            paddling: false,
            clear: None,
            had_command_editor: false,
            had_settings_editor: false,
            area: None,
            hops: 0,
            rebuilds: 0,
            was_settings: None,
            bead: Default::default(),
            tab_bounds: Default::default(),
            side_open: None,
            side_width: 0.,
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
            pane_menu: false,
            sidebar_open: None,
            scene_panes: None,
            scene_sidebar_hidden: false,
            scene_backdrop: None,
            _activation: None,
            focus: None,
            paddling: false,
            clear: None,
            had_command_editor: false,
            had_settings_editor: false,
            area: None,
            hops: 0,
            rebuilds: 0,
            was_settings: None,
            bead: Default::default(),
            tab_bounds: Default::default(),
            side_open: None,
            side_width: 0.,
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

    /// The computer use of the chat on screen, and its folder: Code chats only.
    pub fn current_computer(&self, cx: &App) -> Option<(wack_computer::computer::ChatComputer, std::path::PathBuf)> {
        let session = self.app()?.read(cx).current_chat()?.read(cx);
        Some((session.computer.clone()?, session.cwd().to_path_buf()))
    }

    pub fn current_canvases(&self, cx: &App) -> Option<wack_canvas::Canvases> {
        Some(self.app()?.read(cx).current_chat()?.read(cx).canvases.clone())
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
        if app.read(cx).lightbox.is_some() {
            app.update(cx, |app, cx| {
                app.lightbox = None;
                cx.notify();
            });
            return;
        }
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
            // A prompt open on its own page steps back to the prompts first; the page's editor
            // does not validate, so it simply closes.
            if app.read(cx).prompts.editor.is_some() {
                app.update(cx, |app, cx| app.close_prompt(cx));
                return;
            }
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
        // So is an access card, which Esc denies.
        if let Some(card) = self.current_session(cx).and_then(|session| session.read(cx).computer_card()) {
            self.answer_computer_card(card.id, wack_computer::computer::Answer::Deny, cx);
            return;
        }
        self.stop(&Stop, window, cx);
    }

    /// Answers the access card of the chat on screen.
    fn answer_computer_card(&mut self, id: u64, answer: wack_computer::computer::Answer, cx: &mut Context<Self>) {
        let Some(chat) = self.current_session(cx).map(|session| session.read(cx).id.clone()) else { return };
        self.with_app(cx, |app, cx| app.answer_computer_card(&chat, id, answer, cx));
    }

    /// Runs `change` on the chat on screen.
    fn with_session(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut ChatSession, &mut Context<ChatSession>)) {
        if let Some(session) = self.current_session(cx) {
            session.update(cx, change);
        }
    }

    /// Whether the screen on view may show a pane of `kind`: the Chat area has no project, so
    /// only the browser and its canvases.
    fn pane_allowed(&self, kind: PaneKind, cx: &App) -> bool {
        let chat_area = self.app().is_some_and(|app| app.read(cx).area() == wack_core::ChatKind::Chat);
        !chat_area || matches!(kind, PaneKind::Browser | PaneKind::Canvas)
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
            Selection::NewChat(_) | Selection::NewChatArea => app.current_new_chat_panes(),
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
            Selection::NewChat(_) | Selection::NewChatArea => app.update(cx, |app, cx| app.change_new_chat_layout(cx, change)),
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
        if !self.pane_allowed(kind, cx) {
            return;
        }
        let had_focus = self.focused_pane(window, cx);
        match self.change_layout(cx, |layout| layout.toggle(kind)) {
            Some(Some(pane)) if kind == PaneKind::Terminal => self.focus_pane(pane, window, cx),
            Some(None) if had_focus.is_some() => self.focus_composer(window, cx),
            _ => {}
        }
    }

    /// Puts words after whatever the current chat's composer holds, and gives it the keyboard.
    fn write_in_composer(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app() else { return };
        // The chat's composer, or the new-chat screen's draft.
        let composer = {
            let app = app.read(cx);
            match app.current_chat() {
                Some(session) => session.read(cx).composer.clone(),
                None => app.draft.clone(),
            }
        };
        composer.update(cx, |field, cx| {
            let existing = field.text().trim_end().to_string();
            let joined = if existing.is_empty() { text.to_string() } else { format!("{existing}\n\n{text}") };
            field.set_text(&joined, cx);
        });
        self.focus_composer(window, cx);
    }

    /// A new terminal: as a tab beside `with`, or in the right dock.
    fn new_terminal(&mut self, with: Option<PaneId>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pane_allowed(PaneKind::Terminal, cx) {
            return;
        }
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
            Selection::NewChat(_) | Selection::NewChatArea => {
                if let Some((key, _)) = app.draft_tabs().into_iter().find(|(_, active)| *active) {
                    app.close_draft(key, cx);
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
            let room = solve::room_beside(width, f32::from(SIDEBAR_WIDTH));
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
                // The handle is the gap before the panel: the panel starts past the gap's middle.
                let size = (width - x - solve::GAP / 2.).clamp(DOCK_MIN_WIDTH, solve::widest_dock(width).max(DOCK_MIN_WIDTH));
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

    /// The composer's attach button: pictures for the message on screen.
    fn attach_pictures(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.app() else {
            return;
        };
        let Some(chat) = app.read(cx).composing_in() else {
            return;
        };
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                app.update(cx, |app, cx| app.attach_paths(chat, paths, cx));
            }
        })
        .detach();
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
    app.memory.editor.is_some()
        || app.skills.editor.is_some()
        || app.templates.editor.is_some()
        || app.subagents.editor.is_some()
        || app.web.editor.is_some()
        || app.mcp_settings.editor.is_some()
        || app.prompts.editor.is_some()
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
        crate::browser::set_tab_drag(self.dragging.is_some());
        let (sidebar_data, screen, strip) = match &self.source {
            Source::Live(app) => live_view(app.read(cx), cx),
            Source::Fixed(fixed) => fixed.as_ref().clone(),
            Source::Failed(message) => (SidebarData::default(), Screen::Failed(message.clone()), StripData::default()),
        };
        self.paddle(&sidebar_data, cx);
        // An area hop: Chat sits to the right of Code on the switch, so its page comes in from the
        // right and Code's from the left. Nothing moves at launch.
        if self.area.is_some_and(|last| last != sidebar_data.area) {
            self.hops += 1;
        }
        self.area = Some(sidebar_data.area);
        // Opening or closing Settings rebuilds the window, as version 2 did.
        let in_settings = matches!(screen, Screen::Settings(_));
        if self.was_settings.is_some_and(|was| was != in_settings) {
            self.rebuilds += 1;
        }
        self.was_settings = Some(in_settings);
        let hop = (self.hops > 0).then(|| {
            let from = if sidebar_data.area == ChatKind::Chat { motion::HOP } else { -motion::HOP };
            (self.hops, from)
        });
        // The wrapper keeps the sizing its child expects: the sidebar is a flex child of its
        // pane's column (its foot stays at the bottom), the stage's body fills a sized box.
        let hop_in = |name: &'static str, element: gpui::AnyElement| match hop {
            Some((hops, from)) => {
                let wrapper = match name {
                    "hop-sidebar" => div().w_full().flex_1().min_h_0().flex().flex_col(),
                    _ => div().size_full(),
                };
                motion::slide_in(ElementId::NamedInteger(name.into(), hops as u64), wrapper.child(element), from, 0.).into_any_element()
            }
            None => element,
        };
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
        let still = cx.reduce_motion();
        let mut sidebar_open = self.sidebar_open.take();
        let mut side_open = self.side_open.take();
        let mut side_width = self.side_width;
        // 1 while the side panel is settled; how far in it is while it slides.
        let mut side_shown = 1.;
        let rebuild_step = (self.rebuilds > 0 && !still).then_some(self.rebuilds);
        let ms = std::time::Duration::from_millis;
        // 1 while the sidebar is settled; how far open it is while it slides.
        let mut sidebar_shown = 1.;
        let (solution, docks, words) = {
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
            let settled = solve(width, height, !sidebar_hidden, &panes.layout);
            // The sidebar slides open and shut, and the panes glide with it.
            let target = if settled.sidebar { 1. } else { 0. };
            let ease = sidebar_open.get_or_insert(motion::Ease::at(target));
            ease.toward(target, still);
            let solution = if ease.moving() {
                window.request_animation_frame();
                sidebar_shown = ease.value();
                Solution::sliding(width, height, sidebar_shown, &panes.layout)
            } else {
                // The side panel slides in and out from the frame's right edge.
                let target = if settled.right.is_some() { 1. } else { 0. };
                if let Some(right) = settled.right {
                    side_width = right.width;
                }
                let ease = side_open.get_or_insert(motion::Ease::at(target));
                ease.toward(target, still);
                if ease.moving() && side_width > 0. && panes.layout.dock(Dock::Right).is_some() {
                    window.request_animation_frame();
                    side_shown = ease.value();
                    Solution::panel_sliding(width, height, settled.sidebar, &panes.layout, side_width, side_shown)
                } else {
                    settled
                }
            };
            let frame = dock::Frame {
                panes,
                solution: &solution,
                width,
                height,
                dragging: self.dragging.map(|pane| {
                    let (x, y) = to_solver(f32::from(mouse.x), f32::from(mouse.y));
                    (pane, point(px(x), px(y)))
                }),
                addable: [PaneKind::Changes, PaneKind::Terminal, PaneKind::Tasks, PaneKind::Agents, PaneKind::Browser, PaneKind::Canvas]
                    .into_iter()
                    .filter(|&kind| self.pane_allowed(kind, cx))
                    .collect(),
                menu_open: self.pane_menu,
            };
            let docks = dock::docks(&frame, &theme, cx);
            let words = match &screen {
                Screen::Chat { composer, .. } => Some(PaneWords { chat_area: composer.chat_area, ..PaneWords::read(panes, cx) }),
                _ => None,
            };
            (solution, docks, words)
        };
        self.sidebar_open = sidebar_open;
        self.side_open = side_open;
        self.side_width = side_width;
        let narrow = solution.transcript.width < NARROW_TRANSCRIPT;
        let transcript_height = solution.transcript.height;
        // Build and Run shrink to icons when the chat's header is short of room.
        let compact_commands = solution.transcript.width < COMPACT_COMMANDS_BELOW;
        let inset = if narrow { NARROW_INSET } else { CONTENT_INSET };

        // The picture, when there is one: blurred and dimmed behind everything, and clear across
        // the stage on the new-chat and welcome screens, frosting over as a chat opens.
        let picture = match &self.source {
            Source::Live(app) => app.read(cx).backdrop(),
            _ => self.scene_backdrop.clone(),
        };
        let over_picture = picture.is_some();
        let hero_screen = matches!(screen, Screen::Welcome { .. } | Screen::NewChat { .. } | Screen::ChatHero { .. });
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
        // The panes: thin, vibrant glass over a picture, painted from its vibrant copy once that is
        // ready and blurred only as the user chose; without one, the sidebar lies flat on the frame
        // beside the lit stage.
        let pane_blur = picture.as_ref().map_or(GLASS_BLUR, |picture| picture.blur);
        let under = picture.as_ref().and_then(|picture| {
            let (soft, blur) = backdrop::blur_plan(picture.blur);
            let image = backdrop::vibrant(picture, soft, cx)?;
            let fitted = backdrop::cover(whole, (picture.width, picture.height), picture.focus, picture.zoom);
            Some(Under {
                image,
                fitted: gpui::Bounds::new(point(px(fitted.x), px(fitted.y)), gpui::size(px(fitted.width), px(fitted.height))),
                veil: Hsla { a: theme.veil, ..theme.window },
                blur,
            })
        });
        // Without a picture the stage is the lit window and the side panel is lit beside it, a step
        // back; the sidebar lies flat on the frame.
        let pane = |child: gpui::AnyElement, kind: PaneGround| -> gpui::AnyElement {
            if over_picture {
                glass(PANE_RADIUS, pane_blur, Hsla { a: theme.glass, ..theme.glass_tone }, theme.glass_edge, child)
                    .under(under.clone())
                    .lit(Lit::of(&theme, true))
                    .into_any_element()
            } else {
                match kind {
                    PaneGround::Stage => glass(PANE_RADIUS, 0., theme.window, theme.border, child).lit(Lit::of(&theme, false)).into_any_element(),
                    PaneGround::Side => glass(PANE_RADIUS, 0., theme.dock, theme.border, child).lit(Lit::of(&theme, false)).into_any_element(),
                    PaneGround::Flat => child,
                }
            }
        };
        let gutter = FRAME_GUTTER;
        // While Settings is open the sidebar's pane holds its navigation.
        let settings_nav = match &screen {
            Screen::Settings(data) => Some((data.section, settings::NavConnection::list(&data.providers), data.prompts.config.customised_count())),
            _ => None,
        };
        let sidebar = solution.sidebar.then(|| {
            // While it slides the column narrows and the sidebar slips out under its left edge.
            let column = (gutter + SIDEBAR_WIDTH) * sidebar_shown;
            let hidden_part = (gutter + SIDEBAR_WIDTH) - column;
            let column_box = div().flex_none().w(column).h_full().overflow_hidden().opacity(sidebar_shown.powf(0.6));
            motion::rebuild(rebuild_step, "rebuild-sidebar", column_box.child(
            div().relative().left(-hidden_part).w(gutter + SIDEBAR_WIDTH).h_full().p(gutter).flex().flex_col().child(pane(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(drag_strip(
                        "sidebar-strip",
                        TITLE_STRIP_HEIGHT - gutter,
                        div()
                            .size_full()
                            .flex()
                            .items_start()
                            .justify_end()
                            .gap(px(6.))
                            .pt(px(2.))
                            .pr(px(10.))
                            .child(AreaSwitch { area: sidebar_data.area, other_waiting: sidebar_data.other_waiting })
                            .child(sidebar_toggle(&theme)),
                    ))
                    .child(motion::rebuild(
                        rebuild_step,
                        "rebuild-sidebar-rows",
                        div().w_full().flex_1().min_h_0().flex().flex_col().child(match settings_nav {
                            Some((section, connections, prompt_count)) => settings::nav(section, connections, prompt_count, &theme).into_any_element(),
                            None => hop_in("hop-sidebar", Sidebar::new(sidebar_data).into_any_element()),
                        }),
                        ms(100),
                        0.,
                        8.,
                    ))
                    .into_any_element(),
                PaneGround::Flat,
            ))), ms(0), -28., 0.)
        });

        // A screen arrives as it is opened; a chat does not, so switching tabs never moves text.
        let arrive = |id: String, body: gpui::AnyElement| {
            motion::enter(ElementId::Name(id.into()), div().size_full().child(body)).into_any_element()
        };
        let strip_ground = over_picture.then_some(Hsla { a: theme.glass, ..theme.glass_tone });
        let tray = Tray {
            glass: strip_ground.map(|fill| (fill, under.clone())),
            bead: self.bead.clone(),
            tabs: self.tab_bounds.clone(),
            still: cx.reduce_motion(),
        };
        // Files dragged in from the Finder land anywhere on a chat, not only on the composer.
        let droppable = matches!(screen, Screen::NewChat { .. } | Screen::ChatHero { .. } | Screen::Chat { .. });
        let (strip, body) = match screen {
            Screen::Welcome { notice } => {
                let ground = (clearness > 0.).then_some(Hsla { a: theme.cover(), ..theme.window });
                (div(), arrive("arrive-welcome".into(), welcome(notice, ground, &theme).into_any_element()))
            }
            Screen::NewChat { project, composer, notice } => {
                let id = format!("arrive-new-chat-{}", project.path);
                let ground = (clearness > 0.).then_some(Hsla { a: theme.cover(), ..theme.window });
                let body = new_chat(project, composer, &strip, viewport, notice, ground, &theme).into_any_element();
                (tab_strip(&strip, &tray, &theme), arrive(id, body))
            }
            Screen::ChatHero { greeting, ideas, composer, notice } => {
                let ground = (clearness > 0.).then_some(Hsla { a: theme.cover(), ..theme.window });
                let still = cx.reduce_motion();
                let body = chat_hero(greeting, ideas, composer, notice, over_picture, ground, still, &theme).into_any_element();
                (tab_strip(&strip, &tray, &theme), arrive("arrive-chat-hero".into(), body))
            }
            Screen::Chat { title, project, body, composer } => (
                tab_strip(&strip, &tray, &theme),
                {
                    let header = chat_header(title, &project, &strip, &words.unwrap_or_default(), compact_commands, viewport, &theme).into_any_element();
                    // A Chat chat scrolled up from its end offers the way back.
                    let jump = match &body {
                        Body::Live(session, list) if session.read(cx).is_chat_area() && list.is_scrolled_to_end() == Some(false) => {
                            Some((list.clone(), session.read(cx).running()))
                        }
                        _ => None,
                    };
                    let veil = over_picture.then_some(theme.reading_veil);
                    chat(body, composer, header, narrow, transcript_height, cx.reduce_motion(), jump, veil).into_any_element()
                },
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
        let body = if droppable { drop_zone(body, &theme).into_any_element() } else { body };
        // With the sidebar away, the strip makes room for the traffic lights and offers it back,
        // a clear step from the green light so the two are not mistaken for each other. While the
        // sidebar slides, that room opens and closes with it.
        let away = if solution.sidebar { 1. - sidebar_shown } else { 1. };
        let strip = if away <= 0. {
            strip
        } else {
            div()
                .size_full()
                .flex()
                .child(div().flex_none().w((px(TRAFFIC_LIGHTS.0 + TOGGLE_CLEAR) - gutter) * away))
                .child(div().flex_none().w(px(SIDEBAR_TOGGLE + 8.) * away).overflow_hidden().opacity(away).pt(px(10.)).child(sidebar_toggle(&theme)))
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
                    .left_0()
                    .top_0()
                    .w(px(transcript.width))
                    .h(px(transcript.height))
                    .overflow_hidden()
                    .child(motion::rebuild(rebuild_step, "rebuild-stage", div().size_full().child(hop_in("hop-stage", body)), ms(80), 0., 18.)),
            )
            .children(docks.stage);
        let stage = pane(div().flex_1().min_h_0().w_full().child(content).into_any_element(), PaneGround::Stage);
        let main = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pb(gutter)
            .when(solution.right.is_none(), |main| main.pr(gutter))
            .when(solution.right.is_some() && side_shown < 1., |main| main.pr(gutter * (1. - side_shown)))
            .when(!solution.sidebar, |main| main.pl(gutter))
            .when(solution.sidebar && sidebar_shown < 1., |main| main.pl(gutter * (1. - sidebar_shown)))
            .child(drag_strip("title-strip", TITLE_STRIP_HEIGHT, motion::rebuild(rebuild_step, "rebuild-strip", div().size_full().child(strip), ms(140), 0., 10.)))
            .child(stage);
        // The side panel: the right dock, a pane of its own from the frame's top to its foot. It
        // slides in from the window's right edge and back out, its column narrowing as the stage
        // takes the room.
        let side = solution.right.map(|right| {
            let full = px(right.width) + gutter * 2.;
            let panel = div().flex_none().w(full).h_full().p(gutter).flex().flex_col().child(pane(
                div().flex_1().min_h_0().w_full().relative().children(docks.side).into_any_element(),
                PaneGround::Side,
            ));
            if side_shown < 1. {
                div().flex_none().w(full * side_shown).h_full().overflow_hidden().opacity(side_shown.powf(0.5)).child(panel).into_any_element()
            } else {
                panel.into_any_element()
            }
        });
        // Lines, handles and drop zones lie over everything, in the solver's space.
        let over = div().absolute().left(gutter).top(gutter).w(px(width)).h(px(height)).children(docks.over);
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

        let lightbox = match &self.source {
            Source::Live(app) => app.read(cx).lightbox.clone(),
            _ => None,
        };
        tags::root(
            div()
                .key_context(actions::context())
                .when_some(self.focus.as_ref(), |root, focus| root.track_focus(focus))
                .on_action(cx.listener(|shell, _: &NewChat, _, cx| shell.with_app(cx, |app, cx| app.new_chat(None, cx))))
                .on_action(cx.listener(|shell, _: &EnterCodeArea, _, cx| shell.with_app(cx, |app, cx| app.enter_area(ChatKind::Code, cx))))
                .on_action(cx.listener(|shell, _: &EnterChatArea, _, cx| shell.with_app(cx, |app, cx| app.enter_area(ChatKind::Chat, cx))))
                .on_action(cx.listener(|shell, action: &UseIdea, window, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.use_idea(index, cx));
                    shell.focus_composer(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &RevealScratchpad, _, cx| shell.with_app(cx, |app, _| app.reveal_scratchpad())))
                .on_action(cx.listener(|shell, _: &CopyScratchpadPath, _, cx| shell.with_app(cx, |app, cx| app.copy_scratchpad_path(cx))))
                .on_action(cx.listener(|shell, _: &ToggleArchived, _, cx| shell.with_app(cx, |app, cx| app.toggle_archived(cx))))
                .on_action(cx.listener(|shell, action: &RevealScratchpadFile, _, cx| {
                    let path = action.path.clone();
                    shell.with_app(cx, |app, _| app.reveal_scratchpad_file(&path));
                }))
                .on_action(cx.listener(|shell, _: &ClearSearch, _, cx| shell.with_app(cx, |app, cx| app.clear_search(cx))))
                .on_action(cx.listener(|shell, _: &DeleteAllArchived, _, cx| shell.with_app(cx, |app, cx| app.delete_all_archived(cx))))
                .on_action(cx.listener(|shell, action: &ArchiveChat, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.archive_chat(&id, true, cx));
                }))
                .on_action(cx.listener(|shell, _: &ArchiveCurrentChat, _, cx| {
                    shell.with_app(cx, |app, cx| {
                        if let Selection::Chat(id) = app.selection().clone() {
                            app.archive_chat(&id, false, cx);
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &UnarchiveChat, _, cx| {
                    let (id, ask) = (action.id.clone(), action.ask);
                    shell.with_app(cx, |app, cx| app.unarchive_chat(&id, ask, cx));
                }))
                .on_action(cx.listener(|shell, action: &DeleteArchivedChat, _, cx| {
                    let id = action.id.clone();
                    shell.with_app(cx, |app, cx| app.delete_archived_chat(&id, cx));
                }))
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
                .on_action(cx.listener(|shell, _: &actions::OpenCrewMenu, _, cx| shell.open_menu(Menu::Crew, cx)))
                .on_action(cx.listener(|shell, action: &actions::SetTeam, _, cx| {
                    let on = action.on;
                    shell.with_app(cx, |app, cx| app.set_team(on, cx));
                }))
                .on_action(cx.listener(|shell, action: &actions::ChooseCrew, _, cx| {
                    let (connection, model) = (action.connection.clone(), action.model.clone());
                    shell.with_app(cx, |app, cx| app.choose_crew(Some(connection), Some(model), None, cx));
                }))
                .on_action(cx.listener(|shell, action: &actions::ChooseCrewEffort, _, cx| {
                    let effort = action.effort;
                    shell.with_app(cx, |app, cx| app.choose_crew(None, None, Some(effort), cx));
                }))
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
                .on_action(cx.listener(|shell, action: &crate::actions::AnswerComputerCard, _, cx| {
                    let (id, answer) = (action.id, action.answer);
                    shell.answer_computer_card(id, answer, cx);
                }))
                .on_action(cx.listener(|shell, _: &crate::actions::ToggleComputerUse, _, cx| shell.with_app(cx, |app, cx| app.toggle_computer_use(cx))))
                .on_action(cx.listener(|shell, action: &crate::actions::AskComputerAgain, _, cx| {
                    let path = action.path.clone();
                    shell.with_app(cx, |app, cx| app.forget_always_allowed(&path, cx));
                }))
                .on_action(cx.listener(|shell, action: &crate::actions::ForgetNeverAllowed, _, cx| {
                    let bundle_id = action.bundle_id.clone();
                    shell.with_app(cx, |app, cx| app.forget_never_allowed(&bundle_id, cx));
                }))
                .on_action(cx.listener(|shell, action: &crate::actions::RequestComputerPermission, _, cx| {
                    let screen = action.screen;
                    shell.with_app(cx, |app, cx| app.request_computer_permission(screen, cx));
                }))
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
                .on_action(cx.listener(|shell, _: &crate::actions::ToggleAgentOpensPane, _, cx| {
                    shell.with_app(cx, |app, cx| app.toggle_agent_opens_pane(cx))
                }))
                .on_action(cx.listener(|shell, action: &crate::actions::ChooseDefaultViewport, _, cx| {
                    let viewport = action.viewport;
                    shell.with_app(cx, |app, cx| app.choose_default_viewport(viewport, cx))
                }))
                .on_action(cx.listener(|shell, _: &crate::actions::AskClearBrowsingData, _, cx| {
                    shell.with_app(cx, |app, cx| app.ask_clear_browsing_data(true, cx))
                }))
                .on_action(cx.listener(|shell, _: &crate::actions::CancelClearBrowsingData, _, cx| {
                    shell.with_app(cx, |app, cx| app.ask_clear_browsing_data(false, cx))
                }))
                .on_action(cx.listener(|shell, _: &crate::actions::ConfirmClearBrowsingData, _, cx| {
                    shell.with_app(cx, |app, cx| app.clear_browsing_data(cx))
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
                .on_action(cx.listener(|shell, action: &actions::ChooseAgentTeam, _, cx| {
                    let lead = action.lead;
                    shell.with_app(cx, |app, cx| app.choose_agent_team(lead, cx));
                }))
                .on_action(cx.listener(|shell, _: &actions::ToggleTeamModels, _, cx| shell.with_app(cx, |app, cx| app.toggle_team_models(cx))))
                .on_action(cx.listener(|shell, action: &actions::ChooseTeamDefault, _, cx| {
                    let (connection, model) = (action.connection.clone(), action.model.clone());
                    shell.with_app(cx, |app, cx| app.choose_team_default(connection, model, cx));
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
                .on_action(cx.listener(|shell, action: &OpenPrompt, window, cx| {
                    let id = action.id;
                    let field = shell.app().and_then(|app| app.update(cx, |app, cx| app.open_prompt(id, cx)));
                    shell.focus_new_field(field, window, cx);
                }))
                .on_action(cx.listener(|shell, _: &SavePrompt, window, cx| {
                    shell.with_app(cx, |app, cx| app.save_prompt(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &ClosePrompt, window, cx| {
                    shell.with_app(cx, |app, cx| app.close_prompt(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &AskRestorePrompt, _, cx| shell.with_app(cx, |app, cx| app.ask_restore_prompt(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelRestorePrompt, _, cx| shell.with_app(cx, |app, cx| app.ask_restore_prompt(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmRestorePrompt, window, cx| {
                    shell.with_app(cx, |app, cx| app.confirm_restore_prompt(cx));
                    shell.refocus_if_closed(window, cx);
                }))
                .on_action(cx.listener(|shell, _: &TogglePromptDefault, _, cx| shell.with_app(cx, |app, cx| app.toggle_prompt_default(cx))))
                .on_action(cx.listener(|shell, _: &AskRestoreAllPrompts, _, cx| shell.with_app(cx, |app, cx| app.ask_restore_all_prompts(true, cx))))
                .on_action(cx.listener(|shell, _: &CancelRestoreAllPrompts, _, cx| shell.with_app(cx, |app, cx| app.ask_restore_all_prompts(false, cx))))
                .on_action(cx.listener(|shell, _: &ConfirmRestoreAllPrompts, _, cx| shell.with_app(cx, |app, cx| app.confirm_restore_all_prompts(cx))))
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
                .on_action(cx.listener(|shell, _: &ToggleBrowserPane, window, cx| shell.toggle_pane(PaneKind::Browser, window, cx)))
                .on_action(cx.listener(|shell, _: &ToggleCanvasPane, window, cx| shell.toggle_pane(PaneKind::Canvas, window, cx)))
                .on_action(cx.listener(|shell, action: &WriteInComposer, window, cx| shell.write_in_composer(&action.text, window, cx)))
                .on_action(cx.listener(|shell, action: &ShowCanvas, _, cx| {
                    let (canvas, artboards) = (action.canvas.clone(), action.artboards.clone());
                    shell.change_layout(cx, |layout| layout.open(PaneKind::Canvas));
                    shell.with_session(cx, |session, cx| session.panes.show_canvas(&canvas, &artboards, cx));
                }))
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
                .on_action(cx.listener(|shell, _: &AttachPictures, _, cx| shell.attach_pictures(cx)))
                .on_action(cx.listener(|shell, action: &OpenPicture, _, cx| {
                    let lightbox = crate::pictures::Lightbox::new(action.pictures.clone(), action.index);
                    shell.with_app(cx, |app, cx| {
                        app.lightbox = Some(lightbox);
                        cx.notify();
                    });
                }))
                .on_action(cx.listener(|shell, _: &ClosePicture, _, cx| {
                    shell.with_app(cx, |app, cx| {
                        app.lightbox = None;
                        cx.notify();
                    });
                }))
                .on_action(cx.listener(|shell, action: &StepPicture, _, cx| {
                    let by = action.by;
                    shell.with_app(cx, |app, cx| {
                        if let Some(lightbox) = &mut app.lightbox {
                            lightbox.step(by);
                            cx.notify();
                        }
                    });
                }))
                .on_action(cx.listener(|shell, _: &ZoomPicture, _, cx| {
                    shell.with_app(cx, |app, cx| {
                        if let Some(lightbox) = &mut app.lightbox {
                            lightbox.zoomed = !lightbox.zoomed;
                            cx.notify();
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &AttachFiles, _, cx| {
                    let paths = action.paths.clone();
                    shell.with_app(cx, |app, cx| {
                        if let Some(chat) = app.composing_in() {
                            app.attach_paths(chat, paths, cx);
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &InsertPasted, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| {
                        if let Some(chat) = app.composing_in() {
                            app.insert_pasted(chat, index, cx);
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &RemovePasted, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| {
                        if let Some(chat) = app.composing_in() {
                            app.remove_pasted(chat, index, cx);
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &SteerQueued, _, cx| {
                    if let Some(chat) = shell.selected_chat(cx) {
                        let index = action.index;
                        shell.with_app(cx, |app, cx| app.steer_queued(&chat, index, cx));
                    }
                }))
                .on_action(cx.listener(|shell, action: &RemoveQueued, _, cx| {
                    if let Some(chat) = shell.selected_chat(cx) {
                        let index = action.index;
                        shell.with_app(cx, |app, cx| app.remove_queued(&chat, index, cx));
                    }
                }))
                .on_action(cx.listener(|shell, action: &RemoveAttachment, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| {
                        if let Some(chat) = app.composing_in() {
                            app.remove_attachment(chat, index, cx);
                        }
                    });
                }))
                .on_action(cx.listener(|shell, action: &ToggleModelRow, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| {
                        app.form.toggle_open(index);
                        cx.notify();
                    });
                }))
                .on_action(cx.listener(|shell, action: &ToggleModelVision, _, cx| {
                    let index = action.index;
                    shell.with_app(cx, |app, cx| app.toggle_model_vision(index, cx));
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
                .on_action(cx.listener(|shell, action: &PreviewCrop, _, cx| {
                    let focus = (action.across.clamp(0., 1.), action.down.clamp(0., 1.));
                    shell.with_app(cx, |app, cx| {
                        app.crop_preview = Some(focus);
                        cx.notify();
                    });
                }))
                .on_action(cx.listener(|shell, _: &CommitCrop, _, cx| {
                    let mut taken = None;
                    shell.with_app(cx, |app, _| taken = app.crop_preview.take());
                    let Some((across, down)) = taken else { return };
                    shell.change_backdrop(cx, |backdrop| (backdrop.focus_x, backdrop.focus_y) = (across, down));
                }))
                .on_action(cx.listener(|shell, action: &ChooseZoom, window, cx| {
                    let zoom = (wack_core::appearance::MIN_ZOOM + action.steps as u16 * 10).min(wack_core::appearance::MAX_ZOOM);
                    let viewport = window.viewport_size();
                    let frame = crate::panes::Rect { x: 0., y: 0., width: f32::from(viewport.width), height: f32::from(viewport.height) };
                    shell.change_backdrop(cx, |backdrop| {
                        let focus = backdrop::zoom_focus(frame, (backdrop.width, backdrop.height), (backdrop.focus_x, backdrop.focus_y), backdrop.zoom as f32 / 100., zoom as f32 / 100.);
                        (backdrop.focus_x, backdrop.focus_y, backdrop.zoom) = (focus.0, focus.1, zoom);
                    });
                }))
                .on_action(cx.listener(|shell, _: &ResetCrop, _, cx| {
                    shell.change_backdrop(cx, |backdrop| (backdrop.focus_x, backdrop.focus_y, backdrop.zoom) = (0.5, 0.5, wack_core::appearance::MIN_ZOOM));
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
                .on_action(cx.listener(|shell, action: &SelectDraft, _, cx| {
                    let key = action.key;
                    shell.with_app(cx, |app, cx| app.select_draft(key, cx));
                }))
                .on_action(cx.listener(|shell, action: &CloseDraft, _, cx| {
                    let key = action.key;
                    shell.with_app(cx, |app, cx| app.close_draft(key, cx));
                }))
                .on_action(cx.listener(|shell, _: &TogglePaneMenu, _, cx| {
                    shell.pane_menu = !shell.pane_menu;
                    cx.notify();
                }))
                .on_action(cx.listener(|shell, _: &ClosePaneMenu, _, cx| {
                    shell.pane_menu = false;
                    cx.notify();
                }))
                .on_action(cx.listener(|shell, action: &OpenPaneKind, window, cx| {
                    shell.pane_menu = false;
                    match action.kind {
                        PaneKind::Terminal => shell.new_terminal(None, window, cx),
                        kind if shell.pane_allowed(kind, cx) => {
                            shell.change_layout(cx, |layout| layout.open(kind));
                        }
                        _ => {}
                    }
                    cx.notify();
                }))
                .on_action(cx.listener(|shell, _: &RevealProject, _, cx| shell.with_app(cx, |app, _| app.reveal_project())))
                .on_action(cx.listener(|shell, _: &ToggleSidePanel, window, cx| {
                    shell.pane_menu = false;
                    let had_focus = shell.focused_pane(window, cx).is_some();
                    let chat_area = shell.app().is_some_and(|app| app.read(cx).area() == ChatKind::Chat);
                    shell.change_layout(cx, |layout| {
                        if layout.dock(Dock::Right).is_some() {
                            let hidden = layout.hidden(Dock::Right);
                            layout.set_hidden(Dock::Right, !hidden);
                        } else {
                            layout.open(if chat_area { PaneKind::Browser } else { PaneKind::Changes });
                        }
                    });
                    if had_focus {
                        shell.focus_composer(window, cx);
                    }
                }))
                .on_action(cx.listener(|shell, _: &HideSidePanel, window, cx| {
                    shell.pane_menu = false;
                    let had_focus = shell.focused_pane(window, cx).is_some();
                    shell.change_layout(cx, |layout| layout.set_hidden(Dock::Right, true));
                    if had_focus {
                        shell.focus_composer(window, cx);
                    }
                }))
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
                // Browser pages are placed by what this frame painted between these two.
                .child(present_mark(crate::browser::begin_frame))
                .children(behind)
                .children(sidebar)
                .child(main)
                .children(side)
                .child(over)
                .children(lights)
                .children(lightbox.map(|lightbox| crate::pictures::LightboxView { lightbox }))
                .child(present_mark(crate::browser::end_frame)),
        )
    }
}

/// An empty element whose paint registers a presentation callback, at its place in paint order.
fn present_mark(mark: fn(&mut Window)) -> impl IntoElement {
    gpui::canvas(|_, _, _| (), move |_, _, window, _| mark(window)).absolute().size_0()
}

/// The toggles at the right of a chat's header that show and hide its panes.
#[derive(Clone, Debug, Default)]
pub struct PaneWords {
    pub changes: bool,
    pub changed_files: Option<usize>,
    pub terminal: bool,
    pub browser: bool,
    /// The chat has a canvas, so the Canvas word shows; and whether it is in front.
    pub has_canvas: bool,
    pub canvas: bool,
    /// A Chat chat: no project, so no Changes or Terminal, and its scratchpad instead.
    pub chat_area: bool,
    /// Some other pane shows in the side panel (Tasks, Agents…).
    pub other_pane: bool,
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
            browser: visible.iter().any(|pane| pane.kind == PaneKind::Browser),
            has_canvas: panes.canvases().is_some_and(|canvases| !canvases.is_empty()) || panes.layout.first_of(PaneKind::Canvas).is_some(),
            canvas: visible.iter().any(|pane| pane.kind == PaneKind::Canvas),
            chat_area: false,
            other_pane: panes.layout.shown(Dock::Right).is_some(),
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
    /// The area's new chats, each a tab of its own: their keys, and which is on screen.
    pub drafts: Vec<(u64, bool)>,
    pub commands: Option<[CommandButtonData; 2]>,
    pub command_editor: Option<CommandEditorData>,
}

impl StripData {
    /// The same strip with no chat tabs open, for scenes.
    pub fn without_tabs(mut self) -> Self {
        self.tabs.clear();
        self
    }
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
    let area = app.area();
    let terms = app.search_terms(cx);
    let searching = !terms.is_empty();
    let archived = store.archived_chats(area);
    let mut sidebar = SidebarData {
        area,
        other_waiting: app.area_waiting(if area == ChatKind::Chat { ChatKind::Code } else { ChatKind::Chat }, cx),
        search: Some(app.search.clone()),
        searching,
        has_archived: !archived.is_empty(),
        ..SidebarData::default()
    };
    let asking = |chat: &str, which: RowConfirm| app.confirming.as_ref().is_some_and(|(asking, what)| asking == chat && *what == which);
    let project_name = |chat: &wack_core::Chat| chat.project.as_deref().and_then(|id| store.project(id)).map(|project| project.name.clone());
    // What a search looks in: the title, and in Code the project's name.
    let found = |chat: &wack_core::Chat| !searching || crate::sidebar::matches_search(&format!("{} {}", chat.title, project_name(chat).unwrap_or_default()), &terms);
    if app.archived_open {
        sidebar.archived = Some(ArchivedData {
            total: archived.len(),
            confirming_all: app.confirming_all,
            rows: archived
                .iter()
                .filter(|chat| found(chat))
                .map(|chat| ArchivedRow {
                    id: chat.id.clone().into(),
                    title: chat.title.clone().into(),
                    age: AppState::age(chat.archived.unwrap_or(chat.updated)).into(),
                    place: project_name(chat).unwrap_or_else(|| "Chat".into()).into(),
                    selected: *app.selection() == Selection::Chat(chat.id.clone()),
                    confirming: [RowConfirm::Unarchive, RowConfirm::Delete].into_iter().find(|which| asking(&chat.id, *which)),
                })
                .collect(),
        });
    }
    let row = |chat: &wack_core::Chat| {
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
            confirming_archive: asking(&chat.id, RowConfirm::Archive),
            fresh: crate::chat::now_millis().saturating_sub(chat.created) < FRESH_ROW_MS,
        }
    };
    if area == ChatKind::Chat {
        let chats = store.chat_area_chats().into_iter().filter(|chat| found(chat)).map(|chat| (chat.updated, row(chat))).collect();
        sidebar.groups = crate::chat_area::group_by_recency(chats, crate::chat::now_millis())
            .into_iter()
            .map(|(group, chats)| ChatGroup { label: group.label().into(), chats })
            .collect();
    }
    for project in store.projects().iter().filter(|_| area == ChatKind::Code) {
        let chats: Vec<ChatRow> = store.chats(&project.id).into_iter().filter(|chat| found(chat)).map(&row).collect();
        // A search shows only the projects it found chats in, open.
        if searching && chats.is_empty() {
            continue;
        }
        let row = ProjectRow {
            id: project.id.clone().into(),
            name: project.name.clone().into(),
            chats,
            open: searching || app.is_open(&project.id),
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
                prompts: app.prompts.data(store.prompts(), store.planning_unrestricted(), cx),
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
                browser: app.browser_data(),
                computer: app.computer_data(),
            }))
        }
        Selection::NewChatArea => {
            let choice = store.defaults_for(ChatKind::Chat).clone();
            let resolved = store.resolve(&choice);
            let (model, effort) = model_words(resolved.as_ref());
            let hour = wack_core::clock::local(crate::chat::now_millis()).hour;
            Screen::ChatHero {
                greeting: crate::chat_area::greeting(hour).into(),
                ideas: crate::chat_area::pick_ideas(app.hero_visit, 3),
                composer: ComposerData {
                    chat_area: true,
                    archived: None,
                    input: Input::Field(app.draft.clone()),
                    model,
                    connection: connection_name(resolved.as_ref()),
                    effort,
                    ultra_effort: is_ultra(resolved.as_ref()),
                    crew: None,
                    mode: AgentMode::Build,
                    key: "chat-area".into(),
                    meter: None,
                    context_open: false,
                    working: false,
                    waiting: Vec::new(),
                    roomy: true,
                    menu: menu_data(app, resolved.as_ref(), None),
                    slash: app.slash_data(None),
                    hint: None,
                    question: None,
                    access: None,
                    todos: None,
                    activity: None,
                    todos_open: false,
                    tasks: Vec::new(),
                    tasks_open: false,
                    changes: None,
                    foreground: false,
                    attachments: app.draft_attachments.clone(),
                    pasted: app.draft_pasted.clone(),
                    pictures_refused: app.pictures_refused_for(&choice, app.draft_attachments.len()).map(Into::into),
                    comments: None,
                    sent_at: app.sent_at,
                },
                notice: app.notice.clone(),
            }
        }
        Selection::NewChat(project) => match info(project) {
            Some(project) => {
                let choice = store.defaults().clone();
                let resolved = store.resolve(&choice);
                let (model, effort) = model_words(resolved.as_ref());
                let team = team_parts(app, app.new_chat_team.as_ref(), false);
                Screen::NewChat {
                    project,
                    composer: ComposerData {
                        chat_area: false,
                        archived: None,
                        input: Input::Field(app.draft.clone()),
                        model,
                        connection: connection_name(resolved.as_ref()),
                        effort,
                        ultra_effort: is_ultra(resolved.as_ref()),
                        crew: team.0,
                        mode: app.new_chat_mode,
                        key: "new-chat".into(),
                        meter: None,
                        context_open: false,
                        working: false,
                        waiting: Vec::new(),
                        roomy: true,
                        menu: menu_data(app, resolved.as_ref(), Some(team.1)),
                        slash: app.slash_data(None),
                        hint: app.command_hint(None, cx),
                        question: None,
                        access: None,
                        todos: None,
                        activity: None,
                        todos_open: false,
                        tasks: Vec::new(),
                        tasks_open: false,
                        changes: None,
                        foreground: false,
                        attachments: app.draft_attachments.clone(),
                        pasted: app.draft_pasted.clone(),
                        pictures_refused: app.pictures_refused_for(&choice, app.draft_attachments.len()).map(Into::into),
                        comments: None,
                        sent_at: app.sent_at,
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
            let project = info(chat.project.as_deref().unwrap_or_default()).unwrap_or(ProjectInfo { name: "".into(), path: "".into(), branch: None });
            let resolved = store.resolve(&chat.choice);
            let (model, effort) = model_words(resolved.as_ref());
            let state = session.read(cx);
            let team = (!chat.is_chat_area()).then(|| team_parts(app, chat.team.as_ref(), state.running()));
            let window = resolved.as_ref().and_then(|resolved| resolved.model.context_window);
            let meter = state.context_tokens.map(|tokens| meter(tokens, window, !state.is_empty(), state.context_parts.clone()));
            let mut waiting: Vec<Waiting> =
                state.steering.iter().map(|text| Waiting { text: first_line(text), steering: true }).collect();
            waiting.extend(state.queue.iter().map(|text| Waiting { text: first_line(text), steering: false }));
            Screen::Chat {
                title: chat.title.clone().into(),
                project,
                body: Body::Live(session.clone(), state.list.clone()),
                composer: ComposerData {
                    chat_area: chat.is_chat_area(),
                    archived: (chat.is_archived()).then(|| chat.id.clone().into()),
                    input: Input::Field(state.composer.clone()),
                    model,
                    connection: connection_name(resolved.as_ref()),
                    effort,
                    ultra_effort: is_ultra(resolved.as_ref()),
                    crew: team.as_ref().and_then(|team| team.0.clone()),
                    mode: chat.mode,
                    key: chat.id.clone().into(),
                    meter,
                    context_open: false,
                    working: state.running(),
                    waiting,
                    roomy: false,
                    menu: menu_data(app, resolved.as_ref(), team.map(|team| team.1)),
                    slash: app.slash_data(Some(id)),
                    hint: (!chat.is_chat_area()).then(|| app.command_hint(Some(id), cx)).flatten(),
                    question: state.asking.as_ref().map(|asking| crate::question::QuestionData {
                        questions: asking.questions.clone(),
                        tab: asking.tab,
                        drafts: asking.drafts(cx),
                        wrap_up: asking.wrap_up,
                        other: Input::Field(asking.others[asking.tab.min(asking.others.len() - 1)].clone()),
                        focus: Some(asking.focus.clone()),
                        nods: asking.nods,
                    }),
                    access: state.computer_card(),
                    todos: {
                        let view = crate::todo_panel::view(state.todos(), &state.todo_faded, state.todo_show_done);
                        (view.total > 0).then(|| crate::todo_panel::TodoData { view, busy: state.running() })
                    },
                    activity: state.running().then(|| state.activity()).flatten(),
                    todos_open: state.todos_open,
                    tasks: task_lines(&state.tasks),
                    tasks_open: state.tasks_open,
                    changes: state.changes,
                    foreground: state.tasks.has_foreground() || state.agents.active().iter().any(|info| !info.background),
                    attachments: state.attachments.clone(),
                    pasted: state.pasted.clone(),
                    pictures_refused: app.pictures_refused_for(&chat.choice, state.attachments.len()).map(Into::into),
                    comments: {
                        let count: usize = state.comment_drafts.iter().map(|(_, n)| n).sum();
                        state.comment_drafts.first().map(|(canvas, _)| (count, canvas.clone()))
                    },
                    sent_at: app.sent_at,
                },
            }
        }
    };
    let strip = StripData {
        tabs: app
            .area_tabs()
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
        new_chat: matches!(app.selection(), Selection::NewChat(_) | Selection::NewChatArea),
        drafts: app.draft_tabs(),
        commands: app.command_buttons(),
        command_editor: app.command_editor_data(cx),
    };
    (sidebar, screen, strip)
}

/// A Code composer's Team: the crew's half of the model's pill while Team is on, and the model
/// picker's switch. `locked` while a run is going.
fn team_parts(app: &AppState, crew: Option<&wack_core::ModelChoice>, locked: bool) -> (Option<CrewPill>, TeamRow) {
    let store = app.store();
    let unavailable = (store.agents_config().delegation == wack_core::agents::Delegation::Off)
        .then(|| SharedString::from("Sub-agents are off in Settings › Sub-agents, so there is no one to lead."));
    let pill = crew.map(|crew| {
        let resolved = store.resolve_crew(crew);
        let model = match &resolved {
            Some(resolved) => model_name(&resolved.model.id),
            None => crew.model.as_deref().map(model_name).unwrap_or_else(|| "No model".into()),
        };
        CrewPill {
            model: model.into(),
            connection: connection_name(resolved.as_ref()),
            // Only an effort chosen for the crew is named; otherwise the model's own default runs.
            effort: crew.effort.and(resolved.as_ref()).map(|resolved| resolved.effort.label().into()),
            problem: store.team_problem(Some(crew)).map(Into::into),
            missing: resolved.is_none(),
        }
    });
    (pill, TeamRow { on: crew.is_some(), locked, unavailable })
}

/// The open menu's choices, with the one in use marked. `team` is the model picker's Team
/// switch, for a Code chat's composer.
fn menu_data(app: &AppState, resolved: Option<&wack_core::Resolved>, team: Option<TeamRow>) -> Option<MenuData> {
    let kind = app.menu?;
    if kind == Menu::Crew {
        return crew_menu(app, team);
    }
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
        Menu::Crew => Vec::new(),
    };
    let team = team.filter(|_| kind == Menu::Model);
    Some(MenuData { kind, groups, team })
}

/// The crew's picker: every model of every connection that is on, the crew's marked, then its
/// effort's levels (the model's own default first; never Ultra, which is the chat's own).
fn crew_menu(app: &AppState, team: Option<TeamRow>) -> Option<MenuData> {
    let crew = match app.selection() {
        Selection::Chat(id) => app.store().chat(id).and_then(|chat| chat.team.clone()),
        Selection::NewChat(_) => app.new_chat_team.clone(),
        _ => None,
    }?;
    let store = app.store();
    let mut groups: Vec<MenuGroup> = store
        .enabled_connections()
        .filter(|connection| !connection.models.is_empty())
        .map(|connection| MenuGroup {
            title: Some(connection.name.clone().into()),
            items: connection
                .models
                .iter()
                .map(|model| MenuItem {
                    label: model_name(&model.id).into(),
                    detail: model.context_window.map(|window| crate::work::count(window).into()),
                    selected: crew.connection.as_deref() == Some(connection.id.as_str()) && crew.model.as_deref() == Some(model.id.as_str()),
                    choice: Choice::Crew { connection: connection.id.clone(), model: model.id.clone() },
                })
                .collect(),
        })
        .collect();
    if let Some(resolved) = store.resolve_crew(&crew) {
        let levels: Vec<wack_core::Reasoning> = resolved.model.choices().into_iter().filter(|level| !level.is_ultra()).collect();
        if levels.len() > 1 {
            let mut items = vec![MenuItem { label: "Default".into(), detail: None, selected: crew.effort.is_none(), choice: Choice::CrewEffort(None) }];
            items.extend(levels.into_iter().map(|level| MenuItem {
                label: level.label().into(),
                detail: None,
                selected: crew.effort == Some(level),
                choice: Choice::CrewEffort(Some(level)),
            }));
            groups.push(MenuGroup { title: None, items });
        }
    }
    let _ = team;
    Some(MenuData { kind: Menu::Crew, groups, team: None })
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

/// A chat that takes files dropped anywhere on it: while they hover, a frosted sheet over the
/// column says where they will go.
fn drop_zone(body: gpui::AnyElement, theme: &Theme) -> impl IntoElement {
    div()
        .id("chat-drop")
        .group("chat-drop")
        .relative()
        .size_full()
        .on_drop(|paths: &gpui::ExternalPaths, window, cx| {
            window.dispatch_action(Box::new(actions::AttachFiles { paths: paths.paths().to_vec() }), cx);
        })
        .child(body)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .p(px(14.))
                .invisible()
                .group_drag_over::<gpui::ExternalPaths>("chat-drop", |sheet| sheet.visible())
                .child(
                    div()
                        .size_full()
                        .rounded(px(18.))
                        .border_2()
                        .border_color(theme.accent_line)
                        .bg(Hsla { a: 0.86, ..theme.window })
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(10.))
                        .child(Icon::Paperclip.view(px(26.), theme.accent_text))
                        .child(div().typography(Type::Heading).text_color(theme.text).child("Drop to attach"))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child("Pictures go with your next message")),
                ),
        )
}

/// How full the context is for the ring round Send: getting full from 80% (the agent compacts
/// on its own there), and worth compacting by hand from a quarter (or 50k tokens when the
/// model's window is unknown).
fn meter(tokens: u64, window: Option<u64>, has_messages: bool, parts: Vec<wack_core::context::Part>) -> Meter {
    let window = window.filter(|window| *window > 0);
    let (high, can_compact) = match window {
        Some(window) => {
            let percent = tokens as f32 / window as f32 * 100.;
            (percent >= wack_core::context::COMPACT_AT_PERCENT as f32, has_messages && percent >= 25.)
        }
        None => (false, has_messages && tokens >= 50_000),
    };
    Meter { tokens, window, high, can_compact, parts }
}

/// The connection's name, for the mark on the model's pill.
fn connection_name(resolved: Option<&wack_core::Resolved>) -> SharedString {
    resolved.map(|resolved| resolved.connection.name.clone().into()).unwrap_or_default()
}

/// What a pane lies on when there is no picture to frost.
#[derive(Clone, Copy)]
enum PaneGround {
    /// The lit window.
    Stage,
    /// Lit, a step back from the stage.
    Side,
    /// Nothing: flat on the frame.
    Flat,
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
                            run_button::split_button(button, strip.command_editor.as_ref(), false, viewport, theme)
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

/// The Chat area's new chat, after version 2's hero: a small duck, a greeting for the hour, a
/// line of welcome, the composer, and three ideas to start from (each fills the composer, never
/// sends). Over a picture the picture stays clear: the small words sit on frosted pills and the
/// greeting has its halo. With no picture the duck floats on a pond of light, its ripples
/// widening slowly (still under Reduce Motion).
#[allow(clippy::too_many_arguments)]
fn chat_hero(
    greeting: SharedString,
    ideas: Vec<usize>,
    composer: ComposerData,
    notice: Option<SharedString>,
    over_picture: bool,
    ground: Option<Hsla>,
    still: bool,
    theme: &Theme,
) -> impl IntoElement {
    let hint = |keys: &'static str, what: &'static str| {
        div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.text_dim).child(keys))
            .child(div().text_color(theme.text_faint).child(what))
    };
    let chip = |index: usize| {
        let idea = crate::chat_area::IDEAS[index];
        let pill = div()
            .id(SharedString::from(format!("idea-{index}")))
            .h(px(30.))
            .px(px(14.))
            .rounded_full()
            .flex()
            .items_center()
            .typography(Type::Ui)
            .text_color(theme.text_soft)
            .border_1()
            .border_color(theme.border)
            .when(ground.is_none(), |pill| pill.bg(theme.surface))
            .hover(|pill| pill.border_color(theme.accent_line).text_color(theme.text))
            .on_click(move |_, window, cx| window.dispatch_action(Box::new(UseIdea { index }), cx))
            .child(idea.label)
            .tag("idea");
        match ground {
            Some(ground) => glass(15., GLASS_BLUR, ground, theme.border, pill).into_any_element(),
            None => pill.into_any_element(),
        }
    };
    centred(
        div()
            .relative()
            .w_full()
            .max_w(px(NEW_CHAT_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .child(duck_on_pond(!over_picture, still, theme))
            .child(div().pt(px(10.)).child(on_ground(div().typography(Type::Label).text_color(theme.text_dim).child(greeting), ground, theme)))
            .child(div().pt(px(6.)).child(display_line("What's on your mind?".into(), ground.is_some(), theme)))
            .child(div().pt(px(8.)).child(on_ground(
                div().typography(Type::Body).text_color(theme.text_soft).child("WackCode is all ears. Ask, ramble, or think out loud."),
                ground,
                theme,
            )))
            .child(div().w_full().pt(px(22.)).child(Composer::new(composer)))
            .child(div().pt(px(14.)).flex().flex_wrap().justify_center().gap(px(8.)).children(ideas.into_iter().map(chip)))
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

/// The hero's duck: in a small round of the window's colour, and, with no picture behind it,
/// on a pond of light whose three ripples widen and fade in turn.
/// The hero's duck, on a pond of light when no picture is set: three rings spread from it and
/// fade, each a third behind the last. With Reduce Motion the rings hold still where they are a
/// third and two thirds out.
fn duck_on_pond(pond: bool, still: bool, theme: &Theme) -> impl IntoElement {
    const SIZE: f32 = 96.;
    const RIPPLE: std::time::Duration = std::time::Duration::from_millis(4200);
    let duck = div()
        .size(px(44.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.window)
        .border_1()
        .border_color(theme.border)
        .child(Icon::Duck.view(px(22.), theme.accent_text));
    let ripples = (0..3).filter(|_| pond).map(|ring| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size(px(SIZE))
            .flex()
            .items_center()
            .justify_center()
            .child({
                let ring_at = move |ripple: gpui::Div, t: f32| {
                    let size = 44. + (SIZE - 44.) * t;
                    ripple.size(px(size)).opacity(0.35 * (1. - t))
                };
                let ripple = div().rounded_full().border_1().border_color(theme.accent);
                if still {
                    ring_at(ripple, ring as f32 / 3.).into_any_element()
                } else {
                    ripple
                        .with_animation(ElementId::Name(format!("ripple-{ring}").into()), gpui::Animation::new(RIPPLE).repeat(), move |ripple, t| {
                            // Each ripple a third of the way behind the last.
                            ring_at(ripple, (t + ring as f32 / 3.) % 1.)
                        })
                        .into_any_element()
                }
            })
    });
    div()
        .relative()
        .flex_none()
        .size(px(SIZE))
        .flex()
        .items_center()
        .justify_center()
        .when(pond, |round| round.rounded_full().bg(Hsla { a: 0.07, ..theme.accent }))
        .children(ripples)
        .child(duck)
        .tag("chat-hero-duck")
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

/// What the tab tray is laid on, and its bead, carried from the shell into the strip.
pub struct Tray {
    /// Over a picture: the panes' glass fill and the softened copy beneath it.
    glass: Option<(Hsla, Option<Under>)>,
    bead: Rc<RefCell<motion::Bead>>,
    /// Each tab's bounds as last laid out, in window coordinates, in order.
    tabs: Rc<RefCell<Vec<gpui::Bounds<Pixels>>>>,
    still: bool,
}

impl Tray {
    /// `child` on the tray's glass: the panes' own over a picture, the lit stage's without.
    fn glass(&self, radius: f32, child: impl IntoElement, theme: &Theme) -> crate::glass::Glass {
        match &self.glass {
            Some((fill, under)) => glass(radius, GLASS_BLUR, *fill, theme.glass_edge, child).under(under.clone()).lit(Lit::of(theme, false)),
            None => glass(radius, 0., theme.window, theme.border, child).lit(Lit::of(theme, false)),
        }
    }
}

/// How the open tab's bead looks: raised glass with an accent rim, glowing faintly.
#[derive(Clone, Copy)]
struct BeadLook {
    fill: Hsla,
    line: Hsla,
    highlight: Hsla,
    glow: Hsla,
}

fn paint_bead(rect: gpui::Bounds<Pixels>, look: BeadLook, window: &mut Window) {
    let corners = gpui::Corners::all(rect.size.height / 2.);
    let line = |colour: Hsla, offset: f32, blur: f32, spread: f32, inset: bool| gpui::BoxShadow {
        color: colour,
        offset: point(px(0.), px(offset)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset,
    };
    window.paint_drop_shadows(rect, corners, &[line(look.glow, 4., 14., -4., false)]);
    window.paint_layer(rect, |window| window.paint_quad(gpui::quad(rect, corners, look.fill, px(1.), look.line, Default::default())));
    window.paint_layer(rect, |window| window.paint_inset_shadows(rect, corners, &[line(look.highlight, 1., 0., 0., true)]));
}

/// The open chats in a tray of glass across the strip, the one in front carried by a bead that
/// slides to each tab as it opens, then a round button for a new chat.
fn tab_strip(strip: &StripData, tray: &Tray, theme: &Theme) -> gpui::Div {
    // A chat's tab, or (with no id) a new chat's, by its draft key.
    let tab = |id: Option<SharedString>, draft: u64, title: SharedString, look: TitleLook, active: bool, state: ChatState| {
        let name: SharedString = match &id {
            Some(id) => id.clone(),
            None => format!("new-{draft}").into(),
        };
        let group: SharedString = format!("chat-tab-{name}").into();
        let close = match &id {
            Some(id) => Box::new(CloseChatTab { id: id.to_string() }) as Box<dyn gpui::Action>,
            None => Box::new(CloseDraft { key: draft }),
        };
        let select = id.clone().map(|id| SelectChat { id: id.to_string() });
        div()
            .id(SharedString::from(format!("tab-{name}")))
            .group(group.clone())
            .occlude()
            .relative()
            // When the strip runs short the other tabs give way, down to a stub each; the open one
            // keeps its title.
            .map(|tab| if active { tab.flex_none() } else { tab.flex_shrink_1() })
            .min_w(px(64.))
            .max_w(px(220.))
            .h(px(28.))
            .pl(px(12.))
            .pr(px(5.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(!active, |tab| tab.hover(|tab| tab.bg(Hsla { a: 0.5, ..theme.hover })))
            .when(id.is_none() && !active, |tab| {
                tab.on_click(move |_, window, cx| window.dispatch_action(Box::new(SelectDraft { key: draft }), cx))
            })
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
            // A light crosses a tab while its chat works, under its words.
            .when(state == ChatState::Working, |tab| {
                tab.children(motion::sheen_band(SharedString::from(format!("sheen-{group}")), Hsla { a: 0.22, ..theme.accent }, 14., tray.still))
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
                    .id(SharedString::from(format!("close-{name}")))
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
        strip.tabs.iter().map(|data| tab(Some(data.id.clone()), 0, data.title.clone(), data.look.clone(), data.active, data.state)).collect();
    // Every new chat of the area is a tab after the chats'; a scene's new chat has no draft yet.
    let drafts: Vec<(u64, bool)> = if strip.drafts.is_empty() && strip.new_chat { vec![(0, true)] } else { strip.drafts.clone() };
    tabs.extend(drafts.iter().map(|&(key, active)| tab(None, key, "New chat".into(), TitleLook::Plain, active, ChatState::Idle)));
    // The tab the bead carries, by place in the row and by name.
    let active = strip
        .tabs
        .iter()
        .position(|tab| tab.active)
        .map(|index| (index, strip.tabs[index].id.clone()))
        .or_else(|| drafts.iter().position(|(_, active)| *active).map(|at| (strip.tabs.len() + at, SharedString::from(format!("new-{}", drafts[at].0)))));
    let look = BeadLook {
        fill: if tray.glass.is_some() { Hsla { a: 0.86, ..theme.raised } } else { theme.raised },
        line: theme.accent_line,
        highlight: theme.glass_highlight,
        glow: Hsla { a: 0.32, ..theme.accent },
    };
    let (state, bounds, still) = (tray.bead.clone(), tray.tabs.clone(), tray.still);
    // Painted under the tabs, from where they were laid out this frame: the row records them as it
    // is prepainted, which is over before anything paints.
    let bead = gpui::canvas(
        |_, _, _| (),
        move |area, _, window, _| {
            let Some((index, key)) = active else { return };
            let Some(tab) = bounds.borrow().get(index).copied() else { return };
            let now = std::time::Instant::now();
            let mut bead = state.borrow_mut();
            let span = (f32::from(tab.origin.x), f32::from(tab.origin.x + tab.size.width));
            let (left, right) = bead.at(&key, span, still, now);
            if bead.moving(now) {
                window.request_animation_frame();
            }
            // A tab scrolled out of the row keeps its bead inside the tray.
            let (inner_left, inner_right) = (f32::from(area.origin.x) + 3., f32::from(area.origin.x + area.size.width) - 3.);
            let (left, right) = (left.max(inner_left), right.min(inner_right));
            if right - left > 1. {
                paint_bead(gpui::Bounds::new(point(px(left), tab.origin.y), gpui::size(px(right - left), tab.size.height)), look, window);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    let recorded = tray.tabs.clone();
    let row = div()
        .h(px(28.))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(2.))
        .overflow_hidden()
        .on_children_prepainted(move |children, _, _| *recorded.borrow_mut() = children)
        .children(tabs);
    let tray_box = div().relative().flex_initial().min_w_0().h(px(34.)).p(px(3.)).flex().items_center().child(bead).child(row);
    // Always there, as version 2's was: on a new chat it opens another beside it. Outside the
    // clipped row, so it shows however many tabs there are.
    let new_tab = Some(()).map(|_| {
        tray.glass(
            17.,
            div()
                .id("new-tab")
                .occlude()
                .flex_none()
                .size(px(34.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(|button| button.bg(Hsla { a: 0.5, ..theme.hover }))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(NewChat), cx))
                .child(Icon::Plus.view(px(14.), theme.text_dim))
                .tag("new-tab"),
            theme,
        )
    });
    div()
        .size_full()
        .pl(px(2.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(tray.glass(17., tray_box, theme))
        .children(new_tab)
}

/// A chat's header over its transcript: its title, the project and branch it works in, and at
/// the right its Build and Run buttons and the toggles for its panes.
fn chat_header(
    title: SharedString,
    project: &ProjectInfo,
    strip: &StripData,
    words: &PaneWords,
    compact: bool,
    viewport: Size<Pixels>,
    theme: &Theme,
) -> impl IntoElement {
    // Where the chat lives, as small pills under its title, after version 2: the folder (which
    // shows it in the Finder) and the branch. In a narrow header they give way before the
    // buttons, their words cut short.
    let pill = |id: &'static str, icon: Icon, text: SharedString, tip: SharedString| {
        div()
            .id(id)
            .flex_shrink_1()
            .min_w(px(40.))
            .max_w(px(340.))
            .h(px(20.))
            .pl(px(7.))
            .pr(px(8.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(5.))
            .bg(Hsla { a: 0.05, ..theme.text })
            .hover(|pill| pill.bg(Hsla { a: 0.1, ..theme.text }))
            .tooltip(crate::tooltip::tip(tip))
            .child(div().flex_none().child(icon.view(px(12.), theme.accent_text)))
            .child(div().min_w_0().truncate().typography(Type::Small).text_color(theme.text_soft).child(text))
    };
    // The folder as its last two parts, as version 2 showed it.
    let folder: SharedString = {
        let parts: Vec<&str> = project.path.split('/').filter(|part| !part.is_empty() && *part != "~").collect();
        if parts.is_empty() { project.name.clone() } else { parts[parts.len().saturating_sub(2)..].join("/").into() }
    };
    let commands = strip.commands.as_ref().filter(|_| !words.chat_area).map(|commands| {
        div().flex_none().flex().items_center().gap(px(8.)).children(
            commands.iter().map(|button| run_button::split_button(button, strip.command_editor.as_ref(), compact, viewport, theme)),
        )
    });
    let panel_on = words.changes || words.terminal || words.browser || words.canvas || words.other_pane;
    let panel_button = div()
        .id("toggle-side-panel")
        .occlude()
        .flex_none()
        .size(px(30.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(if panel_on { theme.border_strong } else { theme.border })
        .when(panel_on, |button| button.bg(Hsla { a: 0.6, ..theme.hover }))
        .hover(|button| button.bg(theme.hover).border_color(theme.border_strong))
        .cursor_pointer()
        .tooltip(crate::tooltip::tip(if panel_on { "Hide panel  ⌥⌘B" } else { "Show panel  ⌥⌘B" }))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSidePanel), cx))
        .child(Icon::PanelRight.view(px(14.), if panel_on { theme.text } else { theme.text_soft }))
        .tag("toggle-side-panel");
    div()
        .flex_none()
        .w_full()
        .min_h(px(58.))
        .flex()
        .items_center()
        .gap(px(12.))
        .pl(px(20.))
        .pr(px(14.))
        .pt(px(10.))
        .pb(px(9.))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .id("chat-title")
                        .min_w_0()
                        .truncate()
                        .typography(Type::Heading)
                        .text_color(theme.text)
                        .tooltip(crate::tooltip::tip("Double-click to rename"))
                        .on_click(|event, window, cx| {
                            if event.click_count() >= 2 {
                                window.dispatch_action(Box::new(RenameChat), cx);
                            }
                        })
                        .child(title),
                )
                .when(!project.name.is_empty(), |text| {
                    text.child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(
                                pill("header-folder", Icon::Folder, folder, format!("{} · Show in Finder", project.path).into())
                                    .cursor_pointer()
                                    .on_click(|_, window, cx| window.dispatch_action(Box::new(RevealProject), cx)),
                            )
                            .when_some(project.branch.clone(), |pills, branch| {
                                pills.child(pill("header-branch", Icon::Branch, branch.clone(), format!("On {branch}").into()))
                            }),
                    )
                }),
        )
        .child(div().flex_none().flex().items_center().gap(px(8.)).children(commands).child(pane_toggles(words, theme)).child(panel_button))
        .tag("chat-header")
}

/// The toggles for a chat's panes, as one segment of icons; the open ones lit in the accent.
fn pane_toggles(words: &PaneWords, theme: &Theme) -> impl IntoElement {
    let toggle = |id: &'static str, icon: Icon, label: &'static str, count: Option<usize>, on: bool, action: Box<dyn gpui::Action>| {
        let count = count.filter(|count| *count > 0);
        div()
            .id(id)
            .occlude()
            .flex_none()
            .h(px(28.))
            .min_w(px(32.))
            .px(px(9.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .when(on, |toggle| toggle.bg(theme.accent_wash).border_1().border_color(theme.accent_line))
            .when(!on, |toggle| toggle.hover(|toggle| toggle.bg(Hsla { a: 0.6, ..theme.hover })))
            .cursor_pointer()
            .tooltip(crate::tooltip::tip(label))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            .child(icon.view(px(14.), if on { theme.accent_text } else { theme.text_dim }))
            .when_some(count, |toggle, count| {
                toggle.child(div().typography(Type::Small).text_color(if on { theme.accent_text } else { theme.text_faint }).child(count.to_string()))
            })
            .tag(id)
    };
    div()
        .flex_none()
        .h(px(32.))
        .p(px(2.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(2.))
        .bg(Hsla { a: 0.45, ..theme.sidebar })
        .border_1()
        .border_color(theme.glass_edge)
        .when(!words.chat_area, |segment| {
            segment
                .child(toggle("toggle-changes", Icon::Branch, "Changes ⌘⇧G", words.changed_files, words.changes, Box::new(ToggleChanges)))
                .child(toggle("toggle-terminal", Icon::Terminal, "Terminal ⌃`", None, words.terminal, Box::new(ToggleTerminal)))
        })
        .when(words.chat_area, |segment| {
            segment.child(toggle("reveal-scratchpad", Icon::Folder, "Scratchpad", None, false, Box::new(RevealScratchpad)))
        })
        .child(toggle("toggle-browser", Icon::Globe, "Browser", None, words.browser, Box::new(ToggleBrowserPane)))
        .when(words.has_canvas, |segment| {
            segment.child(toggle("toggle-canvas", Icon::Board, "Canvas", None, words.canvas, Box::new(ToggleCanvasPane)))
        })
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

/// `jump`: the transcript is scrolled up from its end (its list, and whether a reply is coming
/// in), so a pill over the composer takes it back.
#[allow(clippy::too_many_arguments)]
fn chat(
    body: Body,
    composer: ComposerData,
    header: gpui::AnyElement,
    narrow: bool,
    height: f32,
    still: bool,
    jump: Option<(ListState, bool)>,
    veil: Option<Hsla>,
) -> impl IntoElement {
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
                still,
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
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .children(veil.map(|veil| reading_veil(veil, narrow)))
        .child(header)
        // Text fades out as it scrolls under the chat's header and the composer, rather than
        // being cut off; the fade is the text's own, so a backdrop shows through untouched.
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(faded(transcript, TOP_FADE, DOCK_FADE))
                .when_some(jump, |area, (list, coming)| area.child(jump_pill(list, coming))),
        )
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

/// Over a picture, a band of calmer glass down the stage behind the transcript's column: full
/// across the column, fading out through the gutters either side, so it has no edge to see.
fn reading_veil(veil: Hsla, narrow: bool) -> impl IntoElement {
    let clear = Hsla { a: 0., ..veil };
    let side = gutter(narrow);
    let fade = |from: Hsla, to: Hsla| {
        div().flex_none().w(side).h_full().bg(gpui::linear_gradient(90., gpui::linear_color_stop(from, 0.), gpui::linear_color_stop(to, 1.)))
    };
    div().absolute().top_0().left_0().size_full().flex().justify_center().child(
        div()
            .w_full()
            .max_w(TRANSCRIPT_MAX_WIDTH + side * 2.)
            .h_full()
            .flex()
            .child(fade(clear, veil))
            .child(div().flex_1().h_full().bg(veil))
            .child(fade(veil, clear)),
    )
}

/// "Jump to latest", or "New messages" while a reply comes in below: over the transcript's foot.
fn jump_pill(list: ListState, coming: bool) -> impl IntoElement {
    let words = if coming { "New messages" } else { "Jump to latest" };
    div().absolute().bottom(px(4.)).left_0().right_0().flex().justify_center().child(JumpPill { list, words })
}

#[derive(IntoElement)]
struct JumpPill {
    list: ListState,
    words: &'static str,
}

impl RenderOnce for JumpPill {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx);
        let list = self.list;
        div()
            .id("jump-to-latest")
            .occlude()
            .h(px(30.))
            .px(px(14.))
            .rounded_full()
            .flex()
            .items_center()
            .gap(px(6.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_md()
            .cursor_pointer()
            .hover(|pill| pill.border_color(theme.accent_line))
            .on_click(move |_, window, _| {
                list.scroll_to_end();
                window.refresh();
            })
            .child(Icon::ChevronDown.view(px(12.), theme.accent_text))
            .child(div().typography(Type::Small).text_color(theme.text).child(self.words))
            .tag("jump-to-latest")
    }
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
    fn the_meter_fills_as_a_share_and_offers_compact_from_a_quarter() {
        let meter = |tokens, window| meter(tokens, window, true, Vec::new());
        let small = meter(4_000, Some(1_000_000));
        assert_eq!(small.share(), Some(0.004));
        assert!(!small.can_compact && !small.high);

        let quarter = meter(250_000, Some(1_000_000));
        assert!(quarter.can_compact);
        assert!(!super::meter(250_000, Some(1_000_000), false, Vec::new()).can_compact, "nothing to compact in an empty chat");

        assert!(meter(850_000, Some(1_000_000)).high, "from where the agent compacts on its own");
        assert!(!meter(790_000, Some(1_000_000)).high);
        assert_eq!(meter(2_000_000, Some(1_000_000)).share(), Some(1.), "a share never passes full");
    }

    #[test]
    fn without_a_window_the_meter_has_no_share() {
        let unknown = meter(41_200, None, true, Vec::new());
        assert_eq!(unknown.share(), None);
        assert!(!unknown.can_compact);
        assert!(meter(60_000, Some(0), true, Vec::new()).can_compact, "a zero window counts as unknown");
    }

    #[test]
    fn model_names_drop_the_organisation() {
        assert_eq!(model_name("zai-org/GLM-5.3-Flash"), "GLM-5.3-Flash");
        assert_eq!(model_name("claude-opus-5-5"), "claude-opus-5-5");
    }
}
