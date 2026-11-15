//! Settings › Tools: what the agent's tools may do. For now one switch, version 2's "Remove Plan /
//! Ultra Plan read-only restrictions", which asks before it turns on: it lets a planning run
//! change things. Turning it off needs no asking.

use gpui::{div, prelude::*, px};

use crate::{
    actions::{CancelUnrestrictedPlanning, ConfirmUnrestrictedPlanning, ToggleUnrestrictedPlanning},
    button::{Button, Kind},
    tags::Tag,
    theme::Theme,
    typography::{Type, Typography},
};

#[derive(Clone, Debug, Default)]
pub struct ToolsData {
    /// Plan and Ultra Plan may use every tool.
    pub unrestricted_planning: bool,
    /// Turning it on is waiting for the user to confirm.
    pub confirming: bool,
}

/// An on / off switch, drawn from theme tokens.
pub fn switch(on: bool, theme: &Theme) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .p(px(2.))
        .rounded_full()
        .flex()
        .when(on, |track| track.justify_end().bg(theme.accent))
        .when(!on, |track| track.bg(theme.border_strong))
        .child(div().size(px(16.)).rounded_full().bg(if on { theme.on_accent } else { theme.surface }))
}

pub fn page(data: ToolsData, theme: &Theme) -> impl IntoElement {
    let row = div()
        .id("unrestricted-planning")
        .pt(px(30.))
        .flex()
        .items_start()
        .gap(px(16.))
        .cursor_pointer()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleUnrestrictedPlanning), cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().typography(Type::Heading).text_color(theme.text).child("Remove Plan / Ultra Plan read-only restrictions"))
                .child(div().max_w(px(520.)).typography(Type::Ui).text_color(theme.text_dim).child(
                    "Both planning modes can use all enabled tools, including tools that change connected services. The interview, plan review and approval before implementation still apply.",
                )),
        )
        .child(div().pt(px(2.)).child(switch(data.unrestricted_planning, theme)))
        .tag("unrestricted-planning");

    let confirm = data.confirming.then(|| {
        div()
            .mt(px(16.))
            .p(px(16.))
            .rounded(px(12.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border_strong)
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().typography(Type::UiStrong).text_color(theme.text).child("Remove planning restrictions?"))
                    .child(div().typography(Type::Ui).text_color(theme.text_dim).child(
                        "Applies to all chats from their next turn and stays enabled until you switch it off.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("planning-keep", "Keep restrictions", Kind::Secondary).action(CancelUnrestrictedPlanning))
                    .child(Button::new("planning-remove", "Remove restrictions", Kind::Danger).action(ConfirmUnrestrictedPlanning)),
            )
    });

    div()
        .flex()
        .flex_col()
        .child(div().typography(Type::Title).text_color(theme.text).child("Tools"))
        .child(div().pt(px(8.)).max_w(px(560.)).typography(Type::Body).text_color(theme.text_soft).child(
            "What the agent may do while it plans. Building always has every tool.",
        ))
        .child(row)
        .children(confirm)
}
