//! Named UI scenes: a view in a known state at a known size.
//!
//! The `gallery` binary renders these to PNG files, which is how UI work is checked without
//! launching the app. Add a scene for every component state worth looking at.

use gpui::{AnyView, App, Context, Pixels, Size, Window, prelude::*, px, size};

use crate::{
    composer::{ComposerData, Input},
    panes::{ChangesPane, ChatPanes, Dock, Edge, Layout, PaneKind, PaneModel, Target, TerminalPane},
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
            name: "effort-menu",
            description: "The effort menu with Ultra chosen, over a chat in Ultra Plan.",
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
                            groups: crate::shell::effort_groups(Some(wack_core::Reasoning::Ultra)),
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
            description: "A Build run keeping a todo list: one task folded, one under way, dependencies shown.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        working: true,
                        todos: Some(fixtures::todos()),
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
            description: "A todo list and a long queue in the smallest window: the queue scrolls.",
            size: size(px(716.), px(492.)),
            build: |_, cx| {
                cx.new(|_| {
                    let waiting = (1..=6)
                        .map(|n| crate::composer::Waiting { text: format!("Queued message number {n}, waiting for the run.").into(), steering: n == 1 })
                        .collect();
                    let composer = ComposerData { working: true, todos: Some(fixtures::todos()), waiting, ..fixtures::composer_working() };
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
                    let mut todos = fixtures::todos();
                    todos.folded = true;
                    let composer = ComposerData {
                        mode,
                        working: true,
                        question: Some(fixtures::question(false)),
                        todos: Some(todos),
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
            description: "Settings › Providers: two connections and the add form with a problem to fix.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| {
                let mut form = ProviderForm::new(cx);
                form.name.update(cx, |field, cx| field.set_text("OpenRouter", cx));
                form.address.update(cx, |field, cx| field.set_text("openrouter.ai/api/v1", cx));
                form.key.update(cx, |field, cx| field.set_text("not-a-real-key", cx));
                form.error = Some("The address must start with https:// or http://.".into());
                let data = SettingsData {
                    section: Section::Providers,
                    providers: ProvidersData::new(&fixtures::connections(), &form),
                    appearance: fixtures::appearance(None, cx),
                    tools: Default::default(),
                };
                cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(data))).into()
            },
        },
        Scene {
            name: "settings-tools",
            description: "Settings › Tools, asking before planning loses its read-only restrictions.",
            size: size(px(1180.), px(760.)),
            build: |_, cx| {
                let form = ProviderForm::new(cx);
                let data = SettingsData {
                    section: Section::Tools,
                    providers: ProvidersData::new(&fixtures::connections(), &form),
                    appearance: fixtures::appearance(None, cx),
                    tools: crate::tools_page::ToolsData { unrestricted_planning: false, confirming: true },
                };
                cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(data))).into()
            },
        },
        Scene {
            name: "settings-appearance",
            description: "Settings › Appearance with the default colours and no backdrop yet.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| settings_scene(None, cx),
        },
        Scene {
            name: "settings-appearance-backdrop",
            description: "Settings › Appearance with a backdrop chosen, drawn over it.",
            size: size(px(1440.), px(900.)),
            build: |_, cx| settings_scene(Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "new-chat-backdrop",
            description: "The new-chat screen over a backdrop: the picture clear across the stage, glass only around the heading.",
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

/// Settings › Appearance, drawn over `backdrop` when there is one.
fn settings_scene(backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Appearance,
        providers: ProvidersData::new(&fixtures::connections(), &form),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        tools: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(data));
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
