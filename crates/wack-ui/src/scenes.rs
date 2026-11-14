//! Named UI scenes: a view in a known state at a known size.
//!
//! The `gallery` binary renders these to PNG files, which is how UI work is checked without
//! launching the app. Add a scene for every component state worth looking at.

use gpui::{AnyView, App, Context, Pixels, Size, Window, prelude::*, px, size};

use crate::{
    composer::{ComposerData, Input},
    settings::{ProviderForm, ProvidersData},
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
                    Shell::showing(
                        fixtures::sidebar(Selected::None),
                        Screen::NewChat {
                            project: fixtures::project(),
                            composer: fixtures::composer_empty(true),
                            notice: None,
                        },
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
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::finished_turn()),
                            composer: fixtures::composer_after_turn(Input::Field(field)),
                        },
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
                    Shell::showing(
                        fixtures::sidebar(Selected::Working),
                        Screen::Chat {
                            title: fixtures::WORKING_TITLE.into(),
                            project: fixtures::project(),
                            body: Body::Items(fixtures::working_turn()),
                            composer: fixtures::composer_working(),
                        },
                    )
                })
                .into()
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
                let data = ProvidersData::new(&fixtures::connections(), &form);
                cx.new(|_| Shell::showing(fixtures::sidebar(Selected::None), Screen::Settings(data))).into()
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
