//! The window's root view.
//!
//! The shell owns the title strip: the app draws its own, so the strip is also what moves the
//! window.

use gpui::{
    Context, FontWeight, MouseButton, Pixels, Window, WindowControlArea, div, prelude::*, px,
};

use crate::{
    tags::{self, Tag},
    theme::Theme,
};

/// Height of the strip across the top that holds the traffic lights and drags the window.
pub const TITLE_STRIP_HEIGHT: Pixels = px(40.);

pub struct Shell;

impl Shell {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.text)
            .text_size(px(14.))
            .child(
                div()
                    .id("title-strip")
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
                    .tag("title-strip"),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .gap(px(6.))
                    .p(px(28.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("WackCode")
                            .tag("wordmark"),
                    )
                    .child(
                        div()
                            .text_color(theme.text_dim)
                            .child("Version 3 foundation. Nothing to open yet."),
                    ),
            );
        tags::root(content)
    }
}
