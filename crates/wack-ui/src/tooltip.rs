//! A tooltip: one line of small text on a raised card, for controls whose word or icon does not
//! say everything (a key, what a second click does).

use gpui::{AnyView, App, Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};

use crate::{
    theme::Theme,
    typography::{Type, Typography},
};

struct Tip {
    text: SharedString,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx);
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(7.))
            .bg(theme.raised)
            .border_1()
            .border_color(theme.border_strong)
            .shadow_md()
            .typography(Type::Small)
            .text_color(theme.text_soft)
            .child(self.text.clone())
    }
}

/// A builder for `.tooltip(...)` showing `text`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip { text: text.clone() }).into()
}
