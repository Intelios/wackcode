//! Settings › Tools: what the agent's tools may do. The Planning card has version 2's "Remove Plan /
//! Ultra Plan read-only restrictions", which asks inside the card before it turns on: it lets a
//! planning run change things. Turning it off needs no asking. The Web card switches Web Fetch.

use gpui::{div, prelude::*, px};

use crate::{
    actions::{CancelUnrestrictedPlanning, ConfirmUnrestrictedPlanning, OpenSettingsSection, ToggleUnrestrictedPlanning, ToggleWebFetch},
    button::{Button, Kind},
    settings::Section,
    settings_kit::{card, dispatch, pill, row, staged_head},
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
    /// The agent may read public web pages.
    pub web_fetch: bool,
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
        .when(on, |track| track.justify_end().bg(theme.accent_fill()))
        .when(!on, |track| track.bg(theme.border_strong))
        .child(div().size(px(16.)).rounded_full().bg(if on { theme.on_accent } else { theme.surface }))
}

pub fn page(data: ToolsData, theme: &Theme) -> impl IntoElement {
    let control = div()
        .id("unrestricted-planning")
        .cursor_pointer()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleUnrestrictedPlanning), cx))
        .child(switch(data.unrestricted_planning, theme))
        .tag("unrestricted-planning");

    let web_fetch = div()
        .id("web-fetch")
        .cursor_pointer()
        .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleWebFetch), cx))
        .child(switch(data.web_fetch, theme))
        .tag("web-fetch");

    let confirm = data.confirming.then(|| {
        div()
            .mx(px(18.))
            .mb(px(16.))
            .p(px(14.))
            .rounded(px(12.))
            .bg(theme.window)
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
        .child(staged_head(crate::settings::Section::Tools, "Tools", "What the agent may do while it plans, and what it may reach beyond this Mac.", None, theme))
        .child(
            card(Some("Planning"), None, None, theme)
                .child(
                    row(
                        "Remove Plan / Ultra Plan read-only restrictions",
                        Some("Both planning modes can use all enabled tools, including tools that change connected services. The interview, plan review and approval before implementation still apply."),
                    )
                    .control(control)
                    .view(theme),
                )
                .children(confirm),
        )
        .child(
            div().pt(px(18.)).child(
                card(Some("Web"), None, None, theme).child(
                    row(
                        "Web Fetch",
                        Some("Reads public web pages the agent has a link to. It never reaches this Mac or your local network, and sends no cookies or sign-ins."),
                    )
                    .control(web_fetch)
                    .view(theme),
                ),
            ),
        )
        .child(
            div().pt(px(18.)).child(
                card(Some("Elsewhere"), None, None, theme).child(
                    row("Memory", Some("Notes it saves and recalls, per project."))
                        .control(pill("tools-memory", "Open", None, false, theme).cursor_pointer().on_click(dispatch(OpenSettingsSection { section: Section::Memory })))
                        .view(theme),
                ),
            ),
        )
}
