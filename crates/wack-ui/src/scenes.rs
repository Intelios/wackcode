//! Named UI scenes: a view in a known state at a known size.
//!
//! The `gallery` binary renders these to PNG files, which is how UI work is checked without
//! launching the app. Add a scene for every component state worth looking at.

use gpui::{AnyView, App, Context, Pixels, Size, Window, prelude::*, px, size};

use crate::{
    fixtures::{self, Selected},
    shell::{Screen, Shell},
    specimen::Specimen,
    tags,
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
                cx.new(|_| Shell::showing(Default::default(), Screen::Welcome)).into()
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
                        },
                    )
                })
                .into()
            },
        },
        Scene {
            name: "chat",
            description: "A finished turn: prompt, work log with an open diff, reply with code.",
            size: size(px(1180.), px(1080.)),
            build: |_, cx| {
                cx.new(|_| {
                    Shell::showing(
                        fixtures::sidebar(Selected::Finished),
                        Screen::Chat {
                            title: fixtures::FINISHED_TITLE.into(),
                            project: fixtures::project(),
                            items: fixtures::finished_turn(),
                            composer: fixtures::composer_after_turn(),
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
                            items: fixtures::working_turn(),
                            composer: fixtures::composer_working(),
                        },
                    )
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
