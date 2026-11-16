//! Named UI scenes: a view in a known state at a known size.
//!
//! The `gallery` binary renders these to PNG files, which is how UI work is checked without
//! launching the app. Add a scene for every component state worth looking at.

use gpui::{AnyView, App, Context, Pixels, Size, Window, prelude::*, px, size};
use wack_core::{CommandKind, CommandSession, CommandStatus};

use crate::{
    composer::{ComposerData, Input},
    panes::{ChangesPane, ChatPanes, Dock, Edge, Layout, PaneKind, PaneModel, RunPane, Target, TerminalPane},
    run_button::{CommandButtonData, CommandEditorData},
    settings::{ProviderForm, ProvidersData, Section, SettingsData},
    fixtures::{self, Selected},
    shell::{Body, Screen, Shell},
    specimen::Specimen,
    tags,
    text_field::TextField,
    typography::Type,
};

pub struct Scene {
    pub name: &'static str,
    pub description: &'static str,
    pub size: Size<Pixels>,
    build: fn(&mut Window, &mut App) -> AnyView,
}

impl Scene {
    pub fn build(&self, window: &mut Window, cx: &mut App) -> AnyView {
        (self.build)(window, cx)
    }
}

/// The app's default window size.
const WINDOW: Size<Pixels> = size(px(1180.), px(760.));

pub fn all() -> Vec<Scene> {
    vec![
        Scene {
            name: "welcome",
            description: "First launch: no projects yet.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| Shell::showing(Default::default(), Screen::Welcome { notice: None })).into()
            },
        },
        Scene {
            name: "new-chat",
            description: "The new-chat screen in a project, with the sidebar.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: None,
                        },
                        fixtures::strip(None),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "new-chat-no-connection",
            description: "Sending a first message before any connection is set up.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: Some("Add a connection in Settings › Providers to start chatting.".into()),
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat",
            description: "A finished turn: prompt, work log with an open diff, reply with code. The composer is a live text field.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| {
                let field = cx.new(|cx| {
                    let mut field = TextField::new(Type::Body, crate::chat::FOLLOW_UP_PLACEHOLDER, cx).multiline(12);
                    field.set_text(fixtures::NEXT_MESSAGE, cx);
                    field
                });
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Field(field)),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-working",
            description: "The agent at work: live log, streaming reply, stop and a queued message.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::working_turn()),
                            composer: fixtures::composer_working(),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-panes",
            description: "A chat with Changes beside the transcript and a terminal below, at a wide window.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                let mut layout = Layout::default();
                let changes = layout.open(PaneKind::Changes);
                let terminal = layout.add(PaneKind::Terminal);
                layout.right_width = Some(560.);
                panes_scene(layout, &[changes, terminal], false, cx)
            },
        },
        Scene {
            name: "chat-panes-narrow",
            description: "The smallest window, 716 points: the sidebar steps aside and the transcript keeps its minimum.",
            size: size(px(716.), px(720.)),
            build: |_, cx| {
                let mut layout = Layout::default();
                let changes = layout.open(PaneKind::Changes);
                panes_scene(layout, &[changes], false, cx)
            },
        },
        Scene {
            name: "chat-panes-split",
            description: "Sidebar hidden; the right dock split into Changes over two terminal tabs.",
            size: size(px(1280.), px(860.)),
            build: |_, cx| {
                let mut layout = Layout::default();
                let changes = layout.open(PaneKind::Changes);
                let first = layout.add(PaneKind::Terminal);
                let second = layout.add(PaneKind::Terminal);
                layout.move_pane(first, Target::Beside(changes, Edge::Bottom));
                layout.move_pane(second, Target::With(first));
                layout.set_hidden(Dock::Bottom, true);
                panes_scene(layout, &[changes, first, second], true, cx)
            },
        },
        Scene {
            name: "model-menu",
            description: "The model menu open over the composer, two connections.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        model: "claude-opus-5-5".into(),
                        menu: Some(fixtures::model_menu()),
                        ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer,
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "slash-menu",
            description: "The / menu over a chat's composer, following /re.",
            size: WINDOW,
            build: |_, cx| slash_scene(Some(fixtures::slash_rows()), "/re", None, cx),
        },
        Scene {
            name: "slash-menu-empty",
            description: "The / menu when nothing matches.",
            size: WINDOW,
            build: |_, cx| slash_scene(Some(Vec::new()), "/zz", None, cx),
        },
        Scene {
            name: "composer-command-hint",
            description: "A bare /command, with what it takes under the field.",
            size: WINDOW,
            build: |_, cx| slash_scene(None, "/pdf", Some("[file]  ·  added after the skill's instructions"), cx),
        },
        Scene {
            name: "transcript-command",
            description: "A chat opened with a skill: the /pdf bubble and the skill it used.",
            size: WINDOW,
            build: |_, cx| command_scene(false, cx),
        },
        Scene {
            name: "transcript-command-prompt",
            description: "The same chat with the prompt sent to the agent open under the bubble.",
            size: WINDOW,
            build: |_, cx| command_scene(true, cx),
        },
        Scene {
            name: "composer-modes",
            description: "The composer in Build, Plan and Ultra Plan, with Ultra effort, and locked by a run.",
            size: size(px(760.), px(700.)),
            build: |_, cx| {
                use wack_core::AgentMode;
                let drawn = |mode| Input::Drawn { text: None, placeholder: crate::app::placeholder(mode, false).into() };
                let composer = |mode: AgentMode| ComposerData { mode, input: drawn(mode), ..fixtures::composer_after_turn(drawn(mode)) };
                let rows = vec![
                    ("Build", composer(AgentMode::Build)),
                    ("Plan", composer(AgentMode::Plan)),
                    ("Ultra Plan, Ultra effort", ComposerData { effort: Some("Ultra".into()), ultra_effort: true, ..composer(AgentMode::UltraPlan) }),
                    ("Ultra Plan while working", ComposerData { working: true, ..composer(AgentMode::UltraPlan) }),
                ];
                cx.new(|_| ComposerSheet { rows }).into()
            },
        },
        Scene {
            name: "effort-slider",
            description: "The effort slider at High, opened from the effort's pill in the smallest window.",
            size: size(px(716.), px(720.)),
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        effort: Some("High".into()),
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::High),
                        }),
                        ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer,
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "effort-slider-default",
            description: "The effort slider at Default: the provider's own effort, no fill.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        effort: Some("Default effort".into()),
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::Off),
                        }),
                        ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer,
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "effort-menu",
            description: "The effort slider with Ultra chosen, its glyph field at rest, over a chat in Ultra Plan.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let mode = wack_core::AgentMode::UltraPlan;
                    let drawn = Input::Drawn { text: None, placeholder: crate::app::placeholder(mode, false).into() };
                    let composer = ComposerData {
                        mode,
                        effort: Some("Ultra".into()),
                        ultra_effort: true,
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::Ultra),
                        }),
                        ..fixtures::composer_after_turn(drawn)
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer,
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "new-chat-ultra-plan",
            description: "The new-chat screen in Ultra Plan, over a backdrop.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    let mode = wack_core::AgentMode::UltraPlan;
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: ComposerData {
                                mode,
                                input: Input::Drawn { text: None, placeholder: crate::app::placeholder(mode, true).into() },
                                ..fixtures::composer_empty(true)
                            },
                            notice: None,
                        },
                        fixtures::strip(None),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "question",
            description: "Plan mode asking two questions, the first answered, with an earlier answer opened in the work log.",
            size: WINDOW,
            build: |_, cx| question_scene(false, None, WINDOW, cx),
        },
        Scene {
            name: "question-ultra",
            description: "Ultra Plan's question card, with Write the plan now, over a backdrop.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                question_scene(true, Some(picture), WINDOW, cx)
            },
        },
        Scene {
            name: "question-small",
            description: "The question card in the smallest window the app allows.",
            size: size(px(716.), px(492.)),
            build: |_, cx| question_scene(false, None, size(px(716.), px(492.)), cx),
        },
        Scene {
            name: "plan-card",
            description: "A plan awaiting review in Plan mode.",
            size: WINDOW,
            build: |_, cx| plan_scene(false, false, None, cx),
        },
        Scene {
            name: "plan-card-ultra",
            description: "A plan from Ultra Plan awaiting review, over a backdrop.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                plan_scene(true, false, Some(picture), cx)
            },
        },
        Scene {
            name: "plan-card-discard",
            description: "Discard pressed: the plan card asks in its footer.",
            size: WINDOW,
            build: |_, cx| plan_scene(false, true, None, cx),
        },
        Scene {
            name: "todo-list",
            description: "A Build run keeping a todo list, opened from its pill: one task folded, one under way, dependencies shown.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        working: true,
                        todos: Some(fixtures::todos()),
                        todos_open: true,
                        ..fixtures::composer_working()
                    };
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::working_turn()), composer },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "todo-queue-small",
            description: "A todo list and a long queue, both opened, in the smallest window: the queue scrolls.",
            size: size(px(716.), px(492.)),
            build: |_, cx| {
                cx.new(|_| {
                    let waiting = (1..=6)
                        .map(|n| crate::composer::Waiting { text: format!("Queued message number {n}, waiting for the run.").into(), steering: n == 1 })
                        .collect();
                    let composer = ComposerData {
                        working: true,
                        todos: Some(fixtures::todos()),
                        waiting,
                        todos_open: true,
                        waiting_open: true,
                        ..fixtures::composer_working()
                    };
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::working_turn()), composer },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "todo-question-small",
            description: "A todo list, a question and a queued message in the smallest window.",
            size: size(px(716.), px(492.)),
            build: |_, cx| {
                cx.new(|_| {
                    let mode = wack_core::AgentMode::Plan;
                    let todos = fixtures::todos();
                    let composer = ComposerData {
                        mode,
                        working: true,
                        question: Some(fixtures::question(false)),
                        todos: Some(todos),
                        todos_open: true,
                        ..fixtures::composer_working()
                    };
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::asking_turn()), composer },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "settings-providers",
            description: "Settings › Providers: two connections, Local switched off, the overview.",
            size: size(px(1180.), px(820.)),
            build: |_, cx| providers_scene(Providers::Listed, false, cx),
        },
        Scene {
            name: "settings-providers-titles",
            description: "Settings › Providers › Automatic titles: on, with the title model list open.",
            size: size(px(1180.), px(1100.)),
            build: |_, cx| providers_scene(Providers::Titles, false, cx),
        },
        Scene {
            name: "chat-title-arriving",
            description: "The model's title arriving: the stand-in leaves the tab to the left, the new one comes in from the right; the sidebar row crossfades.",
            size: WINDOW,
            build: |_, cx| title_scene(TitleScene::Swap(90), cx),
        },
        Scene {
            name: "chat-title-glint",
            description: "The model's title settled on its tab, the accent glint passing over it.",
            size: WINDOW,
            build: |_, cx| title_scene(TitleScene::Swap(620), cx),
        },
        Scene {
            name: "chat-title-glint-backdrop",
            description: "The title glint over a backdrop.",
            size: WINDOW,
            build: |_, cx| title_scene(TitleScene::Backdrop, cx),
        },
        Scene {
            name: "chat-rename-tab",
            description: "Renaming a chat on its tab (double-click, or File › Rename Chat…).",
            size: WINDOW,
            build: |_, cx| title_scene(TitleScene::RenameTab, cx),
        },
        Scene {
            name: "chat-rename-sidebar",
            description: "Renaming a chat on its sidebar row (double-click).",
            size: WINDOW,
            build: |_, cx| title_scene(TitleScene::RenameSidebar, cx),
        },
        Scene {
            name: "settings-providers-form",
            description: "Settings › Providers' new-connection editor, with two models and a problem to fix.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| providers_scene(Providers::Adding, false, cx),
        },
        Scene {
            name: "settings-providers-edit",
            description: "Settings › Providers editing a saved connection: its fields, two models, its switch.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| providers_scene(Providers::Editing, false, cx),
        },
        Scene {
            name: "settings-providers-empty",
            description: "Settings › Providers with no connections yet: the first-run tile.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| providers_scene(Providers::Empty, false, cx),
        },
        Scene {
            name: "settings-providers-small",
            description: "The smallest window with the sidebar away: the sections become a row of pills above the page.",
            size: size(px(716.), px(720.)),
            build: |_, cx| providers_scene(Providers::Listed, true, cx),
        },
        Scene {
            name: "settings-providers-form-small",
            description: "The connection editor in the smallest window: the fields fit, the save bar stays at the foot.",
            size: size(px(716.), px(900.)),
            build: |_, cx| providers_scene(Providers::Adding, true, cx),
        },
        Scene {
            name: "settings-tools",
            description: "Settings › Tools, asking before planning loses its read-only restrictions.",
            size: size(px(1180.), px(760.)),
            build: |_, cx| {
                let form = ProviderForm::new(cx);
                let data = SettingsData {
                    section: Section::Tools,
                    providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
                    appearance: fixtures::appearance(None, cx),
                    tools: crate::tools_page::ToolsData { unrestricted_planning: false, confirming: true },
                    memory: Default::default(),
                };
                cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)))).into()
            },
        },
        Scene {
            name: "settings-memory",
            description: "Settings › Memory: a project with notes of every kind, one switched off, and one waiting for a first note.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| memory_scene(MemoryScene::Notes, None, cx),
        },
        Scene {
            name: "settings-memory-backdrop",
            description: "Settings › Memory over a backdrop.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| memory_scene(MemoryScene::Notes, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-memory-empty",
            description: "Settings › Memory before the agent has saved anything: how memory works, in three steps.",
            size: WINDOW,
            build: |_, cx| memory_scene(MemoryScene::Empty, None, cx),
        },
        Scene {
            name: "settings-memory-off",
            description: "Settings › Memory with the master switch off: every switch reads off and the page says the tools are withdrawn.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| memory_scene(MemoryScene::Off, None, cx),
        },
        Scene {
            name: "settings-memory-confirm",
            description: "Settings › Memory asking before a note moves to the Trash.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| memory_scene(MemoryScene::Confirm, None, cx),
        },
        Scene {
            name: "settings-memory-editor",
            description: "The memory editor, writing a new note: its name follows the title.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| memory_scene(MemoryScene::Editor, None, cx),
        },
        Scene {
            name: "settings-appearance",
            description: "Settings › Appearance in the theme it is drawn with, and no picture yet.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| settings_scene(None, cx),
        },
        Scene {
            name: "settings-appearance-backdrop",
            description: "Settings › Appearance over a backdrop: a library of two pictures, each captioned with its theme.",
            size: size(px(1440.), px(900.)),
            build: |_, cx| settings_scene(Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "new-chat-backdrop",
            description: "The new-chat screen over a backdrop: the picture clear across the stage, the small words on frosted pills.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: None,
                        },
                        fixtures::strip(None),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "welcome-backdrop",
            description: "First launch over a backdrop: the picture clear across the stage.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| Shell::showing(Default::default(), Screen::Welcome { notice: None }).with_backdrop(picture)).into()
            },
        },
        Scene {
            name: "new-chat-backdrop-ascii",
            description: "The new-chat screen over a backdrop drawn through the ASCII texture, with a notice under the composer.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::Ascii), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: Some("Add a connection in Settings › Providers to start chatting.".into()),
                        },
                        fixtures::strip(None),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "chat-backdrop",
            description: "A finished turn over a backdrop, at a laptop's full-screen size.",
            size: size(px(1512.), px(945.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "specimen",
            description: "Every text style and colour token in the current theme.",
            size: size(px(1180.), px(960.)),
            build: |_, cx| cx.new(|_| Specimen).into(),
        },
        Scene {
            name: "build-run-buttons",
            description: "A chat's title strip with configured Build and active Run split buttons.",
            size: WINDOW,
            build: |_, cx| {
                let mut strip = fixtures::strip(Some(fixtures::FINISHED_TITLE));
                strip.commands = Some([
                    CommandButtonData::idle(CommandKind::Build, Some("cargo check --workspace")),
                    CommandButtonData {
                        kind: CommandKind::Run,
                        command: Some("cargo run --bin dev-server".into()),
                        status: CommandStatus::Running,
                        editor_open: false,
                    },
                ]);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        strip,
                    )
                })
                .into()
            },
        },
        Scene {
            name: "build-run-new-chat",
            description: "New chat screen with centered Build/Run pills and an open Run output dock.",
            size: WINDOW,
            build: |_, cx| {
                let mut strip = fixtures::strip(None);
                strip.commands = Some([
                    CommandButtonData::idle(CommandKind::Build, Some("cargo check --workspace")),
                    CommandButtonData {
                        kind: CommandKind::Run,
                        command: Some("cargo run --bin dev-server".into()),
                        status: CommandStatus::Running,
                        editor_open: false,
                    },
                ]);
                cx.new(|cx| {
                    let mut layout = Layout::default();
                    let run_id = layout.open(PaneKind::Run);
                    let mut panes = ChatPanes::new(layout, "/Users/you/Code/wackcode".into());
                    let session = CommandSession::fixture(
                        CommandKind::Run,
                        "/Users/you/Code/wackcode",
                        "cargo run --bin dev-server",
                        CommandStatus::Running,
                        120,
                        18,
                        fixtures::RUN_SESSION.as_bytes(),
                    );
                    let model = PaneModel::Run(cx.new(|cx| {
                        RunPane::fixture(
                            CommandKind::Run,
                            "/Users/you/Code/wackcode",
                            Some("cargo run --bin dev-server"),
                            Some(session),
                            cx,
                        )
                    }));
                    panes.insert(run_id, model, cx);
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: None,
                        },
                        strip,
                    )
                    .with_panes(panes, false)
                })
                .into()
            },
        },
        Scene {
            name: "build-run-editor",
            description: "The Build command configuration popover anchored below the Build split pill.",
            size: WINDOW,
            build: |_, cx| {
                let input = cx.new(|cx| {
                    let mut field = TextField::new(Type::Mono, "cargo build", cx);
                    field.set_text("cargo build --workspace", cx);
                    field
                });
                let mut strip = fixtures::strip(Some(fixtures::FINISHED_TITLE));
                strip.commands = Some([
                    CommandButtonData {
                        kind: CommandKind::Build,
                        command: Some("cargo check --workspace".into()),
                        status: CommandStatus::Failed { exit_code: Some(101), signal: None },
                        editor_open: true,
                    },
                    CommandButtonData::idle(CommandKind::Run, None),
                ]);
                strip.command_editor = Some(CommandEditorData {
                    kind: CommandKind::Build,
                    project_name: fixtures::project().name,
                    project_path: fixtures::project().path,
                    input,
                    has_session: true,
                    active: false,
                    can_run: true,
                    error: None,
                });
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        strip,
                    )
                })
                .into()
            },
        },
        Scene {
            name: "build-run-panes",
            description: "Failed Build and active Run panes stacked in the bottom dock beside Changes.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                let mut strip = fixtures::strip(Some(fixtures::FINISHED_TITLE));
                strip.commands = Some([
                    CommandButtonData {
                        kind: CommandKind::Build,
                        command: Some("cargo check --workspace".into()),
                        status: CommandStatus::Failed { exit_code: Some(101), signal: None },
                        editor_open: false,
                    },
                    CommandButtonData {
                        kind: CommandKind::Run,
                        command: Some("cargo run --bin dev-server".into()),
                        status: CommandStatus::Running,
                        editor_open: false,
                    },
                ]);
                cx.new(|cx| {
                    let mut layout = Layout::default();
                    let changes_id = layout.open(PaneKind::Changes);
                    let run_id = layout.open(PaneKind::Run);
                    let build_id = layout.open(PaneKind::Build);
                    layout.right_width = Some(520.);
                    layout.bottom_height = Some(280.);
                    let mut panes = ChatPanes::new(layout, "/Users/you/Code/wackcode".into());
                    panes.insert(
                        changes_id,
                        PaneModel::Changes(cx.new(|cx| {
                            ChangesPane::fixture(fixtures::changed_files(), fixtures::CHANGES_SELECTED, fixtures::CHANGES_DIFF, cx)
                        })),
                        cx,
                    );
                    let run_session = CommandSession::fixture(
                        CommandKind::Run,
                        "/Users/you/Code/wackcode",
                        "cargo run --bin dev-server",
                        CommandStatus::Running,
                        120,
                        18,
                        fixtures::RUN_SESSION.as_bytes(),
                    );
                    panes.insert(
                        run_id,
                        PaneModel::Run(cx.new(|cx| {
                            RunPane::fixture(
                                CommandKind::Run,
                                "/Users/you/Code/wackcode",
                                Some("cargo run --bin dev-server"),
                                Some(run_session),
                                cx,
                            )
                        })),
                        cx,
                    );
                    let build_session = CommandSession::fixture(
                        CommandKind::Build,
                        "/Users/you/Code/wackcode",
                        "cargo check --workspace",
                        CommandStatus::Failed { exit_code: Some(101), signal: None },
                        120,
                        18,
                        fixtures::BUILD_SESSION.as_bytes(),
                    );
                    panes.insert(
                        build_id,
                        PaneModel::Build(cx.new(|cx| {
                            RunPane::fixture(
                                CommandKind::Build,
                                "/Users/you/Code/wackcode",
                                Some("cargo check --workspace"),
                                Some(build_session),
                                cx,
                            )
                        })),
                        cx,
                    );
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        strip,
                    )
                    .with_panes(panes, false)
                })
                .into()
            },
        },
        Scene {
            name: "build-run-narrow",
            description: "Smallest window (716pt): compact icon-only Build and Run strip buttons.",
            size: size(px(716.), px(720.)),
            build: |_, cx| {
                let mut strip = fixtures::strip(Some(fixtures::FINISHED_TITLE));
                strip.commands = Some([
                    CommandButtonData::idle(CommandKind::Build, Some("cargo check")),
                    CommandButtonData {
                        kind: CommandKind::Run,
                        command: Some("cargo run".into()),
                        status: CommandStatus::Running,
                        editor_open: false,
                    },
                ]);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        strip,
                    )
                })
                .into()
            },
        },
    ]
}

/// A chat whose last run proposed a plan.
fn plan_scene(ultra: bool, confirming: bool, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let mode = if ultra { wack_core::AgentMode::UltraPlan } else { wack_core::AgentMode::Plan };
    cx.new(|_| {
        let composer = ComposerData {
            mode,
            ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::app::placeholder(mode, false).into() })
        };
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::plan_turn(ultra, confirming)),
                composer,
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        );
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// A chat in Plan or Ultra Plan with the question card open.
fn question_scene(ultra: bool, backdrop: Option<crate::backdrop::Picture>, _size: Size<Pixels>, cx: &mut App) -> AnyView {
    let mode = if ultra { wack_core::AgentMode::UltraPlan } else { wack_core::AgentMode::Plan };
    cx.new(|_| {
        let composer = ComposerData {
            mode,
            working: true,
            question: Some(fixtures::question(ultra)),
            ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::app::placeholder(mode, false).into() })
        };
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::asking_turn()), composer },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        );
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// `picture`, with the theme made ready for text over it, as the app's theme is.
fn over(picture: crate::backdrop::Picture, cx: &mut App) -> crate::backdrop::Picture {
    let input = crate::theme::ThemeInput { ground: Some(picture.ground()), ..crate::theme::Theme::get(cx).input.clone() };
    cx.set_global(crate::theme::Theme::derive(input));
    picture
}

#[derive(Clone, Copy, PartialEq)]
enum Providers {
    Listed,
    /// Automatic titles on, with the model list open.
    Titles,
    Adding,
    Editing,
    Empty,
}

/// Settings › Providers in one of its states.
fn providers_scene(state: Providers, sidebar_hidden: bool, cx: &mut App) -> AnyView {
    let mut form = ProviderForm::new(cx);
    let mut connections = fixtures::connections();
    connections[1].enabled = false;
    match state {
        Providers::Listed | Providers::Titles => {}
        Providers::Adding => {
            form.open_new(cx);
            form.name.update(cx, |field, cx| field.set_text("OpenRouter", cx));
            form.address.update(cx, |field, cx| field.set_text("openrouter.ai/api/v1", cx));
            form.key.update(cx, |field, cx| field.set_text("not-a-real-key", cx));
            form.models[0].id.update(cx, |field, cx| field.set_text("anthropic/claude-sonnet-5.5", cx));
            form.models[0].context.update(cx, |field, cx| field.set_text("200k", cx));
            form.add_model("google/gemini-3-pro-preview", Some("1M"), cx);
            form.error = Some("The address must start with https:// or http://.".into());
        }
        Providers::Editing => form.edit(&connections[0].clone(), cx),
        Providers::Empty => connections.clear(),
    }
    let data = SettingsData {
        section: Section::Providers,
        providers: ProvidersData::new(&connections, |id| id != "c2", &form, cx).with_titles(match state {
            Providers::Titles => {
                let config = wack_core::AutoTitleConfig { enabled: true, connection: Some("c1".into()), model: Some("claude-sonnet-5-5".into()) };
                crate::titles_card::TitlesData::new(&config, &connections, true, None)
            }
            _ => crate::titles_card::TitlesData::new(&Default::default(), &connections, false, None),
        }),
        appearance: fixtures::appearance(None, cx),
        tools: Default::default(),
        memory: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        if sidebar_hidden { shell.without_sidebar() } else { shell }
    })
    .into()
}

/// Settings › Appearance, drawn over `backdrop` when there is one.
fn settings_scene(backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Appearance,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        tools: Default::default(),
        memory: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// The finished chat with `layout`'s panes showing fixture models.
fn panes_scene(layout: Layout, ids: &[u32], sidebar_hidden: bool, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut panes = ChatPanes::new(layout, "/".into());
        for id in ids {
            let kind = panes.layout.panes().iter().find(|(_, pane)| pane.id == *id).map(|(_, pane)| pane.kind);
            let model = match kind {
                Some(PaneKind::Changes) => PaneModel::Changes(cx.new(|cx| {
                    ChangesPane::fixture(fixtures::changed_files(), fixtures::CHANGES_SELECTED, fixtures::CHANGES_DIFF, cx)
                })),
                _ => PaneModel::Terminal(cx.new(|cx| {
                    TerminalPane::fixture(fixtures::TERMINAL_SESSION.as_bytes(), 140, 22, None, cx)
                })),
            };
            panes.insert(*id, model, cx);
        }
        Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::finished_turn()),
                composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        )
        .with_panes(panes, sidebar_hidden)
    })
    .into()
}

/// Composers in a column, each under a caption: for comparing one control's states.
struct ComposerSheet {
    rows: Vec<(&'static str, ComposerData)>,
}

impl Render for ComposerSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::typography::Typography;
        let theme = crate::theme::Theme::get(cx).clone();
        gpui::div().size_full().bg(theme.window).p(px(32.)).flex().flex_col().gap(px(22.)).children(self.rows.iter().map(|(caption, data)| {
            gpui::div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(gpui::div().typography(Type::Label).text_color(theme.text_dim).child(*caption))
                .child(crate::composer::Composer::new(data.clone()))
        }))
    }
}

pub fn find(name: &str) -> Option<Scene> {
    all().into_iter().find(|scene| scene.name == name)
}

/// Wraps a scene's view so any scene can be the root of a window.
pub struct SceneRoot {
    view: AnyView,
}

impl SceneRoot {
    pub fn new(view: AnyView) -> Self {
        Self { view }
    }
}

impl Render for SceneRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tags::root(self.view.clone())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MemoryScene {
    Notes,
    Empty,
    Off,
    Confirm,
    Editor,
}

/// Settings › Memory with fixture notes, in the state `scene` names.
fn memory_scene(scene: MemoryScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use wack_core::memory::{Kind, MemoryOverview, MemoryProject, Note};

    use crate::memory_page::{MemoryConfirm, MemoryEditor, MemoryState};

    let note = |kind: Kind, name: &str, title: &str, description: &str, modified: &str| Note {
        name: name.into(),
        kind,
        title: title.into(),
        description: description.into(),
        modified: Some(modified.into()),
        body: String::new(),
    };
    let today = wack_core::memory::now_iso();
    let folder = |name: &str| std::path::PathBuf::from(format!("/Users/you/Library/Application Support/com.wackcode.desktop/v3/memory/{name}"));
    let projects = vec![
        MemoryProject {
            key: "3f2a9c1b7e44".into(),
            name: "wackcode".into(),
            path: "/Users/you/Code/wackcode".into(),
            dir: folder("wackcode-3f2a9c1b7e44"),
            notes: vec![
                note(Kind::User, "user_prefers-short-answers", "Prefers short answers", "Lead with the outcome; no recaps", &today),
                note(Kind::Feedback, "feedback_run-the-gallery", "Check UI changes in the gallery", "Render the scene before calling a view done", "2026-11-12T09:30:00Z"),
                note(Kind::Project, "project_memory-port", "Memory is being ported from version 2", "", "2026-11-03T16:05:00Z"),
                note(Kind::Reference, "reference_testing-connection", "The Testing connection", "The only connection agents may use", "2026-10-21T08:00:00Z"),
            ],
            enabled: true,
        },
        MemoryProject {
            key: "8b1d0e6f2a90".into(),
            name: "tokentrail".into(),
            path: "/Users/you/Code/tokentrail".into(),
            dir: folder("tokentrail-8b1d0e6f2a90"),
            notes: vec![note(Kind::Feedback, "feedback_ledger-is-append-only", "The usage ledger is append-only", "Never rewrite a line once written", "2026-11-14T18:40:00Z")],
            enabled: false,
        },
        MemoryProject {
            key: "c0ffee123456".into(),
            name: "scratch".into(),
            path: "/Users/you/Code/scratch".into(),
            dir: folder("scratch-c0ffee123456"),
            notes: Vec::new(),
            enabled: true,
        },
    ];
    let mut state = MemoryState {
        overview: MemoryOverview { enabled: scene != MemoryScene::Off, projects },
        ..Default::default()
    };
    match scene {
        MemoryScene::Empty => state.overview.projects.retain(|project| project.notes.is_empty()),
        MemoryScene::Confirm => {
            let project = &state.overview.projects[0];
            state.confirming = Some(MemoryConfirm::Note {
                dir: project.dir.clone(),
                name: project.notes[2].name.clone(),
                title: project.notes[2].title.clone(),
            });
        }
        MemoryScene::Editor => {
            let editor = MemoryEditor::new(&state.overview.projects[0], None, cx);
            editor.title.update(cx, |field, cx| field.set_text("Run clippy before committing", cx));
            editor.description.update(cx, |field, cx| field.set_text("Warnings fail review", cx));
            let name = editor.suggested_name(cx);
            editor.name.update(cx, |field, cx| field.set_text(&name, cx));
            editor.body.update(cx, |field, cx| {
                field.set_text("cargo clippy --workspace must be clean before a commit.\nThe owner treats a new warning as a failed review.", cx)
            });
            state.editor = Some(editor);
        }
        MemoryScene::Notes | MemoryScene::Off => {}
    }
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Memory,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        tools: Default::default(),
        memory: state.data(cx),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

enum TitleScene {
    /// The swap this many milliseconds after the model's title landed.
    Swap(u64),
    Backdrop,
    RenameTab,
    RenameSidebar,
}

/// The finished chat with its title changing: arriving from the model, or being renamed.
fn title_scene(state: TitleScene, cx: &mut App) -> AnyView {
    use crate::{motion::SwapLook, sidebar::TitleLook};
    let stand_in = "Can you look at why the SSE decoder drops the last event when…";
    let mut sidebar = fixtures::sidebar(Selected::Finished);
    let mut strip = fixtures::strip(Some(fixtures::FINISHED_TITLE));
    let row = sidebar.projects.iter_mut().chain(sidebar.pinned.iter_mut()).flat_map(|project| project.chats.iter_mut()).find(|row| row.selected);
    let tab = strip.tabs.iter_mut().find(|tab| tab.active);
    let swap = |ms: u64| SwapLook::at(stand_in.into(), std::time::Duration::from_millis(ms), false);
    let mut backdrop = None;
    match state {
        TitleScene::Swap(ms) => {
            if let Some(row) = row {
                row.look = TitleLook::Swap(swap(ms));
            }
            if let Some(tab) = tab {
                tab.look = TitleLook::Swap(swap(ms));
            }
        }
        TitleScene::Backdrop => {
            backdrop = Some(over(fixtures::backdrop(wack_core::Texture::None), cx));
            if let Some(tab) = tab {
                tab.look = TitleLook::Swap(swap(620));
            }
        }
        TitleScene::RenameTab | TitleScene::RenameSidebar => {
            let field = cx.new(|cx| {
                let mut field = TextField::new(Type::Ui, "Chat name", cx);
                field.set_text(fixtures::FINISHED_TITLE, cx);
                field.select_all_text(cx);
                field
            });
            match (state, row, tab) {
                (TitleScene::RenameSidebar, Some(row), _) => row.look = TitleLook::Editing(field),
                (_, _, Some(tab)) => tab.look = TitleLook::Editing(field),
                _ => {}
            }
        }
    }
    cx.new(|_| {
        let shell = Shell::showing_tabs(
            sidebar,
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::finished_turn()),
                composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
            },
            strip,
        );
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// A chat whose composer holds `text`, with the `/` menu showing `rows` (when some) or a hint.
fn slash_scene(rows: Option<Vec<crate::slash_menu::SlashRow>>, text: &'static str, hint: Option<&'static str>, cx: &mut App) -> AnyView {
    cx.new(|_| {
        let composer = ComposerData {
            slash: rows.map(|rows| crate::slash_menu::SlashData { rows, selected: 0, scroll: gpui::ScrollHandle::new() }),
            hint: hint.map(Into::into),
            ..fixtures::composer_after_turn(Input::Drawn { text: Some(text.into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
        };
        Shell::showing(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat { title: fixtures::FINISHED_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::finished_turn()), composer },
        )
    })
    .into()
}

fn command_scene(open: bool, cx: &mut App) -> AnyView {
    cx.new(|_| {
        Shell::showing(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: "Fill in the W-9".into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::skill_turn(open)),
                composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
            },
        )
    })
    .into()
}
