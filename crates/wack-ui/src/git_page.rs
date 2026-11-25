//! Settings › Git: whether coding chats end their commits with a `Co-authored-by` trailer naming
//! the chat's model and the WackCode Agent GitHub App.

use gpui::{FontWeight, SharedString, div, prelude::*, px};
use wack_core::WACKCODE_COAUTHOR_EMAIL;

use crate::{
    actions::ToggleCommitAttribution,
    icon::Icon,
    settings_kit::{card, dispatch, row, staged_head},
    tags::Tag,
    theme::Theme,
    tools_page::switch,
    typography::{Type, Typography},
};

#[derive(Clone, Debug, PartialEq)]
pub struct GitData {
    pub commit_attribution: bool,
    pub error: Option<SharedString>,
}

impl Default for GitData {
    fn default() -> Self {
        Self { commit_attribution: true, error: None }
    }
}

const NOTES: [(Icon, &str, &str); 4] = [
    (Icon::Commit, "Only the agent’s commits", "Commits you make yourself stay yours alone."),
    (Icon::File, "Projects can opt out", "An AGENTS.md or CLAUDE.md that asks for no co-author line wins for that project."),
    (Icon::Comment, "Chat mode never commits", "A Chat mode chat has no project and no Git: its files stay in its scratchpad."),
    (Icon::Lock, "No access to your account", "WackCode Agent is a GitHub App with no permissions that never signs in. The line is plain text."),
];

pub fn page(data: GitData, theme: &Theme) -> impl IntoElement {
    let on = data.commit_attribution;
    let control = div()
        .id("git-commit-attribution")
        .cursor_pointer()
        .on_click(dispatch(ToggleCommitAttribution))
        .child(switch(on, theme))
        .tag("git-commit-attribution");

    let attribution = card(
        Some("Commit attribution"),
        Some(if on { "Co-authoring" } else { "Off" }.into()),
        None,
        theme,
    )
    .child(
        row(
            "WackCode as co-author",
            Some("When WackCode commits for you, it ends the message with a Co-authored-by line naming the model it ran on. GitHub then shows WackCode Agent beside you on the commit, so you can see how much it helped."),
        )
        .control(control)
        .view(theme),
    )
    .when_some(data.error, |card, error| {
        card.child(div().px(px(18.)).pb(px(14.)).typography(Type::Ui).text_color(theme.danger).child(error))
    });

    let trailer = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(3.))
        .gap_y(px(4.))
        .when(!on, |line| line.opacity(0.45))
        .child(
            div()
                .typography(Type::Mono)
                .text_color(theme.text_soft)
                .when(!on, |text| text.line_through())
                .child("Co-authored-by:"),
        )
        .child(
            div()
                .px(px(4.))
                .rounded(px(4.))
                .bg(if on { theme.accent_wash } else { theme.hover })
                .border_1()
                .border_color(if on { theme.accent_line } else { theme.border })
                .typography(Type::Small)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if on { theme.accent_text } else { theme.text_dim })
                .when(!on, |pill| pill.line_through())
                .child("the chat’s model"),
        )
        .child(
            div()
                .typography(Type::Mono)
                .text_color(theme.text_soft)
                .when(!on, |text| text.line_through())
                .child(format!("(WackCode) <{WACKCODE_COAUTHOR_EMAIL}>")),
        );

    let preview = card(Some("The line it adds"), None, None, theme).child(
        div()
            .px(px(18.))
            .py(px(14.))
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().typography(Type::Small).text_color(theme.text_dim).child(if on {
                "The last line of each commit message the agent writes, after a blank line. GitHub matches the address, so the name can say anything."
            } else {
                "Switched off: the agent’s commits carry no co-author line."
            }))
            .child(
                div()
                    .w_full()
                    .px(px(10.))
                    .py(px(11.))
                    .rounded(px(10.))
                    .bg(theme.window)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(div().typography(Type::Mono).text_color(theme.text).child("Fix the flaky upload test"))
                    .child(trailer)
                    .tag("git-trailer-preview"),
            ),
    );

    let mut notes = card(Some("Good to know"), None, None, theme);
    for (icon, title, detail) in NOTES {
        notes = notes.child(
            div()
                .px(px(18.))
                .py(px(12.))
                .border_t_1()
                .border_color(theme.border)
                .flex()
                .items_start()
                .gap(px(14.))
                .child(
                    div()
                        .flex_none()
                        .size(px(30.))
                        .rounded(px(9.))
                        .bg(theme.window)
                        .border_1()
                        .border_color(theme.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon.view(px(15.), theme.accent_text)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().typography(Type::UiStrong).text_color(theme.text).child(title))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child(detail)),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .child(staged_head(crate::settings::Section::Git, 
            "Git",
            "How WackCode attributes commits it writes for you. Changes apply from a chat’s next message.",
            None,
            theme,
        ))
        .child(attribution)
        .child(div().pt(px(18.)).child(preview))
        .child(div().pt(px(18.)).child(notes))
}
