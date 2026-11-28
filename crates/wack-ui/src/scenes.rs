//! Named UI scenes: a view in a known state at a known size.
//!
//! The `gallery` binary renders these to PNG files, which is how UI work is checked without
//! launching the app. Add a scene for every component state worth looking at.

use gpui::{AnyView, App, Context, Pixels, Size, Window, prelude::*, px, size};
use wack_core::{CommandKind, CommandSession, CommandStatus};

use crate::{
    composer::{ComposerData, Input},
    panes::{AgentsPane, BrowserPane, ChangesPane, ChatPanes, Edge, Layout, PaneKind, PaneModel, RunPane, Target, TasksPane, TerminalPane},
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
            name: "new-chat-project-picker",
            description: "The new-chat screen with its project list open under the project's name: pinned first, the current one ticked, Add project… at the foot.",
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
                        crate::shell::StripData { project_picker: Some(fixtures::picker_projects()), ..fixtures::strip(None) },
                    )
                })
                .into()
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
            name: "chat-area-hero",
            description: "The Chat area's new chat: the duck on its pond, the greeting, the composer with no mode track, three ideas.",
            size: WINDOW,
            build: |_, cx| cx.new(|_| Shell::showing_tabs(fixtures::chat_sidebar(false, false), fixtures::chat_hero(), fixtures::chat_strip(None))).into(),
        },
        Scene {
            name: "chat-area-hero-backdrop",
            description: "The Chat area's new chat over a backdrop: the picture clear, the small words and ideas on frosted pills, no pond.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| Shell::showing_tabs(fixtures::chat_sidebar(false, true), fixtures::chat_hero(), fixtures::chat_strip(None)).with_backdrop(picture))
                    .into()
            },
        },
        Scene {
            name: "chat-area-mention",
            description: "The Chat area's composer with `@li` typed: the picker lists the area's other chats under Chats, newest match first.",
            size: WINDOW,
            build: |_, cx| cx.new(|_| Shell::showing_tabs(fixtures::chat_sidebar(false, false), fixtures::chat_hero_mentioning(), fixtures::chat_strip(None))).into(),
        },
        Scene {
            name: "sidebar-search",
            description: "The Code sidebar searched for \"the\": only projects with matching chats, open; one row's Archive asking; the foot's Archived button.",
            size: WINDOW,
            build: |_, cx| {
                let sidebar = fixtures::sidebar_searching(cx);
                let screen = Screen::NewChat { project: fixtures::project(), composer: fixtures::composer_empty(true), notice: None };
                cx.new(|_| Shell::showing_tabs(sidebar, screen, fixtures::strip(None))).into()
            },
        },
        Scene {
            name: "chat-area-archived",
            description: "The Chat area's Archived view, one row asking before it deletes, and an archived chat open with Unarchive in the composer's place.",
            size: WINDOW,
            build: |_, cx| {
                let sidebar = fixtures::chat_archived_sidebar(cx);
                cx.new(|_| Shell::showing_tabs(sidebar, fixtures::archived_chat(), fixtures::chat_strip(None).without_tabs())).into()
            },
        },
        Scene {
            name: "chat-area-transcript",
            description: "A Chat chat as a messenger reads: a time divider, the duck and name over each reply, a folded run of calls, the answer in a bubble with Copy, the note it wrote, and a reply still coming (a live chip, the typing bubble).",
            size: WINDOW,
            build: |_, cx| {
                let screen = Screen::Chat {
                    title: fixtures::CHAT_AREA_TITLE.into(),
                    project: crate::shell::ProjectInfo { name: "".into(), path: "".into(), branch: None },
                    body: crate::shell::Body::Items(fixtures::chat_area_turns(&[])),
                    composer: ComposerData {
                        chat_area: true,
                        input: Input::Drawn { text: None, placeholder: crate::app::CHAT_FOLLOW_UP_PLACEHOLDER.into() },
                        activity: Some("Working for 6s".into()),
                        waiting: Vec::new(),
                        ..fixtures::composer_working()
                    },
                };
                cx.new(|_| Shell::showing_tabs(fixtures::chat_sidebar(true, false), screen, fixtures::chat_strip(Some(fixtures::CHAT_AREA_TITLE)))).into()
            },
        },
        Scene {
            name: "chat-area-empty",
            description: "The Chat area with no chats yet: the sidebar says what a chat here is.",
            size: WINDOW,
            build: |_, cx| {
                let sidebar = crate::sidebar::SidebarData { area: wack_core::ChatKind::Chat, ..Default::default() };
                cx.new(|_| Shell::showing_tabs(sidebar, fixtures::chat_hero(), fixtures::chat_strip(None).without_tabs())).into()
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
            name: "chat-computer",
            description: "The agent working in Calculator: each step as the app and its summary, the last still running.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::computer_turn()),
                            composer: fixtures::composer_working(),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-browser",
            description: "The agent driving the browser: each browser step in words, the last still running.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::browser_turn()),
                            composer: fixtures::composer_working(),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
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
            name: "chat-groups",
            description: "A finished turn as it reads: thought and reading folded into a line, the model's words between groups, edits and checks folded, the answer and its changes.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::grouped_turn(&[]), cx),
        },
        Scene {
            name: "chat-groups-open",
            description: "The same turn with its first group opened and its thought's body open: capped, scrolling inside.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::grouped_turn(&["turn-1", "turn-1-1"]), cx),
        },
        Scene {
            name: "chat-thinking",
            description: "The model reasoning now: the duck paddling, \"Thinking\" shining and how long, the line opening to its words.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::thinking_turn(&[]), cx),
        },
        Scene {
            name: "chat-thinking-open",
            description: "A live thought opened: its body holds to the newest reasoning, at a capped height.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::thinking_turn(&["turn-1-3"]), cx),
        },
        Scene {
            name: "chat-groups-backdrop",
            description: "The grouped turn over a backdrop, with a live thought below.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                let mut items = fixtures::grouped_turn(&[]);
                items.extend(fixtures::thinking_turn(&[]).into_iter().skip(1));
                let composer = fixtures::composer_after_turn(crate::composer::Input::Drawn { text: None, placeholder: "Reply, or ask for the next change".into() });
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(items), composer },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "chat-writing-plan",
            description: "A plan being written: the row says so, and how much has come.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::writing_plan_turn(), cx),
        },
        Scene {
            name: "chat-commits",
            description: "Commit milestone rows breaking work groups: a landed commit with its hash and ±stats, and a second commit expanded to its command and output.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::commits_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-commits-states",
            description: "Commit milestone edge states: a refused commit in the plain error look, a landed commit whose chained push failed, and a live commit running.",
            size: size(px(1180.), px(820.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::commits_states_turn()),
                            composer: ComposerData {
                                activity: Some("Working for 6s".into()),
                                waiting: Vec::new(),
                                ..fixtures::composer_working()
                            },
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-commits-backdrop",
            description: "Commit milestone rows over an image backdrop: collapsed and expanded milestones breaking work groups on glass.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::commits_turn()),
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
            name: "chat-turn-changes",
            description: "Per-turn Changes cards beneath settled turns: mixed create/edit with colliding folder hints, inline diff expanded, Show more toggle, and a Created card.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::turn_changes_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-turn-changes-backdrop",
            description: "Per-turn Changes cards over an image backdrop: expanded inline diff and collapsed created-files card on glass.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::turn_changes_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "chat-compaction",
            description: "Compaction summary cards in Code: a collapsed manual compaction boundary and an expanded mid-turn compaction with token counts, Markdown summary, files and quoted user messages.",
            size: size(px(1180.), px(1280.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::compaction_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-compaction-backdrop",
            description: "Compaction summary cards over an image backdrop: collapsed boundary and expanded summary card on glass.",
            size: size(px(1180.), px(1280.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::compaction_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "chat-area-compaction",
            description: "Chat area compaction summaries: a collapsed pill and an expanded summary sheet.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| {
                let screen = Screen::Chat {
                    title: fixtures::CHAT_AREA_TITLE.into(),
                    project: crate::shell::ProjectInfo { name: "".into(), path: "".into(), branch: None },
                    body: crate::shell::Body::Items(fixtures::chat_area_compaction_turns()),
                    composer: ComposerData {
                        chat_area: true,
                        input: Input::Drawn { text: None, placeholder: crate::app::CHAT_FOLLOW_UP_PLACEHOLDER.into() },
                        ..fixtures::composer_after_turn(Input::Drawn {
                            text: None,
                            placeholder: crate::app::CHAT_FOLLOW_UP_PLACEHOLDER.into(),
                        })
                    },
                };
                cx.new(|_| Shell::showing_tabs(fixtures::chat_sidebar(true, false), screen, fixtures::chat_strip(Some(fixtures::CHAT_AREA_TITLE)))).into()
            },
        },
        Scene {
            name: "suggestions",
            description: "The suggested-task tray under a settled turn: open, queued, started, opened-in and dismissed cards.",
            size: size(px(1180.), px(1240.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::suggestions_turn()),
                            composer: fixtures::composer_after_turn(crate::composer::Input::Drawn {
                                text: None,
                                placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                            }),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-panes",
            description: "A chat with a terminal beside the transcript, a tab of the right dock next to Changes, at a wide window.",
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
            name: "chat-panes-backdrop",
            description: "The side panel over a backdrop: Changes and a terminal as glass beside the stage, the frame's full height.",
            size: size(px(1512.), px(945.)),
            build: |_, cx| {
                let mut layout = Layout::default();
                let changes = layout.open(PaneKind::Changes);
                let terminal = layout.add(PaneKind::Terminal);
                layout.right_width = Some(520.);
                panes_scene_over(layout, &[changes, terminal], false, Some(fixtures::backdrop(wack_core::Texture::None)), cx)
            },
        },
        Scene {
            name: "chat-panes-bottom",
            description: "A terminal dragged under the transcript, inside the stage, beside Changes in the side panel.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                let mut layout = Layout::default();
                let changes = layout.open(PaneKind::Changes);
                let terminal = layout.add(PaneKind::Terminal);
                layout.move_pane(terminal, Target::Dock(crate::panes::Dock::Bottom));
                layout.right_width = Some(480.);
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
                        chat_area: false,
                        archived: None,
                        model: "claude-opus-5-5".into(),
                        brand: Some(crate::brand::Brand::Claude),
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
            name: "model-menu-team",
            description: "The model menu with the Team switch at its foot, Team off.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        model: "claude-opus-5-5".into(),
                        brand: Some(crate::brand::Brand::Claude),
                        menu: Some(fixtures::model_menu_with_team(false)),
                        ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat { title: fixtures::FINISHED_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::finished_turn()), composer },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "crew-picker",
            description: "A Team's crew picker open from the crew's half of the pill: every model, the crew's reasoning, and Switch Team off.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        model: "claude-opus-5-5".into(),
                        brand: Some(crate::brand::Brand::Claude),
                        crew: Some(crate::composer::CrewPill { model: "claude-haiku-5-5".into(), ..fixtures::crew_pill(false) }),
                        menu: Some(fixtures::crew_menu()),
                        ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                    };
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat { title: fixtures::FINISHED_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::finished_turn()), composer },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "composer-team",
            description: "The model's pill split into lead and crew: Team off, on, with the crew's own effort, a crew whose model is gone holding Send, and locked by a run.",
            size: size(px(760.), px(820.)),
            build: |_, cx| {
                let drawn = |text: Option<&str>| Input::Drawn { text: text.map(|text| text.to_string().into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let team = |crew| ComposerData { crew: Some(crew), ..fixtures::composer_after_turn(drawn(None)) };
                let rows = vec![
                    ("Team off", fixtures::composer_after_turn(drawn(None))),
                    ("Team on", team(fixtures::crew_pill(false))),
                    ("A crew with its own effort", team(crate::composer::CrewPill { effort: Some("Low".into()), ..fixtures::crew_pill(false) })),
                    ("A crew whose model is gone", ComposerData { crew: Some(fixtures::crew_pill(true)), ..fixtures::composer_after_turn(drawn(Some("Split the parser out of retry.rs"))) }),
                    ("Working", ComposerData { crew: Some(fixtures::crew_pill(false)), ..fixtures::composer_working() }),
                ];
                cx.new(|_| ComposerSheet { rows }).into()
            },
        },
        Scene {
            name: "model-logos",
            description: "All 25 model provider logos in light, dark and Mono, plus composer states for Logos On, custom model, and Logos Off.",
            size: size(px(820.), px(780.)),
            build: |_, cx| {
                let drawn = || Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let rows = vec![
                    (
                        "Logos on · recognised model (Claude)",
                        ComposerData { model: "claude-opus-5-5".into(), brand: Some(crate::brand::Brand::Claude), ..fixtures::composer_after_turn(drawn()) },
                    ),
                    (
                        "Logos on · unrecognised custom model (nothing beside name)",
                        ComposerData { model: "local-finetune-v2".into(), brand: None, ..fixtures::composer_after_turn(drawn()) },
                    ),
                    (
                        "Logos off · nothing beside name",
                        ComposerData { model: "GLM-5.3-Flash".into(), brand: None, ..fixtures::composer_after_turn(drawn()) },
                    ),
                ];
                cx.new(|_| ModelLogosSheet { rows }).into()
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
            name: "file-mentions",
            description: "Code's @ workspace file/folder picker over a chat with highlighted @path mentions, and the 20k-file truncation note.",
            size: WINDOW,
            build: |_, cx| file_mention_scene(fixtures::file_mention_data(true, None), None, cx),
        },
        Scene {
            name: "file-mentions-backdrop",
            description: "Code's @ workspace file/folder picker over a backdrop.",
            size: WINDOW,
            build: |_, cx| file_mention_scene(fixtures::file_mention_data(false, None), Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "file-mentions-error",
            description: "Code's @ workspace file picker when listing failed, with Retry.",
            size: WINDOW,
            build: |_, cx| file_mention_scene(fixtures::file_mention_data(false, Some("File mentions are off for a home folder. Open a project folder instead.")), None, cx),
        },
        Scene {
            name: "transcript-files",
            description: "Attached text files on a sent message: one collapsed chip and one expanded to show the exact text sent.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: "Fix SSE tail test".into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::files_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "terminal-selection",
            description: "A terminal with selected test output and its floating 'Add to draft' pill above the selection, plus a Terminal output chip in the composer.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| terminal_selection_scene(None, None, None, cx),
        },
        Scene {
            name: "terminal-selection-backdrop",
            description: "Terminal selection and floating 'Add to draft' pill over a backdrop picture.",
            size: size(px(1512.), px(945.)),
            build: |_, cx| terminal_selection_scene(None, None, Some(fixtures::backdrop(wack_core::Texture::None)), cx),
        },
        Scene {
            name: "terminal-context-menu",
            description: "Right-click context menu over a terminal selection, offering Copy, Add selection to draft, Paste, Select all, Clear screen, and Restart shell.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| terminal_selection_scene(Some(gpui::point(px(1080.), px(220.))), None, None, cx),
        },
        Scene {
            name: "transcript-pictures",
            description: "Pictures in a chat: two the user sent, the agent's screenshots under their step, and the one its reply shows.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: "Match the login mockups".into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::pictures_turn()),
                            composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "lightbox",
            description: "A screenshot open in the lightbox over the chat, second of three.",
            size: WINDOW,
            build: |_, cx| {
                let pictures: Vec<crate::pictures::Picture> =
                    fixtures::saved_pictures(3, "screenshot of http://localhost:3000/login").iter().map(crate::pictures::Picture::of_image).collect();
                let lightbox = crate::pictures::Lightbox::new(std::sync::Arc::new(pictures), 1);
                cx.new(|_| LightboxScene { lightbox }).into()
            },
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
            name: "composer-attachments",
            description: "Pictures, picked text files, terminal output chips, a model without Vision, and attached files past the cap holding Send.",
            size: size(px(760.), px(960.)),
            build: |_, cx| {
                use crate::attachments::FileAttachment;
                let drawn = |text: Option<&str>| Input::Drawn { text: text.map(|text| text.to_string().into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let composer = |text, count| ComposerData { attachments: fixtures::attachments(count), ..fixtures::composer_after_turn(drawn(text)) };
                let mixed_files = vec![
                    FileAttachment::file("Cargo.toml", "[package]\nname = \"wack-ui\"\nversion = \"3.0.0\"\n"),
                    FileAttachment::from_terminal("error[E0382]: borrow of moved value: `session`\n  --> src/app.rs:412:14\n", &[]).unwrap(),
                ];
                let paste_and_term = {
                    let paste = FileAttachment::from_paste(&"error: linking with `cc` failed\n".repeat(1_240), &[]).unwrap();
                    let term = FileAttachment::from_terminal("$ cargo test\ntest attachments ... ok\n", std::slice::from_ref(&paste)).unwrap();
                    vec![paste, term]
                };
                let rows = vec![
                    (
                        "Pictures, a text file, and terminal output",
                        ComposerData { files: mixed_files, ..composer(Some("Why did this test regress?"), 2) },
                    ),
                    (
                        "Long paste and terminal output",
                        ComposerData { files: paste_and_term, ..fixtures::composer_after_turn(drawn(Some("Compare the linker log with the test output."))) },
                    ),
                    (
                        "A model without Vision",
                        ComposerData {
                            chat_area: false,
                            archived: None,
                            pictures_refused: Some("GLM-5.3-Flash doesn't accept pictures. Turn on Vision for it in Settings › Providers, or remove the pictures.".into()),
                            ..composer(Some("What's wrong here?"), 1)
                        },
                    ),
                    (
                        "Attached text over the cap",
                        ComposerData {
                            files: vec![FileAttachment::file("build.log", "line\n".repeat(400))],
                            files_refused: Some("Attached text is larger than 512 KB. Remove some files to send.".into()),
                            ..fixtures::composer_after_turn(drawn(None))
                        },
                    ),
                ];
                cx.new(|_| ComposerSheet { rows }).into()
            },
        },
        Scene {
            name: "composer-states",
            description: "The composer's tab between runs (todos, commands, changes) and while working (the time, ⌘B), Queue beside Stop, and a long paste attached as a file.",
            size: size(px(760.), px(760.)),
            build: |_, cx| {
                let drawn = |text: Option<&str>| Input::Drawn { text: text.map(|text| text.to_string().into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let between = ComposerData {
                    chat_area: false,
                    archived: None,
                    tasks: crate::shell::task_lines(&fixtures::tasks()),
                    changes: Some(crate::chat::ChangeCount { files: 3, added: 42, removed: 7 }),
                    todos: Some(fixtures::todos()),
                    ..fixtures::composer_after_turn(drawn(None))
                };
                let working = ComposerData {
                    chat_area: false,
                    archived: None,
                    input: drawn(Some("Then add a line to CHANGELOG.md")),
                    foreground: true,
                    changes: Some(crate::chat::ChangeCount { files: 1, added: 3, removed: 1 }),
                    waiting: Vec::new(),
                    ..fixtures::composer_working()
                };
                let pasted = ComposerData {
                    chat_area: false,
                    archived: None,
                    files: vec![crate::attachments::FileAttachment::from_paste(&"error: linking with `cc` failed\n".repeat(1_240), &[]).unwrap()],
                    ..fixtures::composer_after_turn(drawn(Some("Why does the release build fail to link?")))
                };
                let rows = vec![("Between runs", between), ("Working, with words written", working), ("A long paste", pasted)];
                cx.new(|_| ComposerSheet { rows }).into()
            },
        },
        Scene {
            name: "composer-tab-crowded",
            description: "Everything on the composer's tab at once, in a wide and a narrow composer: the tab gives up words to stay inside the card.",
            size: size(px(760.), px(520.)),
            build: |_, cx| {
                let drawn = Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let crowded = ComposerData {
                    chat_area: false,
                    archived: None,
                    input: drawn,
                    activity: Some("Working for 1m 5s".into()),
                    crew_at_work: Some(crate::composer::CrewAtWork { working: 2, queued: 1, done: 2 }),
                    todos: Some(fixtures::todos()),
                    tasks: crate::shell::task_lines(&fixtures::tasks()),
                    changes: Some(crate::chat::ChangeCount { files: 12, added: 1_204, removed: 380 }),
                    foreground: true,
                    waiting: Vec::new(),
                    ..fixtures::composer_working()
                };
                let rows = vec![("Wide", crowded.clone()), ("Narrow", crowded)];
                cx.new(|_| ComposerSheet { rows }).into()
            },
        },
        Scene {
            name: "context-popover",
            description: "The ring round Send hovered: WackCode 2's context popover, every part with its share, and Compact.",
            size: size(px(760.), px(620.)),
            build: |_, cx| {
                let drawn = Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() };
                let tokens = fixtures::CONTEXT_WINDOW * 34 / 100;
                let composer = ComposerData {
                    chat_area: false,
                    archived: None,
                    meter: Some(crate::composer::Meter {
                        tokens,
                        window: Some(fixtures::CONTEXT_WINDOW),
                        high: false,
                        can_compact: true,
                        parts: fixtures::context_parts(tokens),
                        cache: Some(0.924),
                        spent: 3_840_000,
                    }),
                    context_open: true,
                    ..fixtures::composer_after_turn(drawn)
                };
                cx.new(|_| ComposerSheet { rows: vec![("", composer)] }).into()
            },
        },
        Scene {
            name: "chat-model-missing",
            description: "A chat whose model is no longer configured: the transcript stays readable, the composer names why, the model pill invites another pick, and Send waits.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        model: "Choose model".into(),
                        brand: None,
                        effort: None,
                        model_missing: Some(wack_core::MODEL_MISSING_MESSAGE.into()),
                        meter: Some(crate::composer::Meter {
                            tokens: 68_000,
                            window: None,
                            high: false,
                            can_compact: false,
                            parts: fixtures::context_parts(68_000),
                            cache: Some(0.924),
                            spent: 3_840_000,
                        }),
                        ..fixtures::composer_after_turn(Input::Drawn {
                            text: Some("Run the retry tests now.".into()),
                            placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                        })
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
            name: "chat-model-missing-backdrop",
            description: "A chat whose model is no longer configured, over a backdrop.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                cx.new(|_| {
                    let composer = ComposerData {
                        model: "Choose model".into(),
                        brand: None,
                        effort: None,
                        model_missing: Some(wack_core::MODEL_MISSING_MESSAGE.into()),
                        meter: Some(crate::composer::Meter {
                            tokens: 68_000,
                            window: None,
                            high: false,
                            can_compact: false,
                            parts: fixtures::context_parts(68_000),
                            cache: Some(0.924),
                            spent: 3_840_000,
                        }),
                        ..fixtures::composer_after_turn(Input::Drawn {
                            text: Some("Run the retry tests now.".into()),
                            placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into(),
                        })
                    };
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer,
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                    .with_backdrop(picture)
                })
                .into()
            },
        },
        Scene {
            name: "composer-model-missing",
            description: "Missing-model composer states: Code composer holding Send, Chat-area composer, and the context popover refusing Compact.",
            size: size(px(760.), px(900.)),
            build: |_, cx| {
                let missing = |chat_area: bool, text: Option<&str>| ComposerData {
                    chat_area,
                    model: "Choose model".into(),
                    brand: None,
                    effort: None,
                    model_missing: Some(wack_core::MODEL_MISSING_MESSAGE.into()),
                    meter: Some(crate::composer::Meter {
                        tokens: 68_000,
                        window: None,
                        high: false,
                        can_compact: false,
                        parts: fixtures::context_parts(68_000),
                        cache: Some(0.924),
                        spent: 3_840_000,
                    }),
                    ..fixtures::composer_after_turn(Input::Drawn {
                        text: text.map(Into::into),
                        placeholder: if chat_area {
                            crate::app::CHAT_FOLLOW_UP_PLACEHOLDER.into()
                        } else {
                            crate::chat::FOLLOW_UP_PLACEHOLDER.into()
                        },
                    })
                };
                let rows = vec![
                    ("Context popover without Compact", ComposerData { context_open: true, ..missing(false, None) }),
                    ("Code chat holding Send", missing(false, Some("Run the retry tests now."))),
                    ("Chat area", missing(true, None)),
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
                        chat_area: false,
                        archived: None,
                        effort: Some("High".into()),
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::High),
                            team: None,
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
                        chat_area: false,
                        archived: None,
                        effort: Some("Default effort".into()),
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::Off),
                            team: None,
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
                        chat_area: false,
                        archived: None,
                        mode,
                        effort: Some("Ultra".into()),
                        ultra_effort: true,
                        menu: Some(crate::composer::MenuData {
                            kind: crate::app::Menu::Effort,
                            groups: crate::shell::effort_groups(&fixtures::reasoning_model().choices(), wack_core::Reasoning::Ultra),
                            team: None,
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
                                chat_area: false,
                                archived: None,
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
            name: "asked-record",
            description: "A finished interview opened in the log: the question card, settled, a pick lit and an answer in the user's own words.",
            size: WINDOW,
            build: |_, cx| chat_scene(fixtures::asked_record(), cx),
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
            name: "computer-card",
            description: "The computer-use card: the agent asks to use TextEdit, with its icon.",
            size: WINDOW,
            build: |_, cx| computer_card_scene(false, false, None, cx),
        },
        Scene {
            name: "computer-card-look-only",
            description: "The card for a browser that isn't running yet: look-only, and started in the background, over a backdrop.",
            size: WINDOW,
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                computer_card_scene(true, true, Some(picture), cx)
            },
        },
        Scene {
            name: "computer-card-clipboard",
            description: "The clipboard card: the agent asks to read what the user copied; Deny or Allow only.",
            size: WINDOW,
            build: |_, cx| computer_card_scene_with(fixtures::clipboard_card(), None, cx),
        },
        Scene {
            name: "computer-card-screen",
            description: "The screen card: the agent asks to take the pointer and keyboard until it's done, in its own words.",
            size: WINDOW,
            build: |_, cx| computer_card_scene_with(fixtures::screen_card(), None, cx),
        },
        Scene {
            name: "computer-card-small",
            description: "The computer-use card in the smallest window the app allows, with a save that failed.",
            size: size(px(716.), px(492.)),
            build: |_, cx| computer_card_scene(false, true, None, cx),
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
            description: "A Build run keeping a todo list, risen from its segment of the tab: one task folded, one under way, dependencies shown.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    let composer = ComposerData {
                        chat_area: false,
                        archived: None,
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
                        .map(|n| crate::composer::Waiting { text: format!("Queued message number {n}, waiting for the run.").into(), files: Vec::new(), steering: n == 1 })
                        .collect();
                    let composer = ComposerData {
                        chat_area: false,
                        archived: None,
                        working: true,
                        todos: Some(fixtures::todos()),
                        waiting,
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
            name: "todo-question-small",
            description: "A todo list, a question and a queued message in the smallest window.",
            size: size(px(716.), px(492.)),
            build: |_, cx| {
                cx.new(|_| {
                    let mode = wack_core::AgentMode::Plan;
                    let todos = fixtures::todos();
                    let composer = ComposerData {
                        chat_area: false,
                        archived: None,
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
            name: "settings-subagents",
            description: "Settings › Sub-agents: delegation, the helper model list open, the agents (one customised, one off, one from Claude Code), and Automatic titles.",
            size: size(px(1180.), px(1400.)),
            build: |_, cx| subagents_scene(SubagentsScene::List, None, cx),
        },
        Scene {
            name: "settings-subagents-backdrop",
            description: "Settings › Sub-agents over a backdrop.",
            size: size(px(1180.), px(1400.)),
            build: |_, cx| subagents_scene(SubagentsScene::List, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-subagent-editor",
            description: "Settings › Sub-agents: customising the built-in Review, with Restore built-in.",
            size: size(px(1180.), px(1300.)),
            build: |_, cx| subagents_scene(SubagentsScene::Editor, None, cx),
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
            name: "chat-menu",
            description: "A chat's right-click menu from its sidebar row: Rename, its folder and file, its path, Close tab and Archive.",
            size: WINDOW,
            build: |_, cx| {
                let composer = fixtures::composer_after_turn(crate::composer::Input::Drawn { text: None, placeholder: "Reply, or ask for the next change".into() });
                let facts = crate::context_menu::ChatFacts { chat: fixtures::FINISHED_TITLE.into(), chat_area: false, has_file: true, tab_open: true, sidebar: true };
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::grouped_turn(&[])), composer },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                    .with_chat_menu(gpui::point(px(150.), px(184.)), facts)
                })
                .into()
            },
        },
        Scene {
            name: "prompt-actions",
            description: "The pointer over a message: Copy, Edit, Rewind and, on the latest, Retry.",
            size: WINDOW,
            build: |_, cx| {
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, hovered: Some(1), ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "prompt-menu",
            description: "A message's right-click menu: Copy, Edit, Rewind to here and Retry on the latest.",
            size: WINDOW,
            build: |_, cx| {
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, ..Default::default() };
                let menu = (gpui::point(px(760.), px(330.)), crate::context_menu::PromptFacts { number: 1, idle: true, latest: true });
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), Some(menu), cx)
            },
        },
        Scene {
            name: "prompt-editing",
            description: "Editing the latest message in place: ↩ sends it again, Esc leaves it.",
            size: WINDOW,
            build: |_, cx| {
                let field = cx.new(|cx| {
                    let mut field = crate::text_field::TextField::new(crate::typography::Type::Body, "Write the message again", cx).multiline(crate::app::COMPOSER_ROWS);
                    field.set_text("Now add a test for a stream that ends mid-line, and one for a stream that is empty.", cx);
                    field
                });
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, editing: Some((1, field)), ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "rewind-card",
            description: "Rewinding to before the latest message: three files changed since, two ticked to go back.",
            size: WINDOW,
            build: |_, cx| {
                let card = fixtures::rewind_card(crate::rewind::Purpose::Rewind, true);
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, card: Some(card), ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "rewind-card-plain",
            description: "Rewinding when no file changed since the message: one button.",
            size: WINDOW,
            build: |_, cx| {
                let card = fixtures::rewind_card(crate::rewind::Purpose::Rewind, false);
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, card: Some(card), ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "rewind-card-edit",
            description: "Sending an edited message whose files changed since: keep them, or put the ticked ones back and send.",
            size: WINDOW,
            build: |_, cx| {
                let card = fixtures::rewind_card(crate::rewind::Purpose::Edit, true);
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 2, card: Some(card), ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::two_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "rewound",
            description: "Just after a rewind: its row at the end of the transcript offers Undo until anything runs.",
            size: WINDOW,
            build: |_, cx| {
                let prompts = crate::rewind::Prompts { chat: true, idle: true, count: 1, ..Default::default() };
                rewind_scene(Body::Prompted(fixtures::rewound_turns(), Box::new(prompts)), None, cx)
            },
        },
        Scene {
            name: "chat-rename-tab",
            description: "Renaming a chat in its header, at the title's size (double-click the title or its tab, or File › Rename Chat…).",
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
            name: "settings-providers-revealed",
            description: "Settings › Providers with the saved key shown by the eye: plain in its field, and nothing to save.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| providers_scene(Providers::Revealed, false, cx),
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
            name: "settings-providers-catalog-refreshing",
            description: "Settings › Providers' model catalogue card while Refresh asks models.dev.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| providers_scene(Providers::Catalog(CatalogScene::Refreshing), false, cx),
        },
        Scene {
            name: "settings-providers-catalog-failed",
            description: "The catalogue card after a Refresh failed: why, and which copy is still in use.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| providers_scene(Providers::Catalog(CatalogScene::Failed), false, cx),
        },
        Scene {
            name: "settings-providers-catalog-refreshed",
            description: "The catalogue card after a Refresh: the new copy's date and what was new.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| providers_scene(Providers::Catalog(CatalogScene::Refreshed), false, cx),
        },
        Scene {
            name: "settings-providers-suggest",
            description: "The catalogue's suggestions under a model id field, from OpenRouter's own listings (\"opus\" typed).",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| providers_scene(Providers::Suggest { local: false }, false, cx),
        },
        Scene {
            name: "settings-providers-suggest-local",
            description: "Suggestions for a server on this Mac: the makers' records, under their bare ids.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| providers_scene(Providers::Suggest { local: true }, false, cx),
        },
        Scene {
            name: "settings-providers-suggest-backdrop",
            description: "The catalogue's suggestions over a backdrop.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| providers_scene_over(Providers::Suggest { local: false }, false, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-providers-suggest-small",
            description: "The suggestions in the smallest window: the card keeps to the id field's width.",
            size: size(px(716.), px(1000.)),
            build: |_, cx| providers_scene(Providers::Suggest { local: false }, true, cx),
        },
        Scene {
            name: "settings-providers-filled",
            description: "Two rows the catalogue filled: context, max output, Vision and efforts, one with a note to check its efforts.",
            size: size(px(1180.), px(1160.)),
            build: |_, cx| providers_scene(Providers::Filled, false, cx),
        },
        Scene {
            name: "settings-providers-fetch",
            description: "Fetch models: OpenRouter's list with what the catalogue knows, two ticked, one already added.",
            size: size(px(1180.), px(1280.)),
            build: |_, cx| providers_scene(Providers::Fetch(FetchScene::Listed), false, cx),
        },
        Scene {
            name: "settings-providers-fetch-loading",
            description: "Fetch models while the provider answers.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| providers_scene(Providers::Fetch(FetchScene::Loading), false, cx),
        },
        Scene {
            name: "settings-providers-fetch-failed",
            description: "Fetch models when the provider refused the key.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| providers_scene(Providers::Fetch(FetchScene::Failed), false, cx),
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
                    prompts: Default::default(),
                    tools: crate::tools_page::ToolsData { unrestricted_planning: false, confirming: true, web_fetch: true },
                    memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
                };
                cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)))).into()
            },
        },
        Scene {
            name: "settings-skills",
            description: "Settings › Skills: your skills, Claude Code's folder on with a skill that lost its name, an added folder and a file that could not load.",
            size: size(px(1180.), px(1200.)),
            build: |_, cx| skills_scene(SkillsScene::List, false, cx),
        },
        Scene {
            name: "settings-skills-backdrop",
            description: "Settings › Skills over a backdrop.",
            size: size(px(1180.), px(1200.)),
            build: |_, cx| skills_scene(SkillsScene::List, true, cx),
        },
        Scene {
            name: "settings-skills-empty",
            description: "Settings › Skills before the user has a skill of their own.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| skills_scene(SkillsScene::Empty, false, cx),
        },
        Scene {
            name: "settings-skills-editor",
            description: "Settings › Skills: editing one of your skills, asking before it moves to the Trash.",
            size: size(px(1180.), px(1150.)),
            build: |_, cx| skills_scene(SkillsScene::Editor, false, cx),
        },
        Scene {
            name: "settings-skills-view",
            description: "Settings › Skills: a skill from Claude Code's folder, open read-only.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| skills_scene(SkillsScene::View, false, cx),
        },
        Scene {
            name: "settings-prompts",
            description: "Settings › Prompts: three groups, Ultra Plan and Automatic titles customised, the restore-all row at the top.",
            size: size(px(1180.), px(1200.)),
            build: |_, cx| prompts_scene(PromptsScene::Overview, false, cx),
        },
        Scene {
            name: "settings-prompts-backdrop",
            description: "Settings › Prompts over a backdrop.",
            size: size(px(1180.), px(1200.)),
            build: |_, cx| prompts_scene(PromptsScene::Overview, true, cx),
        },
        Scene {
            name: "settings-prompts-editor",
            description: "Settings › Prompts: Ultra Plan's editor customised, the built-in text open beneath it for reference.",
            size: size(px(1180.), px(1400.)),
            build: |_, cx| prompts_scene(PromptsScene::Editor, false, cx),
        },
        Scene {
            name: "settings-prompts-restore",
            description: "Settings › Prompts: restoring a customised prompt asks before the replacement is dropped.",
            size: size(px(1180.), px(1200.)),
            build: |_, cx| prompts_scene(PromptsScene::Restore, false, cx),
        },
        Scene {
            name: "settings-commands",
            description: "Settings › Commands: two commands of your own, one switched off, and WackCode's.",
            size: size(px(1180.), px(1050.)),
            build: |_, cx| commands_scene(false, cx),
        },
        Scene {
            name: "settings-commands-editor",
            description: "Settings › Commands: a command's editor, its preview expanding sample arguments.",
            size: size(px(1180.), px(1100.)),
            build: |_, cx| commands_scene(true, cx),
        },
        Scene {
            name: "settings-git",
            description: "Settings › Git: commit co-author attribution enabled, with the trailer preview and notes.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| git_scene(true, None, cx),
        },
        Scene {
            name: "settings-git-off",
            description: "Settings › Git with commit co-author attribution switched off: the trailer line struck through.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| git_scene(false, None, cx),
        },
        Scene {
            name: "settings-git-backdrop",
            description: "Settings › Git over a backdrop.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| git_scene(true, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-computer",
            description: "Settings › Computer use, off, with neither macOS permission and nothing saved.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| computer_settings_scene(false, None, cx),
        },
        Scene {
            name: "settings-computer-on",
            description: "Settings › Computer use, on, both permissions allowed, an app always allowed and one never allowed.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| computer_settings_scene(true, None, cx),
        },
        Scene {
            name: "settings-computer-backdrop",
            description: "Settings › Computer use, on, over a backdrop.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| computer_settings_scene(true, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-browser",
            description: "Settings › Browser: the agent's tabs open the pane, new tabs at Fit, clearing and the engine.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| browser_settings_scene(crate::browser_page::Clearing::Idle, None, cx),
        },
        Scene {
            name: "settings-browser-confirm",
            description: "Settings › Browser asking to confirm clearing the browsing data.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| browser_settings_scene(crate::browser_page::Clearing::Confirming, None, cx),
        },
        Scene {
            name: "settings-browser-pending",
            description: "Settings › Browser after clearing while the browser ran: cookies gone, the rest at the next start.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| browser_settings_scene(crate::browser_page::Clearing::Pending, None, cx),
        },
        Scene {
            name: "settings-browser-backdrop",
            description: "Settings › Browser over a backdrop.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| browser_settings_scene(crate::browser_page::Clearing::Idle, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-integrations",
            description: "Settings › Integrations: TokenTrail recording, a recent record, and the ledger's folder.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| integrations_scene(UsageScene::Recording, None, cx),
        },
        Scene {
            name: "settings-integrations-backdrop",
            description: "Settings › Integrations over a backdrop.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| integrations_scene(
                UsageScene::Recording,
                Some(over(fixtures::backdrop(wack_core::Texture::None), cx)),
                cx,
            ),
        },
        Scene {
            name: "settings-integrations-paused",
            description: "Settings › Integrations with recording paused.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| integrations_scene(UsageScene::Paused, None, cx),
        },
        Scene {
            name: "settings-integrations-attention",
            description: "Settings › Integrations when a write failed: records waiting, one lost, and the failure itself.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| integrations_scene(UsageScene::Attention, None, cx),
        },
        Scene {
            name: "settings-web",
            description: "Settings › Web: reading through DeepSeek, three providers (one needing a key) and this month's usage.",
            size: size(px(1180.), px(1100.)),
            build: |_, cx| web_scene(WebScene::Providers, None, cx),
        },
        Scene {
            name: "settings-web-empty",
            description: "Settings › Web before any search provider is added.",
            size: size(px(1180.), px(900.)),
            build: |_, cx| web_scene(WebScene::Empty, None, cx),
        },
        Scene {
            name: "settings-web-backdrop",
            description: "Settings › Web over a backdrop.",
            size: size(px(1180.), px(1100.)),
            build: |_, cx| web_scene(WebScene::Providers, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-web-editor",
            description: "Settings › Web's editor for DeepSeek, borrowing a connection's key, after a Test search.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| web_scene(WebScene::Editor, None, cx),
        },
        Scene {
            name: "settings-web-custom",
            description: "Settings › Web's editor for a Custom POST API whose Test search failed.",
            size: size(px(1180.), px(1400.)),
            build: |_, cx| web_scene(WebScene::Custom, None, cx),
        },
        Scene {
            name: "settings-mcp",
            description: "Settings › MCP servers: one connected and open with its tools (one off, two read-only), one scoped to a project and connecting, one that failed, one off.",
            size: size(px(1180.), px(1250.)),
            build: |_, cx| mcp_scene(McpScene::Servers, None, cx),
        },
        Scene {
            name: "settings-mcp-empty",
            description: "Settings › MCP servers before any server is added: the two ways to start.",
            size: size(px(1180.), px(760.)),
            build: |_, cx| mcp_scene(McpScene::Empty, None, cx),
        },
        Scene {
            name: "settings-mcp-backdrop",
            description: "Settings › MCP servers over a backdrop.",
            size: size(px(1180.), px(1250.)),
            build: |_, cx| mcp_scene(McpScene::Servers, Some(over(fixtures::backdrop(wack_core::Texture::None), cx)), cx),
        },
        Scene {
            name: "settings-mcp-delete",
            description: "Settings › MCP servers asking before it deletes a server.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| mcp_scene(McpScene::Deleting, None, cx),
        },
        Scene {
            name: "settings-mcp-editor",
            description: "Settings › MCP servers' editor for a command: arguments, a saved variable and a new one, two chosen projects, after a Test connection.",
            size: size(px(1180.), px(1400.)),
            build: |_, cx| mcp_scene(McpScene::EditorCommand, None, cx),
        },
        Scene {
            name: "settings-mcp-editor-http",
            description: "Settings › MCP servers' editor for a new server at a URL, with a header, whose Test connection was refused.",
            size: size(px(1180.), px(1150.)),
            build: |_, cx| mcp_scene(McpScene::EditorUrl, None, cx),
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
            description: "Settings › Appearance over a backdrop: a library of two pictures, each captioned with its theme, with Shuffle on.",
            size: size(px(1440.), px(1200.)),
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
            description: "A chat's header with configured Build and active Run split buttons.",
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
            description: "New chat screen with centered Build/Run pills and the Run output open in the right dock.",
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
            description: "Failed Build and active Run panes stacked in the right dock under Changes.",
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
                    layout.move_pane(run_id, Target::Beside(changes_id, Edge::Bottom));
                    layout.move_pane(build_id, Target::With(run_id));
                    layout.right_width = Some(560.);
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
            name: "chat-tasks",
            description: "Commands in the background: started, a failed suite waking the chat, their list risen from the tab with each one's newest output.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::task_turns()),
                            composer: fixtures::composer_tasks(true),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-tasks-foreground",
            description: "The agent waiting on a slow command: the tab's time, the running commands and ⌘B Background.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::working_turn()),
                            composer: fixtures::composer_foreground(),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "tasks-pane",
            description: "The Tasks pane in the right dock: a running server, a failed suite, a watch.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                cx.new(|cx| {
                    let mut layout = Layout::default();
                    let tasks_id = layout.open(PaneKind::Tasks);
                    layout.right_width = Some(560.);
                    let mut panes = ChatPanes::new(layout, "/Users/you/Code/wackcode".into());
                    let tasks = fixtures::tasks();
                    panes.insert(
                        tasks_id,
                        PaneModel::Tasks(cx.new(|cx| {
                            let mut pane = TasksPane::new(tasks.clone(), cx);
                            pane.selected = Some("k3v9x2m1".into());
                            pane
                        })),
                        cx,
                    );
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::task_turns()),
                            composer: fixtures::composer_tasks(false),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                    .with_panes(panes, false)
                })
                .into()
            },
        },
        Scene {
            name: "chat-agents",
            description: "Four agents sent out together: one reported, two working (one in the background), one queued; the agents pill.",
            size: WINDOW,
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::agent_turn()),
                            composer: fixtures::composer_agents(),
                        },
                        fixtures::strip(Some(fixtures::WORKING_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat-team",
            description: "A Team at work: Explore and the shared type back, crew Generals on their own files (one in the background), one queued; the lead waiting, the crew pill.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| team_chat_scene(false, cx),
        },
        Scene {
            name: "chat-team-idle-lead",
            description: "A Team whose lead has stopped with its crew still out in the background: the crew's segment of the tab stays up for them.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| team_chat_scene(true, cx),
        },
        Scene {
            name: "chat-agents-ended",
            description: "Agents that failed, were stopped and were interrupted by a quit, and a background report waking the chat.",
            size: size(px(1180.), px(1000.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing_tabs(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::agent_ends()),
                            composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
                        },
                        fixtures::strip(Some(fixtures::FINISHED_TITLE)),
                    )
                })
                .into()
            },
        },
        Scene {
            name: "agents-pane",
            description: "The Agents pane in the right dock, on an Explore agent at work.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| agents_scene(None, cx),
        },
        Scene {
            name: "canvas-board",
            description: "The Canvas pane at most of the stage: two rows of artboards, fitted, beside a 420 pt transcript.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Board, None, cx),
        },
        Scene {
            name: "canvas-board-backdrop",
            description: "The Canvas pane over an image backdrop: the board's own surface keeps the pictures readable.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                canvas_scene(CanvasScene::Board, Some(picture), cx)
            },
        },
        Scene {
            name: "canvas-errors",
            description: "An artboard with console errors selected: its badge and Ask to fix.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Errors, None, cx),
        },
        Scene {
            name: "canvas-far-zoom",
            description: "The board zoomed far out: only the names show over the small pictures.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Far, None, cx),
        },
        Scene {
            name: "canvas-play",
            description: "Playing an artboard: the board dims, the artboard fits above its pill while its live page opens.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Play, None, cx),
        },
        Scene {
            name: "canvas-tweaks",
            description: "The Tweaks panel on the selected artboard: a colour, a choice, a number and a toggle, two of them moved.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Tweaks, None, cx),
        },
        Scene {
            name: "canvas-tweaks-play",
            description: "Tweaks while playing: the live page fits beside the panel, so the panel is never under it.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::TweaksPlay, None, cx),
        },
        Scene {
            name: "canvas-tweaks-narrow",
            description: "Tweaks while playing in a 1,000 pt window: too narrow to play beside the panel, so the page keeps the whole width.",
            size: size(px(1000.), px(760.)),
            build: |_, cx| canvas_scene(CanvasScene::TweaksPlay, None, cx),
        },
        Scene {
            name: "canvas-comments",
            description: "The Comment tool: pins on Calm (two drafts) and Dense (sent), the element under the pointer outlined, and a new comment being written.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Comments, None, cx),
        },
        Scene {
            name: "canvas-comments-backdrop",
            description: "The Comment tool over an image backdrop.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                canvas_scene(CanvasScene::Comments, Some(picture), cx)
            },
        },
        Scene {
            name: "canvas-drawing",
            description: "The model writing an artboard: its row and the board's pill say which and how much has come.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Drawing, None, cx),
        },
        Scene {
            name: "canvas-comment-list",
            description: "The switcher listing the canvas's open comments, drafts and sent.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::CommentList, None, cx),
        },
        Scene {
            name: "canvas-present",
            description: "Present: one artboard over the whole window, stepping with the arrows.",
            size: size(px(1680.), px(1000.)),
            build: |_, cx| canvas_scene(CanvasScene::Present, None, cx),
        },
        Scene {
            name: "canvas-empty",
            description: "The Canvas pane before the agent has drawn anything.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| canvas_scene(CanvasScene::Empty, None, cx),
        },
        Scene {
            name: "canvas-wide",
            description: "The Canvas pane in a 2,500 pt window.",
            size: size(px(2500.), px(1300.)),
            build: |_, cx| canvas_scene(CanvasScene::Board, None, cx),
        },
        Scene {
            name: "canvas-narrow",
            description: "The Canvas pane in a 1,000 pt window: the transcript keeps its minimum width.",
            size: size(px(1000.), px(760.)),
            build: |_, cx| canvas_scene(CanvasScene::Board, None, cx),
        },
        Scene {
            name: "canvas-card",
            description: "A turn that drew on a canvas: its rows, the card with the artboards' pictures, one to fix.",
            size: WINDOW,
            build: |_, cx| canvas_chat_scene(false, cx),
        },
        Scene {
            name: "canvas-card-live",
            description: "The same turn still at work: the card says it is drawing.",
            size: WINDOW,
            build: |_, cx| canvas_chat_scene(true, cx),
        },
        Scene {
            name: "browser-pane",
            description: "The Browser pane on a dev server the user opened, beside a page the agent opened.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Page, None, cx),
        },
        Scene {
            name: "browser-pane-agent",
            description: "The agent driving a tab: its ring round the page, Take control, the loading sweep.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Agent, None, cx),
        },
        Scene {
            name: "browser-pane-agent-backdrop",
            description: "The agent driving a tab, over a backdrop.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                browser_scene(BrowserScene::Agent, Some(picture), cx)
            },
        },
        Scene {
            name: "browser-pane-dialog",
            description: "A page's confirm() held as a card over the page, for the user or the agent to answer.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Dialog, None, cx),
        },
        Scene {
            name: "browser-pane-failed",
            description: "A dev server that isn't running: the failed load as a card, with Try again.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Failed, None, cx),
        },
        Scene {
            name: "browser-pane-crashed",
            description: "A page whose process ended: a card in its place, with Reload.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Crashed, None, cx),
        },
        Scene {
            name: "browser-pane-empty",
            description: "The Browser pane with no pages open.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Empty, None, cx),
        },
        Scene {
            name: "browser-pane-phone",
            description: "A page laid out at a phone's size, scaled down and letterboxed in the pane.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Phone, None, cx),
        },
        Scene {
            name: "browser-pane-sizes",
            description: "The page size menu open over the page.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| browser_scene(BrowserScene::Sizes, None, cx),
        },
        Scene {
            name: "agents-pane-backdrop",
            description: "The Agents pane over a backdrop.",
            size: size(px(1480.), px(940.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                agents_scene(Some(picture), cx)
            },
        },
        Scene {
            name: "agents-pane-team",
            description: "The Agents pane's Team overview: lead and crew, what each spent, the plan and the request's workers.",
            size: size(px(1480.), px(1000.)),
            build: |_, cx| team_pane_scene(None, cx),
        },
        Scene {
            name: "agents-pane-team-backdrop",
            description: "The Team overview over a backdrop.",
            size: size(px(1480.), px(1000.)),
            build: |_, cx| {
                let picture = over(fixtures::backdrop(wack_core::Texture::None), cx);
                team_pane_scene(Some(picture), cx)
            },
        },
        Scene {
            name: "build-run-narrow",
            description: "Smallest window (716pt): compact icon-only Build and Run buttons in the chat's header.",
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
            chat_area: false,
            archived: None,
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
            chat_area: false,
            archived: None,
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

/// A chat whose agent waits on the computer-use card. `failed` shows a save that went wrong.
fn computer_card_scene(look_only: bool, failed: bool, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    computer_card_scene_with(fixtures::computer_card(look_only, failed), backdrop, cx)
}

fn computer_card_scene_with(access: crate::computer_card::CardData, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    cx.new(|_| {
        let composer = ComposerData {
            chat_area: false,
            archived: None,
            working: true,
            access: Some(access),
            ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::app::placeholder(wack_core::AgentMode::Build, false).into() })
        };
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::working_turn()), composer },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        );
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

fn computer_settings_scene(on: bool, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let computer = if on {
        crate::computer_page::ComputerData {
            enabled: true,
            accessibility: true,
            screen_recording: true,
            always: vec![("TextEdit".into(), "/System/Applications/TextEdit.app".into())],
            never: vec!["com.apple.mail".into()],
            error: None,
        }
    } else {
        crate::computer_page::ComputerData::default()
    };
    let data = SettingsData {
        section: Section::Computer,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer,
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

/// `picture`, with the theme made ready for text over it, as the app's theme is.
fn over(picture: crate::backdrop::Picture, cx: &mut App) -> crate::backdrop::Picture {
    let input = crate::theme::ThemeInput { ground: Some(picture.ground()), ..crate::theme::Theme::get(cx).input.clone() };
    cx.set_global(crate::theme::Theme::derive(input));
    picture
}

#[derive(Clone, Copy, PartialEq)]
enum Providers {
    Listed,
    Adding,
    Editing,
    /// A saved key shown plainly by the eye.
    Revealed,
    Empty,
    /// The model catalogue's card on the overview.
    Catalog(CatalogScene),
    /// The catalogue's suggestions under a new row's id field: OpenRouter's own, or the makers'
    /// records for a server on this Mac.
    Suggest { local: bool },
    /// Rows the catalogue filled: one exactly, one whose efforts nobody lists.
    Filled,
    Fetch(FetchScene),
}

/// The catalogue card's states after Refresh; `settings-providers` shows it as built in.
#[derive(Clone, Copy, PartialEq)]
enum CatalogScene {
    Refreshing,
    Failed,
    Refreshed,
}

#[derive(Clone, Copy, PartialEq)]
enum FetchScene {
    Listed,
    Loading,
    Failed,
}

/// Settings › Providers in one of its states.
fn providers_scene(state: Providers, sidebar_hidden: bool, cx: &mut App) -> AnyView {
    providers_scene_over(state, sidebar_hidden, None, cx)
}

/// A new OpenRouter connection's editor, for the catalogue's scenes.
fn openrouter_form(form: &mut ProviderForm, cx: &mut App) {
    form.open_new(cx);
    form.name.update(cx, |field, cx| field.set_text("OpenRouter", cx));
    form.address.update(cx, |field, cx| field.set_text("https://openrouter.ai/api/v1", cx));
    form.key.update(cx, |field, cx| field.set_text("not-a-real-key", cx));
}

/// [`providers_scene`], drawn over `backdrop` when there is one.
fn providers_scene_over(state: Providers, sidebar_hidden: bool, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use crate::{
        model_catalog::CatalogLine,
        settings::{Fetch, FetchList, Fetched, ModelSuggest, listing_facts},
    };
    let mut form = ProviderForm::new(cx);
    let mut connections = fixtures::connections();
    connections[1].enabled = false;
    let catalog = fixtures::catalog();
    let mut line = CatalogLine { date: Some("26 Nov 2026".into()), built_in: true, models: 1_266, refreshing: false, outcome: None };
    match state {
        Providers::Listed => {}
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
        Providers::Revealed => {
            form.edit(&connections[0].clone(), cx);
            form.show_saved_key("sk-ant-api03-fixture-key-only-not-real-7f3a9c2e5b", cx);
        }
        Providers::Empty => connections.clear(),
        Providers::Catalog(kind) => match kind {
            CatalogScene::Refreshing => line.refreshing = true,
            CatalogScene::Failed => {
                line.outcome = Some(Err("Couldn't reach models.dev. Check the connection to the internet. The catalogue from 26 Nov 2026 is still in use.".into()))
            }
            CatalogScene::Refreshed => {
                line.built_in = false;
                line.models = 1_284;
                line.outcome = Some(Ok("Refreshed: 1,284 models, 18 new.".into()));
            }
        },
        Providers::Suggest { local } => {
            openrouter_form(&mut form, cx);
            let (address, typed) = if local { ("http://localhost:1234/v1", "qwen3 coder") } else { ("https://openrouter.ai/api/v1", "opus") };
            if local {
                form.name.update(cx, |field, cx| field.set_text("LM Studio", cx));
                form.address.update(cx, |field, cx| field.set_text(address, cx));
            }
            form.models[0].id.update(cx, |field, cx| field.set_text(typed, cx));
            let matches = catalog.search(typed, address, wack_core::catalog::SUGGESTIONS);
            form.suggest = Some(ModelSuggest { field: form.models[0].id.entity_id(), query: typed.into(), matches, selected: 0 });
        }
        Providers::Filled => {
            openrouter_form(&mut form, cx);
            form.add_model("", None, cx);
            for (index, id) in ["anthropic/claude-opus-5.5", "z-ai/glm-5.3"].into_iter().enumerate() {
                if let Some(found) = catalog.lookup(id, "https://openrouter.ai/api/v1") {
                    form.apply(index, Some(id), &found, cx);
                }
            }
        }
        Providers::Fetch(kind) => {
            openrouter_form(&mut form, cx);
            let address = "https://openrouter.ai/api/v1";
            if let Some(found) = catalog.lookup("anthropic/claude-opus-5.5", address) {
                form.apply(0, Some("anthropic/claude-opus-5.5"), &found, cx);
                form.models[0].open = false;
            }
            form.fetch = match kind {
                FetchScene::Loading => Fetch::Loading { host: "openrouter.ai".into() },
                FetchScene::Failed => Fetch::Failed("openrouter.ai did not accept the API key (401): No auth credentials found".into()),
                FetchScene::Listed => {
                    let ids = [
                        "anthropic/claude-opus-5.5",
                        "anthropic/claude-opus-4.8",
                        "deepseek/deepseek-v4.1-flash",
                        "openai/gpt-5.6-sol",
                        "z-ai/glm-5.3",
                        "mistralai/devstral-3",
                        "moonshotai/kimi-k3.1",
                    ];
                    let models = ids
                        .into_iter()
                        .enumerate()
                        .map(|(index, id)| Fetched {
                            id: id.into(),
                            ticked: matches!(index, 2 | 3),
                            present: index == 0,
                            facts: catalog.lookup(id, address).map(|found| listing_facts(&found.listing)),
                        })
                        .collect();
                    let filter = cx.new(|cx| crate::text_field::TextField::new(crate::typography::Type::Ui, "Filter", cx));
                    Fetch::Listed(FetchList { host: "openrouter.ai".into(), models, filter })
                }
            };
        }
    }
    let data = SettingsData {
        section: Section::Providers,
        providers: ProvidersData::new(&connections, |id| id != "c2", &form, cx).with_catalog(line),
        appearance: fixtures::appearance(None, cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        let shell = if sidebar_hidden { shell.without_sidebar() } else { shell };
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
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
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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
fn terminal_selection_scene(
    menu: Option<gpui::Point<Pixels>>,
    notice: Option<&'static str>,
    picture: Option<crate::backdrop::Picture>,
    cx: &mut App,
) -> AnyView {
    let picture = picture.map(|picture| over(picture, cx));
    cx.new(|cx| {
        let mut layout = Layout::default();
        let terminal = layout.open(PaneKind::Terminal);
        layout.right_width = Some(560.);
        let mut panes = ChatPanes::new(layout, "/".into());
        let pane = cx.new(|cx| {
            let mut pane = TerminalPane::fixture(fixtures::TERMINAL_SESSION.as_bytes(), 140, 22, None, cx).with_selection((4, 0), (8, 40));
            if let Some(position) = menu {
                pane = pane.with_menu(position);
            }
            if let Some(notice) = notice {
                pane = pane.with_notice(notice);
            }
            pane
        });
        panes.insert(terminal, PaneModel::Terminal(pane), cx);
        let chip = crate::attachments::FileAttachment::from_terminal(
            "running 4 tests\ntest sse::tests::splits_events_across_chunks ... ok\ntest sse::tests::ignores_comments ... ok\ntest sse::tests::keeps_a_tail_without_a_blank_line ... ok",
            &[],
        )
        .unwrap();
        let composer = ComposerData {
            files: vec![chip],
            ..fixtures::composer_after_turn(Input::Drawn { text: Some("All four SSE tests pass now.".into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
        };
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::finished_turn()),
                composer,
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        )
        .with_panes(panes, false);
        match picture {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

fn panes_scene(layout: Layout, ids: &[u32], sidebar_hidden: bool, cx: &mut App) -> AnyView {
    panes_scene_over(layout, ids, sidebar_hidden, None, cx)
}

fn panes_scene_over(layout: Layout, ids: &[u32], sidebar_hidden: bool, picture: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let picture = picture.map(|picture| over(picture, cx));
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
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::finished_turn()),
                composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        )
        .with_panes(panes, sidebar_hidden);
        match picture {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

#[derive(Clone, Copy, PartialEq)]
enum CanvasScene {
    Board,
    Errors,
    Far,
    Empty,
    Play,
    Present,
    Tweaks,
    TweaksPlay,
    Comments,
    Drawing,
    CommentList,
}

/// A chat with its Canvas pane in the right dock, at its default width. Pictures are fixtures:
/// scenes have no engine.
fn canvas_scene(scene: CanvasScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut layout = Layout::default();
        let id = layout.open(PaneKind::Canvas);
        let mut panes = ChatPanes::new(layout, "/Users/you/Documents/GitHub/wackcode".into());
        let folder = std::env::temp_dir().join(format!("wackcode-gallery-canvas-{}", if scene == CanvasScene::Empty { "empty" } else { "board" }));
        let canvases = if scene == CanvasScene::Empty {
            let _ = std::fs::remove_dir_all(&folder);
            wack_canvas::Canvases::new(&folder, None)
        } else {
            fixtures::canvases(&folder)
        };
        let items = match scene {
            CanvasScene::Empty => fixtures::agent_turn(),
            CanvasScene::Drawing => fixtures::canvas_drawing_turn(&canvases),
            _ => fixtures::canvas_turn(&canvases, false),
        };
        panes.insert(
            id,
            PaneModel::Canvas(cx.new(|cx| {
                let mut pane = crate::panes::canvas::CanvasPane::new(canvases, cx);
                match scene {
                    CanvasScene::Errors => pane.stage(Some("empty-state"), None),
                    CanvasScene::Far => pane.stage(None, Some(0.16)),
                    CanvasScene::Play => pane.stage_play("dense"),
                    CanvasScene::Tweaks => {
                        pane.stage(Some("calm"), None);
                        pane.stage_tweaks();
                    }
                    CanvasScene::Comments => pane.stage_comments("calm", "0/1", Some(("0/3", "Group these two fields under one heading")), cx),
                    CanvasScene::Drawing => pane.set_writing(Some(crate::conversation::Writing::shown(16_884, Some("Bold"))), cx),
                    CanvasScene::CommentList => pane.stage_switcher(),
                    CanvasScene::TweaksPlay => {
                        pane.stage_play("calm");
                        pane.stage_tweaks();
                    }
                    CanvasScene::Present => {
                        pane.stage(Some("dense"), None);
                        pane.present(cx);
                    }
                    _ => {}
                }
                pane
            })),
            cx,
        );
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(items),
                composer: ComposerData {
                    chat_area: false,
                    archived: None,
                    comments: (scene == CanvasScene::Comments).then(|| (2, "settings".to_string())),
                    ..fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
                },
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        )
        .with_panes(panes, false);
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// A chat whose turn drew on a canvas, the pane closed: the card in the transcript.
fn canvas_chat_scene(live: bool, cx: &mut App) -> AnyView {
    let canvases = fixtures::canvases(&std::env::temp_dir().join("wackcode-gallery-canvas-card"));
    let items = fixtures::canvas_turn(&canvases, live);
    cx.new(|_| {
        Shell::showing_tabs(
            fixtures::sidebar(if live { Selected::Working } else { Selected::Finished }),
            Screen::Chat {
                title: fixtures::FINISHED_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(items),
                composer: if live { fixtures::composer_working() } else { fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }) },
            },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        )
    })
    .into()
}

/// What the Browser pane shows in a browser scene.
#[derive(Clone, Copy, PartialEq)]
enum BrowserScene {
    Page,
    Agent,
    Dialog,
    Failed,
    Crashed,
    Empty,
    Phone,
    Sizes,
}

/// A chat with its Browser pane in the right dock. The page is a still: scenes have no engine.
fn browser_scene(scene: BrowserScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use wack_browser::{ChatTabs, Dialog, DialogKind, Owner, SavedTab, Viewport};
    cx.new(|cx| {
        let mut layout = Layout::default();
        let id = layout.open(PaneKind::Browser);
        layout.right_width = Some(620.);
        let mut panes = ChatPanes::new(layout, "/Users/you/Documents/GitHub/wackcode".into());
        let tabs = ChatTabs::new(None);
        let saved = |url: &str, title: &str, owner: Owner| SavedTab { url: url.into(), title: title.into(), owner };
        if scene != BrowserScene::Empty {
            tabs.restore(vec![
                saved("http://localhost:5173/signup", "Sign up · Tidepool", Owner::User),
                saved("https://docs.example.com/guides/forms", "Forms guide", Owner::Agent),
            ]);
        }
        let list = tabs.tabs();
        let shown = match scene {
            BrowserScene::Agent | BrowserScene::Dialog => list.get(1).cloned(),
            _ => list.first().cloned(),
        };
        if let Some(tab) = &shown {
            tabs.set_shown(Some(tab.id));
            match scene {
                BrowserScene::Agent => tab.update(|state| state.loading = true),
                BrowserScene::Dialog => tab.update(|state| {
                    state.dialog = Some(Dialog {
                        kind: DialogKind::Confirm,
                        message: "Leave this guide? Your notes on the page won't be kept.".into(),
                        default_prompt: String::new(),
                    })
                }),
                BrowserScene::Crashed => tab.update(|state| state.crashed = Some("Out of memory".into())),
                BrowserScene::Failed => tab.update(|state| {
                    state.load_error = Some(("http://localhost:5173/signup".into(), "ERR_CONNECTION_REFUSED".into()))
                }),
                BrowserScene::Phone | BrowserScene::Sizes => tab.update(|state| {
                    state.viewport = if scene == BrowserScene::Phone { Viewport::Phone } else { Viewport::Fit };
                    state.can_go_back = true;
                }),
                _ => tab.update(|state| state.can_go_back = true),
            }
        }
        let picture = match scene {
            BrowserScene::Phone => fixtures::page_picture(390, 844),
            _ => fixtures::page_picture(1280, 1600),
        };
        let menu = scene == BrowserScene::Sizes;
        panes.insert(
            id,
            PaneModel::Browser(cx.new(|cx| {
                let pane = BrowserPane::new(tabs, cx).with_fixture(picture);
                if menu { pane.with_viewport_menu() } else { pane }
            })),
            cx,
        );
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat {
                title: fixtures::WORKING_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::agent_turn()),
                composer: fixtures::composer_agents(),
            },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        )
        .with_panes(panes, false);
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// A Team chat at work: the lead waiting on its crew, or idle with the crew in the background.
/// Either way the chat is working, as the sidebar shows it.
fn team_chat_scene(lead_idle: bool, cx: &mut App) -> AnyView {
    let (selected, title) = (Selected::Working, fixtures::WORKING_TITLE);
    cx.new(|_| {
        Shell::showing_tabs(
            fixtures::sidebar(selected),
            Screen::Chat {
                title: title.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::team_turn(lead_idle)),
                composer: fixtures::composer_team(lead_idle),
            },
            fixtures::strip(Some(title)),
        )
    })
    .into()
}

fn agents_scene(backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut layout = Layout::default();
        let id = layout.open(PaneKind::Agents);
        layout.right_width = Some(560.);
        let mut panes = ChatPanes::new(layout, "/Users/you/Documents/GitHub/wackcode".into());
        let agents = fixtures::agents_registry(&std::env::temp_dir().join("wackcode-gallery-agents"));
        panes.insert(
            id,
            PaneModel::Agents(cx.new(|cx| {
                let mut pane = AgentsPane::new(agents, "/Users/you/Documents/GitHub/wackcode".into(), cx);
                pane.selected = Some("a2".into());
                pane
            })),
            cx,
        );
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat {
                title: fixtures::WORKING_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::agent_turn()),
                composer: fixtures::composer_agents(),
            },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        )
        .with_panes(panes, false);
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// A Team chat with its Agents pane on the Team overview.
fn team_pane_scene(backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut layout = Layout::default();
        let id = layout.open(PaneKind::Agents);
        layout.right_width = Some(560.);
        let mut panes = ChatPanes::new(layout, "/Users/you/Documents/GitHub/wackcode".into());
        let source: crate::panes::agents::TeamSource = std::rc::Rc::new(|_: &App| Some(fixtures::team_view()));
        panes.insert(
            id,
            PaneModel::Agents(cx.new(|cx| {
                let mut pane = AgentsPane::new(fixtures::team_registry(), "/Users/you/Documents/GitHub/wackcode".into(), cx);
                pane.set_team_source(Some(source), cx);
                pane
            })),
            cx,
        );
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat {
                title: fixtures::WORKING_TITLE.into(),
                project: fixtures::project(),
                body: Body::Items(fixtures::team_turn(false)),
                composer: fixtures::composer_team(false),
            },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        )
        .with_panes(panes, false);
        match backdrop {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

#[derive(Clone, Copy, PartialEq)]
enum SubagentsScene {
    List,
    Editor,
}

/// Settings › Sub-agents in one of its states.
fn subagents_scene(scene: SubagentsScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use wack_core::agents::{AgentEntry, AgentsConfig, Delegation};
    let connections = fixtures::connections();
    let mut entries: Vec<AgentEntry> = wack_core::agents::builtins().into_iter().map(|agent| AgentEntry { agent, enabled: true, customised: false }).collect();
    // The user's own Review, a role of theirs switched off, and one from Claude Code.
    entries[1].agent.file = Some("/Users/you/Library/Application Support/com.wackcode.desktop/v3/agents/review.md".into());
    entries[1].agent.source = "Your agents".into();
    entries[1].agent.description = "Reviews a change the way the team does: correctness, then naming, then tests. Makes no edits.".into();
    entries[1].customised = true;
    let mut docs = entries[0].agent.clone();
    docs.name = "docs-check".into();
    docs.description = "Checks the docs match the code and lists every place they disagree. Makes no edits.".into();
    docs.file = Some("/x/docs-check.md".into());
    docs.source = "Your agents".into();
    entries.insert(0, AgentEntry { agent: docs, enabled: false, customised: false });
    let mut tester = entries[3].agent.clone();
    tester.name = "test-runner".into();
    tester.description = "Runs the test suite and reports failures with the shortest reproduction it can find.".into();
    tester.file = Some("/Users/you/.claude/agents/test-runner.md".into());
    tester.source = "Claude Code".into();
    tester.tools = Some(vec!["read".into(), "grep".into(), "bash".into()]);
    entries.push(AgentEntry { agent: tester, enabled: true, customised: false });
    let mut state = crate::subagents_page::SubagentsState {
        entries,
        problems: Vec::new(),
        folder: "/Users/you/Library/Application Support/com.wackcode.desktop/v3/agents".into(),
        editor: None,
        error: None,
        helper_open: scene == SubagentsScene::List,
        team_open: false,
    };
    if scene == SubagentsScene::Editor {
        let review = state.entries.iter().find(|entry| entry.agent.name == "review").map(|entry| entry.agent.clone());
        state.editor = Some(crate::subagents_page::AgentEditor::new(review.as_ref(), false, true, cx));
    }
    let config = AgentsConfig {
        delegation: Delegation::Auto,
        helper: Some(wack_core::ModelChoice { connection: Some(connections[0].id.clone()), model: connections[0].models.last().map(|model| model.id.clone()), effort: None }),
        off: vec!["docs-check".into()],
        tool_folders: vec!["claude".into()],
        team_default: Some(wack_core::ModelChoice { connection: Some(connections[0].id.clone()), model: connections[0].models.last().map(|model| model.id.clone()), effort: None }),
    };
    let titles = crate::titles_card::TitlesData::new(&wack_core::AutoTitleConfig { enabled: true, connection: Some(connections[0].id.clone()), model: connections[0].models.last().map(|model| model.id.clone()) }, &connections, true, false, None);
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Subagents,
        providers: ProvidersData::new(&connections, |_| true, &form, cx),
        appearance: fixtures::appearance(None, cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: state.data(&config, &connections, true, titles, cx),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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

/// Composers in a column, each under a caption: for comparing one control's states.
struct ComposerSheet {
    rows: Vec<(&'static str, ComposerData)>,
}

impl ComposerSheet {
    /// A row captioned "Narrow" is drawn this wide, as in a window with the side panel open.
    const NARROW: f32 = 470.;
}

impl Render for ComposerSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::typography::Typography;
        let theme = crate::theme::Theme::get(cx).clone();
        gpui::div().size_full().bg(theme.window).p(px(32.)).flex().flex_col().justify_end().gap(px(22.)).children(self.rows.iter().enumerate().map(|(index, (caption, data))| {
            // Each its own id, so each composer keeps its own state (its card's width).
            gpui::div()
                .id(("composer-row", index))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(gpui::div().typography(Type::Label).text_color(theme.text_dim).child(*caption))
                .child(gpui::div().when(*caption == "Narrow", |row| row.w(px(Self::NARROW))).child(crate::composer::Composer::new(data.clone())))
        }))
    }
}

/// All 25 model provider logos, then composer rows showing On, unknown custom model, and Off.
struct ModelLogosSheet {
    rows: Vec<(&'static str, ComposerData)>,
}

impl Render for ModelLogosSheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::typography::Typography;
        let theme = crate::theme::Theme::get(cx).clone();
        let grid = gpui::div()
            .p(px(14.))
            .rounded(px(16.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .children(crate::brand::Brand::ALL.into_iter().map(|brand| {
                gpui::div()
                    .w(px(136.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded_full()
                    .bg(theme.window)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(brand.view(px(14.), &theme))
                    .child(gpui::div().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_soft).child(brand.name()))
            }));
        let composers = gpui::div().flex().flex_col().gap(px(16.)).children(self.rows.iter().enumerate().map(|(index, (caption, data))| {
            gpui::div()
                .id(("model-logos-composer", index))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(gpui::div().typography(Type::Label).text_color(theme.text_dim).child(*caption))
                .child(crate::composer::Composer::new(data.clone()))
        }));
        gpui::div()
            .size_full()
            .bg(theme.window)
            .p(px(28.))
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                gpui::div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(gpui::div().typography(Type::Label).text_color(theme.text_dim).child("All 25 provider logos (14 px)"))
                    .child(grid),
            )
            .child(composers)
    }
}

pub fn find(name: &str) -> Option<Scene> {
    all().into_iter().find(|scene| scene.name == name)
}

/// Wraps a scene's view so any scene can be the root of a window.
/// A chat in the sample sidebar showing `items`, with a quiet composer so the turn has room.
fn chat_scene(items: Vec<crate::transcript::Item>, cx: &mut App) -> AnyView {
    let composer = fixtures::composer_after_turn(crate::composer::Input::Drawn { text: None, placeholder: "Reply, or ask for the next change".into() });
    cx.new(|_| {
        Shell::showing_tabs(
            fixtures::sidebar(Selected::Working),
            Screen::Chat { title: fixtures::WORKING_TITLE.into(), project: fixtures::project(), body: Body::Items(items), composer },
            fixtures::strip(Some(fixtures::WORKING_TITLE)),
        )
    })
    .into()
}

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
            key: "chat".into(),
            name: "Chat".into(),
            path: "Every Chat chat".into(),
            dir: folder("chat-chat"),
            notes: vec![note(Kind::User, "user_vegetarian", "Cooks vegetarian", "Suggest meat-free recipes", "2026-11-20T19:05:00Z")],
            enabled: true,
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
        prompts: Default::default(),
        tools: Default::default(),
        memory: state.data(cx),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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
            let kind = if matches!(state, TitleScene::RenameSidebar) { Type::Ui } else { Type::Heading };
            let field = cx.new(|cx| {
                let mut field = TextField::new(kind, "Chat name", cx);
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
            chat_area: false,
            archived: None,
            slash: rows.map(crate::slash_menu::SlashData::commands),
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

fn file_mention_scene(slash: crate::slash_menu::SlashData, picture: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    cx.new(|_| {
        let composer = ComposerData {
            chat_area: false,
            archived: None,
            slash: Some(slash),
            ..fixtures::composer_after_turn(Input::Drawn { text: Some("Compare with @slash".into()), placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() })
        };
        let shell = Shell::showing(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat { title: fixtures::FINISHED_TITLE.into(), project: fixtures::project(), body: Body::Items(fixtures::file_mention_turn()), composer },
        );
        match picture {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
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

#[derive(Clone, Copy, PartialEq)]
enum SkillsScene {
    List,
    Empty,
    Editor,
    View,
}

/// Settings › Skills with believable folders.
fn skills_scene(scene: SkillsScene, backdrop: bool, cx: &mut App) -> AnyView {
    use std::path::PathBuf;
    use wack_core::skills::{Document, FolderKind, Problem, Skill, SkillEntry, SkillFolder};
    let skill = |folder: &str, name: &str, description: &str, manual: bool, hint: Option<&str>, source: &str| Skill {
        name: name.into(),
        description: description.into(),
        file: PathBuf::from(format!("/Users/me/{folder}/{name}/SKILL.md")),
        base_dir: PathBuf::from(format!("/Users/me/{folder}/{name}")),
        manual,
        hint: hint.map(Into::into),
        source: source.into(),
    };
    let on = |skill: Skill| SkillEntry { skill, enabled: true, shadowed_by: None };
    let library = SkillFolder {
        id: "library".into(),
        label: "Your skills".into(),
        path: "/Users/me/.agents/skills".into(),
        display: "~/.agents/skills".into(),
        kind: FolderKind::Library,
        enabled: true,
        skills: if scene == SkillsScene::Empty {
            Vec::new()
        } else {
            vec![
                on(skill(".agents/skills", "release-notes", "Drafts release notes from the commits since a tag, grouped by area. Use when asked for release notes or a changelog.", false, Some("[since]"), "Your skills")),
                on(skill(".agents/skills", "review", "Reviews a change the way the team does: correctness first, then naming, then tests.", true, Some("<path>"), "Your skills")),
                SkillEntry { enabled: false, ..on(skill(".agents/skills", "playtest", "Play-test an app hands-on, the way a real first-time user would, and write up what you find.", false, None, "Your skills")) },
            ]
        },
        problems: Vec::new(),
    };
    let claude = SkillFolder {
        id: "claude".into(),
        label: "Claude Code".into(),
        path: "/Users/me/.claude/skills".into(),
        display: "~/.claude/skills".into(),
        kind: FolderKind::Tool,
        enabled: scene != SkillsScene::Empty,
        skills: vec![
            on(skill(".claude/skills", "pdf", "Fill, merge and split PDF forms. Use whenever a .pdf is involved.", false, None, "Claude Code")),
            SkillEntry { shadowed_by: Some("Your skills".into()), ..on(skill(".claude/skills", "review", "Review pull requests.", false, None, "Claude Code")) },
        ],
        problems: vec![Problem { file: "/Users/me/.claude/skills/draft/SKILL.md".into(), message: "It has no description, which the agent decides from.".into() }],
    };
    let tool = |id: &str, label: &str, path: &str| SkillFolder {
        id: id.into(),
        label: label.into(),
        path: format!("/Users/me/{path}").into(),
        display: format!("~/{path}"),
        kind: FolderKind::Tool,
        enabled: false,
        skills: Vec::new(),
        problems: Vec::new(),
    };
    let added = SkillFolder {
        id: "added:/Users/me/Work/team-skills".into(),
        label: "team-skills".into(),
        path: "/Users/me/Work/team-skills".into(),
        display: "~/Work/team-skills".into(),
        kind: FolderKind::Added,
        enabled: true,
        skills: vec![on(skill("Work/team-skills", "deploy-staging", "Deploys the current branch to staging and checks the health endpoint.", true, None, "team-skills"))],
        problems: Vec::new(),
    };
    let mut folders = vec![library, claude.clone(), tool("codex", "Codex", ".codex/skills"), tool("opencode", "OpenCode", ".config/opencode/skills")];
    if scene != SkillsScene::Empty {
        folders.push(added);
    }
    let mut state = crate::skills_page::SkillsState { folders, ..Default::default() };
    match scene {
        SkillsScene::Editor => {
            let document = Document {
                name: "release-notes".into(),
                description: "Drafts release notes from the commits since a tag, grouped by area. Use when asked for release notes or a changelog.".into(),
                manual: false,
                hint: "[since]".into(),
                body: "# Release notes\n\n1. Find the last tag with `git describe --tags --abbrev=0`, or use the one given.\n2. List the commits since it and group them: Features, Fixes, Under the hood.\n3. Write one line per change, in the user's words, not the commit's.".into(),
            };
            let mut editor = crate::skills_page::SkillEditor::new(Some("/Users/me/.agents/skills/release-notes/SKILL.md".into()), Some(&document), cx);
            editor.confirming = true;
            state.editor = Some(editor);
        }
        SkillsScene::View => {
            state.viewing = Some(crate::skills_page::Viewing {
                entry: claude.skills[0].clone(),
                folder: "Claude Code".into(),
                body: "# PDF\n\nUse scripts/fields.py to list a form's fields before filling them.\nNever flatten a form unless the user asks.".into(),
            });
        }
        SkillsScene::List | SkillsScene::Empty => {}
    }
    let picture = backdrop.then(|| over(fixtures::backdrop(wack_core::Texture::None), cx));
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Skills,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(picture.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: state.data(cx),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        match picture {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

#[derive(PartialEq)]
enum PromptsScene {
    Overview,
    Editor,
    Restore,
}

/// Settings › Prompts: Ultra Plan and Automatic titles carry the user's own words; the editor
/// opens Ultra Plan with its built-in text beneath for reference.
fn prompts_scene(scene: PromptsScene, backdrop: bool, cx: &mut App) -> AnyView {
    let custom_plan = "You are planning WackCode itself: favour small, reversible steps, and ask me before anything that touches my other apps.\n\nInterview me one question at a time. Never more than three questions in a plan.";
    let custom_title = "Name this chat like a release note: short, in the project's own words, never a question.";
    let config = wack_core::PromptConfig {
        ultra_plan: Some(custom_plan.into()),
        title: Some(custom_title.into()),
        ..Default::default()
    };
    let mut state = crate::prompts_page::PromptsState::default();
    if scene != PromptsScene::Overview {
        let mut editor = crate::prompts_page::PromptEditor::new(wack_core::PromptId::UltraPlan, &config, cx);
        if scene == PromptsScene::Editor {
            editor.show_default = true;
        } else {
            editor.confirming_restore = true;
        }
        state.editor = Some(editor);
    }
    let picture = backdrop.then(|| over(fixtures::backdrop(wack_core::Texture::None), cx));
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Prompts,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(picture.clone(), cx),
        prompts: state.data(&config, scene == PromptsScene::Overview, cx),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
    };
    cx.new(|_| {
        let shell = Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)));
        match picture {
            Some(picture) => shell.with_backdrop(picture),
            None => shell,
        }
    })
    .into()
}

/// Settings › Commands with two of the user's commands, or the editor on one of them.
fn commands_scene(editing: bool, cx: &mut App) -> AnyView {
    use std::{collections::HashSet, path::PathBuf};
    use wack_core::skills::{CommandEntry, Template, builtin_key};
    let template = |name: &str, description: &str, hint: Option<&str>, body: &str| Template {
        name: name.into(),
        description: description.into(),
        hint: hint.map(Into::into),
        body: body.into(),
        file: PathBuf::from(format!("/Users/me/Library/Application Support/com.wackcode.desktop/v3/commands/{name}.md")),
    };
    let templates = vec![
        template("review", "Review a file the way the team does", Some("<file>"), "Review $1 for correctness first, then naming, then tests. Say what you would change before changing anything."),
        template("explain", "Explain a piece of the code to a newcomer", Some("[topic]"), "Explain ${1:-this project} as you would to someone joining the team today."),
    ];
    let off: HashSet<String> = [builtin_key("copy"), wack_core::skills::file_key(wack_core::skills::Kind::Template, &templates[1].file)].into();
    let entries: Vec<CommandEntry> = wack_core::skills::catalog(&templates, &[], &HashSet::new())
        .into_iter()
        .map(|command| CommandEntry { enabled: !off.contains(&command.key), command })
        .collect();
    let mut state = crate::commands_page::CommandsState {
        entries,
        folder: "/Users/me/Library/Application Support/com.wackcode.desktop/v3/commands".into(),
        ..Default::default()
    };
    if editing {
        let editor = crate::commands_page::CommandEditor::new(Some(&templates[0]), cx);
        editor.sample.update(cx, |field, cx| field.set_text("crates/wack-llm/src/retry.rs", cx));
        state.editor = Some(editor);
    }
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Commands,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(None, cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: state.data(cx),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
    };
    cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(Box::new(data)))).into()
}

/// What Settings › Integrations shows in a scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageScene {
    /// Recording, with a record just written.
    Recording,
    /// Paused.
    Paused,
    /// A write failed: records waiting, one lost, and the failure itself.
    Attention,
}

#[derive(Clone, Copy, PartialEq)]
enum McpScene {
    Empty,
    Servers,
    Deleting,
    EditorCommand,
    EditorUrl,
}

/// Settings › MCP servers, drawn over `backdrop` when there is one.
fn mcp_scene(scene: McpScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use wack_core::mcp::{McpDraft, McpScope, McpServer, McpToolInfo, McpTransport as Transport, Projects, SecretEntry};
    use crate::mcp_page::{EditorData, Live, McpData, McpEditor, ProbeOutcome, RowTest, ServerRow, reach};

    let projects: Vec<(String, gpui::SharedString)> =
        vec![("p1".into(), "wackcode".into()), ("p2".into(), "site".into()), ("p3".into(), "scenes".into()), ("p4".into(), "notes".into())];
    let tool = |name: &str, description: &str, read_only: bool| McpToolInfo { name: name.into(), description: description.into(), read_only };
    let server = |id: &str, name: &str, transport: Transport| McpServer {
        id: id.into(),
        name: name.into(),
        enabled: true,
        transport,
        timeout_ms: 120_000,
        command: String::new(),
        args: Vec::new(),
        url: String::new(),
        headers: Vec::new(),
        env: Vec::new(),
        disabled_tools: Vec::new(),
        tools: Vec::new(),
        scope: McpScope::default(),
    };
    let github = McpServer {
        command: "npx".into(),
        args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        env: vec!["GITHUB_PERSONAL_ACCESS_TOKEN".into()],
        tools: vec![
            tool("search_repositories", "Search for GitHub repositories.", true),
            tool("get_file_contents", "Get the contents of a file or directory from a repository.", true),
            tool("create_issue", "Create a new issue in a GitHub repository.", false),
            tool("delete_branch", "Delete a branch.", false),
        ],
        disabled_tools: vec!["delete_branch".into()],
        ..server("mcp-1", "GitHub", Transport::Stdio)
    };
    let blender = McpServer {
        command: "uvx".into(),
        args: vec!["blender-mcp".into()],
        scope: McpScope::Code { projects: Projects::Only(vec!["p3".into()]) },
        ..server("mcp-2", "Blender", Transport::Stdio)
    };
    let linear = McpServer {
        url: "https://mcp.linear.app/mcp".into(),
        headers: vec!["Authorization".into()],
        timeout_ms: 30_000,
        scope: McpScope::Code { projects: Projects::Except(vec!["p4".into()]) },
        ..server("mcp-3", "Linear", Transport::Http)
    };
    let docs = McpServer {
        url: "http://localhost:8931/sse".into(),
        enabled: false,
        scope: McpScope::Chat,
        ..server("mcp-4", "Docs", Transport::Sse)
    };
    let row = |server: McpServer, live: Live, test: Option<RowTest>| ServerRow {
        reach: reach(&server.scope, &projects),
        scope: server.scope.clone(),
        scope_pending: false,
        server,
        live,
        test,
    };
    let servers = match scene {
        McpScene::Empty => Vec::new(),
        _ => vec![
            row(github.clone(), Live::Connected { tools: 4, folders: 2 }, None),
            row(blender, Live::Connecting, None),
            row(linear, Live::Failed("The server refused the request (HTTP 401). If it needs a token, add it as a header, e.g. Authorization: Bearer <token>.".into()), None),
            row(docs, Live::Idle, None),
        ],
    };
    let editor = match scene {
        McpScene::EditorCommand => {
            let mut draft = McpDraft::of(&github);
            draft.env.push(SecretEntry { name: "GITHUB_HOST".into(), value: None });
            draft.scope = McpScope::Code { projects: Projects::Only(vec!["p1".into(), "p2".into()]) };
            let mut editor = McpEditor::new(&draft, &["GITHUB_PERSONAL_ACCESS_TOKEN".to_string()], cx);
            editor.test = Some(ProbeOutcome::Found {
                elapsed_ms: 1840,
                tools: github.tools.iter().map(|tool| (tool.name.clone().into(), tool.read_only)).collect(),
            });
            Some(EditorData::of(&editor, projects.clone(), false))
        }
        McpScene::EditorUrl => {
            let mut draft = McpDraft::new(Transport::Http);
            draft.name = "Linear".into();
            draft.url = "https://mcp.linear.app/mcp".into();
            draft.headers = vec![SecretEntry { name: "Authorization".into(), value: None }];
            draft.scope = McpScope::All { projects: Projects::Except(vec!["p4".into()]) };
            let editor = McpEditor::new(&draft, &[], cx);
            editor.headers[0].value.update(cx, |field, cx| field.set_text("Bearer lin_api_0000000000", cx));
            let mut editor = editor;
            editor.test = Some(ProbeOutcome::Failed("The server refused the request (HTTP 401). If it needs a token, add it as a header, e.g. Authorization: Bearer <token>.".into()));
            Some(EditorData::of(&editor, projects.clone(), false))
        }
        _ => None,
    };
    let mcp = McpData {
        servers,
        all_projects: projects,
        expanded: matches!(scene, McpScene::Servers | McpScene::Deleting).then(|| "mcp-1".into()),
        confirming_delete: (scene == McpScene::Deleting).then(|| "mcp-1".into()),
        editor,
        error: None,
    };
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Mcp,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp,
        browser: Default::default(),
        computer: Default::default(),
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

#[derive(Clone, Copy)]
enum WebScene {
    Empty,
    Providers,
    Editor,
    Custom,
}

/// Settings › Web, drawn over `backdrop` when there is one.
fn web_scene(scene: WebScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    use wack_core::{
        web::{
            ProviderSummary, SearchDraft,
            search::{CustomSpec, HttpMethod, KeySource, SearchKind, SearchProvider},
        },
        web_usage::Counters,
    };
    use crate::web_page::{EditorData, TestOutcome, WebData, WebEditor};

    let provider = |id: &str, name: &str, kind: SearchKind, key_source: KeySource| SearchProvider {
        id: id.into(),
        name: name.into(),
        kind,
        key_source,
        base_url: (kind == SearchKind::Searxng).then(|| "https://searx.example.org".into()),
        custom: None,
        price_search_per_1k: (kind == SearchKind::Deepseek).then_some(0.75),
        price_fetch_per_1k: (kind == SearchKind::Deepseek).then_some(0.75),
    };
    let providers = match scene {
        WebScene::Empty => Vec::new(),
        _ => vec![
            ProviderSummary {
                provider: provider("deepseek", "DeepSeek", SearchKind::Deepseek, KeySource::Connection { connection: "ds".into() }),
                active: true,
                problem: None,
                usage: Counters { searches: 214, fetches: 38, billed_searches: 209, billed_fetches: 36, failed: 3 },
            },
            ProviderSummary {
                provider: provider("searx", "Home SearXNG", SearchKind::Searxng, KeySource::None),
                active: false,
                problem: None,
                usage: Counters::default(),
            },
            ProviderSummary {
                provider: provider("brave", "Brave Search", SearchKind::Brave, KeySource::Own),
                active: false,
                problem: Some("Add an API key for Brave Search in Settings › Web.".into()),
                usage: Counters::default(),
            },
        ],
    };
    let editor = match scene {
        WebScene::Editor => {
            let mut draft = SearchDraft::new(SearchKind::Deepseek);
            draft.id = Some("deepseek".into());
            draft.key_source = KeySource::Connection { connection: "ds".into() };
            let mut editor = WebEditor::new(&draft, cx);
            editor.test = Some(TestOutcome::Found {
                elapsed_ms: 812,
                hits: vec![
                    ("WackCode — a native macOS coding agent".into(), "github.com".into()),
                    ("Release notes · WackCode".into(), "wackcode.dev".into()),
                    ("Show HN: WackCode".into(), "news.ycombinator.com".into()),
                ],
            });
            let lenders = vec![("ds".to_string(), "DeepSeek".to_string()), ("ds-work".to_string(), "DeepSeek (work)".to_string())];
            Some(EditorData::of(&editor, false, lenders, cx))
        }
        WebScene::Custom => {
            let mut draft = SearchDraft::new(SearchKind::Custom);
            draft.name = "Company search".into();
            draft.base_url = Some("https://search.example.com/api/v2".into());
            draft.custom = Some(CustomSpec {
                method: HttpMethod::Post,
                auth_header: Some("X-API-Key".into()),
                body_template: Some(r#"{"q": "{{query}}", "size": {{count}}}"#.into()),
                results_path: "data.items".into(),
                title_path: "name".into(),
                url_path: "link.href".into(),
                snippet_path: Some("blurb".into()),
                ..CustomSpec::default()
            });
            let mut editor = WebEditor::new(&draft, cx);
            editor.test = Some(TestOutcome::Failed("Company search rejected the API key. Check it in Settings › Web.".into()));
            Some(EditorData::of(&editor, false, Vec::new(), cx))
        }
        _ => None,
    };
    let web = WebData {
        fetch: true,
        search: !matches!(scene, WebScene::Empty),
        route_fetch: matches!(scene, WebScene::Providers),
        providers,
        connection_names: vec![("ds".into(), "DeepSeek".into())],
        editor,
        error: None,
    };
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Web,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web,
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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

/// Settings › Integrations, drawn over `backdrop` when there is one.
fn integrations_scene(scene: UsageScene, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let status = match scene {
        UsageScene::Recording => wack_core::UsageStatus {
            enabled: true,
            path: "/Users/me/Library/Application Support/com.wackcode.desktop/usage/v1".into(),
            has_history: true,
            error: None,
            pending: 0,
            dropped: 0,
            last_written: Some(crate::chat::now_millis() as i64 - 142_000),
        },
        UsageScene::Paused => wack_core::UsageStatus {
            enabled: false,
            path: "/Users/me/Library/Application Support/com.wackcode.desktop/usage/v1".into(),
            has_history: true,
            error: None,
            pending: 0,
            dropped: 0,
            last_written: Some(crate::chat::now_millis() as i64 - 52_000_000),
        },
        UsageScene::Attention => wack_core::UsageStatus {
            enabled: true,
            path: "/Users/me/Library/Application Support/com.wackcode.desktop/usage/v1".into(),
            has_history: true,
            error: Some("Could not save usage. Recording will retry.".into()),
            pending: 2,
            dropped: 1,
            last_written: Some(crate::chat::now_millis() as i64 - 900_000),
        },
    };
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Integrations,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: crate::integrations_page::IntegrationsData { status },
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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

/// Settings › Git, drawn over `backdrop` when there is one.
fn git_scene(commit_attribution: bool, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Git,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: crate::git_page::GitData { commit_attribution, error: None },
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: Default::default(),
        computer: Default::default(),
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

fn browser_settings_scene(clearing: crate::browser_page::Clearing, backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Browser,
        providers: ProvidersData::new(&fixtures::connections(), |_| true, &form, cx),
        appearance: fixtures::appearance(backdrop.clone(), cx),
        prompts: Default::default(),
        tools: Default::default(),
        memory: Default::default(),
        skills: Default::default(),
        commands: Default::default(),
        subagents: Default::default(),
        git: Default::default(),
        integrations: Default::default(),
        web: Default::default(),
        mcp: Default::default(),
        browser: crate::browser_page::BrowserData {
            agent_opens_pane: true,
            default_viewport: wack_browser::Viewport::Desktop,
            clearing,
            version: wack_browser::cef::version().into(),
            error: None,
        },
        computer: Default::default(),
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

/// The lightbox over a chat, as the shell draws it.
struct LightboxScene {
    lightbox: crate::pictures::Lightbox,
}

impl Render for LightboxScene {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shell = cx.new(|_| {
            Shell::showing(
                fixtures::sidebar(Selected::Finished),
                Screen::Chat {
                    title: "Match the login mockups".into(),
                    project: fixtures::project(),
                    body: Body::Items(fixtures::pictures_turn()),
                    composer: fixtures::composer_after_turn(Input::Drawn { text: None, placeholder: crate::chat::FOLLOW_UP_PLACEHOLDER.into() }),
                },
            )
        });
        gpui::div().size_full().relative().child(shell).child(crate::pictures::LightboxView { lightbox: self.lightbox.clone() })
    }
}

/// A finished chat whose transcript is `body`, with a message's menu open when `menu` says.
fn rewind_scene(body: Body, menu: Option<(gpui::Point<gpui::Pixels>, crate::context_menu::PromptFacts)>, cx: &mut gpui::App) -> gpui::AnyView {
    let composer = fixtures::composer_after_turn(crate::composer::Input::Drawn { text: None, placeholder: "Reply, or ask for the next change".into() });
    cx.new(|_| {
        let shell = Shell::showing_tabs(
            fixtures::sidebar(Selected::Finished),
            Screen::Chat { title: fixtures::FINISHED_TITLE.into(), project: fixtures::project(), body, composer },
            fixtures::strip(Some(fixtures::FINISHED_TITLE)),
        );
        match menu {
            Some((position, facts)) => shell.with_prompt_menu(position, facts),
            None => shell,
        }
    })
    .into()
}
