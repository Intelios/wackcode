//! The composer: where the user writes to the agent.
//!
//! One card, no toolbar of pills: the model and effort are plain words you click, the context
//! meter is a number, and the one filled control is the send button (the stop button while the
//! agent works). Queued messages sit just above the card, so the user sees what will go next.
//!
//! Renders from plain data ([`ComposerData`]). Text editing arrives with `gpui-base`
//! (docs/gpui.md); until then the text is drawn, not edited.

use gpui::{App, SharedString, Window, div, prelude::*, px};

use crate::{
    icon::Icon,
    theme::Theme,
    typography::{Type, Typography},
};

#[derive(Clone, Debug)]
pub struct ComposerData {
    /// What the user has typed; the placeholder shows when `None`.
    pub text: Option<SharedString>,
    pub placeholder: SharedString,
    pub model: SharedString,
    pub effort: SharedString,
    /// Share of the context window in use, 0–1, once the chat has sent anything.
    pub context: Option<f32>,
    pub working: bool,
    /// Messages waiting for the current run to finish.
    pub queued: Vec<SharedString>,
    /// A taller card for the new-chat screen.
    pub roomy: bool,
}

#[derive(IntoElement)]
pub struct Composer {
    data: ComposerData,
}

impl Composer {
    pub fn new(data: ComposerData) -> Self {
        Self { data }
    }
}

impl RenderOnce for Composer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let data = self.data;
        let has_text = data.text.is_some();

        let send = if data.working {
            div()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.border_strong)
                .child(Icon::Stop.view(px(14.), theme.text))
        } else {
            div()
                .size(px(30.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if has_text { theme.accent } else { theme.hover })
                .child(Icon::ArrowUp.view(
                    px(15.),
                    if has_text { theme.on_accent } else { theme.text_faint },
                ))
        };

        let card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(10.))
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .min_h(px(if data.roomy { 64. } else { 22. }))
                    .typography(Type::Body)
                    .map(|text| match data.text {
                        Some(typed) => text.text_color(theme.text).child(typed),
                        None => text.text_color(theme.text_faint).child(data.placeholder),
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(Icon::Plus.view(px(16.), theme.text_dim))
                    .child(picker(data.model, theme.text_soft, &theme))
                    .child(picker(data.effort, theme.text_dim, &theme))
                    .child(div().flex_1())
                    .when_some(data.context, |row, used| {
                        row.child(
                            div()
                                .typography(Type::Small)
                                .text_color(if used >= 0.8 {
                                    theme.warning
                                } else {
                                    theme.text_faint
                                })
                                .child(format!("{}% context", (used * 100.).round())),
                        )
                    })
                    .child(send),
            );

        let mut column = div().w_full().flex().flex_col().gap(px(6.));
        for message in data.queued {
            column = column.child(
                div()
                    .mx(px(14.))
                    .px(px(12.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(9.))
                    .bg(theme.sidebar)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .typography(Type::Small)
                            .text_color(theme.text_dim)
                            .child("Next"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .typography(Type::Ui)
                            .text_color(theme.text_soft)
                            .child(message),
                    ),
            );
        }
        column.child(card)
    }
}

/// A word that opens a menu: the model, the effort.
fn picker(name: SharedString, colour: gpui::Hsla, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(3.))
        .child(div().typography(Type::Ui).text_color(colour).child(name))
        .child(Icon::ChevronDown.view(px(12.), theme.text_faint))
}
