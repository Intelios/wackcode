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
            description: "The smallest window, 700 points: the sidebar steps aside and the transcript keeps its minimum.",
            size: size(px(700.), px(720.)),
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
            build: |_, cx| settings_scene(Some(fixtures::backdrop(wack_core::Texture::None)), cx),
        },
        Scene {
            name: "new-chat-backdrop",
            description: "The new-chat screen over a backdrop: clear across the top, dissolving around the centred block.",
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
                    .with_backdrop(fixtures::backdrop(wack_core::Texture::None))
                })
                .into()
            },
        },
        Scene {
            name: "new-chat-backdrop-ascii",
            description: "The new-chat screen over a backdrop drawn through the ASCII texture.",
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
                    .with_backdrop(fixtures::backdrop(wack_core::Texture::Ascii))
                })
                .into()
            },
        },
        Scene {
            name: "chat-backdrop",
            description: "A finished turn over a backdrop, at a laptop's full-screen size.",
            size: size(px(1512.), px(945.)),
            build: |_, cx| {
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
                    .with_backdrop(fixtures::backdrop(wack_core::Texture::None))
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

/// Settings › Appearance, drawn over `backdrop` when there is one.
fn settings_scene(backdrop: Option<crate::backdrop::Picture>, cx: &mut App) -> AnyView {
    let form = ProviderForm::new(cx);
    let data = SettingsData {
        section: Section::Appearance,
        providers: ProvidersData::new(&fixtures::connections(), &form),
        appearance: fixtures::appearance(backdrop.clone(), cx),
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
