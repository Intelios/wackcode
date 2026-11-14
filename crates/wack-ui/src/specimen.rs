//! A specimen sheet for the gallery: every text style and every colour token, as the theme
//! currently derives them. Render it with different `--colour` and `--accent` values to see what a
//! theme choice does to the whole palette.

use gpui::{App, Context, Hsla, Window, div, prelude::*, px};

use crate::{
    icon::Icon,
    tags,
    theme::{self, Theme},
    typography::{Type, Typography},
};

pub struct Specimen;

impl Render for Specimen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        tags::root(
            div()
                .size_full()
                .flex()
                .gap(px(56.))
                .px(px(64.))
                .pt(px(56.))
                .bg(theme.window)
                .text_color(theme.text)
                .child(type_column(&theme))
                .child(colour_column(&theme, cx)),
        )
    }
}

fn type_column(theme: &Theme) -> impl IntoElement {
    let sample = |kind: Type, caption: &'static str, text: &'static str, colour: Hsla| {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().typography(Type::Small).text_color(theme.text_faint).child(caption))
            .child(div().typography(kind).text_color(colour).child(text))
    };
    div()
        .w(px(500.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(20.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(Icon::Duck.view(px(22.), theme.text))
                .child(div().typography(Type::Title).child("WackCode type and colour")),
        )
        .child(sample(Type::Display, "Display · New York 44 medium", "wackcode", theme.text))
        .child(sample(Type::Title, "Title · New York 24 medium", "Settings › Providers", theme.text))
        .child(sample(
            Type::Prompt,
            "Prompt · New York 18",
            "The SSE decoder drops the last event when a stream ends without a blank line.",
            theme.text,
        ))
        .child(sample(Type::Label, "Label · New York 13.5 italic", "Pinned", theme.text_dim))
        .child(sample(Type::Heading, "Heading · SF Pro 15 semibold", "What changed", theme.text))
        .child(sample(
            Type::Body,
            "Body · SF Pro 14 / 22",
            "Fixed. The decoder now flushes what it has buffered when the stream ends, so a final \
             event without its blank line is delivered instead of dropped.",
            theme.text_body,
        ))
        .child(sample(Type::Ui, "Ui · SF Pro 13", "Port the theme derivation", theme.text_soft))
        .child(sample(Type::Small, "Small · SF Pro 11.5", "19s · GLM-5.3 Flash · 12.4k tokens", theme.text_dim))
        .child(sample(Type::Mono, "Mono · SF Mono 12", "cargo test -p wack-llm sse", theme.text_soft))
}

fn colour_column(theme: &Theme, _cx: &App) -> impl IntoElement {
    let group = |name: &'static str, tokens: Vec<(&'static str, Hsla)>| {
        let mut grid = div().flex().flex_wrap().gap_y(px(10.));
        for (token, colour) in tokens {
            grid = grid.child(
                div()
                    .w(px(162.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(30.))
                            .rounded(px(7.))
                            .bg(colour)
                            .border_1()
                            .border_color(theme.border),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .typography(Type::Small)
                            .child(div().text_color(theme.text_soft).child(token))
                            .child(div().text_color(theme.text_faint).child(theme::hex(colour))),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().typography(Type::Label).text_color(theme.text_dim).child(name))
            .child(grid)
    };
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(group(
            "Surfaces",
            vec![
                ("sidebar", theme.sidebar),
                ("window", theme.window),
                ("surface", theme.surface),
                ("raised", theme.raised),
                ("hover", theme.hover),
                ("border", theme.border),
                ("border_strong", theme.border_strong),
                ("code_ground", theme.code_ground),
            ],
        ))
        .child(group(
            "Ink",
            vec![
                ("text", theme.text),
                ("text_body", theme.text_body),
                ("text_soft", theme.text_soft),
                ("text_dim", theme.text_dim),
                ("text_faint", theme.text_faint),
            ],
        ))
        .child(group(
            "Interaction",
            vec![
                ("accent", theme.accent),
                ("accent_hover", theme.accent_hover),
                ("on_accent", theme.on_accent),
                ("accent_text", theme.accent_text),
                ("accent_wash", theme.accent_wash),
                ("accent_line", theme.accent_line),
            ],
        ))
        .child(group(
            "Meaning (fixed)",
            vec![
                ("danger", theme.danger),
                ("warning", theme.warning),
                ("success", theme.success),
                ("diff_added", theme.diff_added),
                ("diff_removed", theme.diff_removed),
                ("added_ground", theme.diff_added_ground),
                ("removed_ground", theme.diff_removed_ground),
            ],
        ))
        .child(group(
            "Code",
            vec![
                ("keyword", theme.code_keyword),
                ("string", theme.code_string),
                ("number", theme.code_number),
                ("title", theme.code_title),
                ("meta", theme.code_meta),
                ("comment", theme.code_comment),
            ],
        ))
}
