//! Scrolls a saved chat in a real window, as a trackpad would, and reports the frames the screen
//! got: how many a second, and how many came late. The gallery's headless pictures cannot show
//! this, and the GPU's share of a frame (a live blur, say) only shows in a real window.
//!
//! ```text
//! cargo run -p gallery --example scroll_window -- <session.jsonl> [seconds] [blur]
//! ```
//!
//! `blur` draws it over the gallery's backdrop with that much blur, in points; leave it out for no
//! picture. The chat is drawn in the gallery's sample sidebar, from a session file (a dev app chat
//! in `…/com.wackcode.desktop.dev/v3/sessions/`), which is only read. A window opens for the
//! run plus two seconds, then the app quits.

use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};

use gpui::{App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, WindowBounds, WindowOptions, px, size};
use wack_ui::{
    chat::ChatSession,
    fixtures,
    panes::layout::Layout,
    shell::{Body, Screen, Shell},
    theme::{Appearance, Theme, ThemeInput, ThemeName},
};

struct Probe {
    shell: gpui::AnyView,
    frames: Rc<RefCell<Vec<Instant>>>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.frames.borrow_mut().push(Instant::now());
        gpui::div().size_full().child(self.shell.clone())
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("session path");
    let seconds: u64 = std::env::args().nth(2).map(|n| n.parse().unwrap()).unwrap_or(6);
    let blur: Option<f32> = std::env::args().nth(3).map(|n| n.parse().unwrap());
    gpui_platform::application().with_assets(wack_ui::icon::Assets).run(move |cx: &mut App| {
        wack_ui::init(cx);
        cx.set_global(Theme::derive(ThemeInput { appearance: Appearance::Dark, theme: ThemeName::default(), ground: None }));
        let scratch = tempfile::tempdir().unwrap();
        let tmp = scratch.path().to_path_buf();
        let frames = Rc::new(RefCell::new(Vec::new()));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1680.), px(1050.)), cx))),
            ..Default::default()
        };
        let probe_frames = frames.clone();
        let window = cx
            .open_window(options, move |_, cx| {
                let session = cx.new(|cx| {
                    ChatSession::load(
                        "bench",
                        tmp.clone(),
                        std::path::Path::new(&path),
                        Layout::from_saved(None),
                        wack_core::runner::Runner::new(),
                        (None, None),
                        tmp.join("tasks"),
                        wack_browser::ChatTabs::new(None),
                        wack_canvas::Canvases::new(tmp.join("canvases"), None),
                        cx,
                    )
                });
                let state = session.read(cx).list.clone();
                let composer = fixtures::composer_after_turn(wack_ui::composer::Input::Drawn { text: None, placeholder: "Reply".into() });
                let shell = cx.new(|_| {
                    let shell = Shell::showing_tabs(
                        fixtures::sidebar(fixtures::Selected::Working),
                        Screen::Chat { title: "Bench".into(), project: fixtures::project(), body: Body::Live(session, state), composer },
                        fixtures::strip(Some("Bench")),
                    );
                    match blur {
                        Some(blur) => {
                            let mut picture = fixtures::backdrop(wack_core::Texture::None);
                            picture.blur = blur;
                            shell.with_backdrop(picture)
                        }
                        None => shell,
                    }
                });
                cx.new(|_| Probe { shell: shell.into(), frames: probe_frames })
            })
            .unwrap();
        cx.spawn(async move |cx| {
            // Let pictures and baked copies arrive.
            cx.background_executor().timer(Duration::from_secs(2)).await;
            frames.borrow_mut().clear();
            let start = Instant::now();
            let mut tick = 0u64;
            while start.elapsed() < Duration::from_secs(seconds) {
                // A trackpad's stream of small moves, up for a second and a half, then down.
                let dy = if (tick / 180).is_multiple_of(2) { 6.5 } else { -6.5 };
                let _ = window.update(cx, |_, window, cx| {
                    window.dispatch_event(
                        gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                            position: gpui::point(px(1000.), px(500.)),
                            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
                            modifiers: Default::default(),
                            touch_phase: gpui::TouchPhase::Moved,
                        }),
                        cx,
                    );
                });
                tick += 1;
                cx.background_executor().timer(Duration::from_micros(8333)).await;
            }
            let frames = frames.borrow();
            let mut gaps: Vec<f64> = frames.windows(2).map(|pair| (pair[1] - pair[0]).as_secs_f64() * 1000.).collect();
            let fps = frames.len() as f64 / seconds as f64;
            gaps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let long = gaps.iter().filter(|gap| **gap > 20.).count();
            println!(
                "blur {:?}: {} frames in {}s = {:.1} fps; gap p50 {:.1}ms p90 {:.1}ms p99 {:.1}ms; {} gaps over 20ms",
                blur,
                frames.len(),
                seconds,
                fps,
                gaps[gaps.len() / 2],
                gaps[gaps.len() * 9 / 10],
                gaps[gaps.len() * 99 / 100],
                long
            );
            drop(scratch);
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
