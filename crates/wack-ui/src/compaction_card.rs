//! Compaction summary card: an inspectable boundary showing what the summary carried into the
//! model's context, ported from version 2's `CompactionRow.tsx` and `CompactionNote`.
//!
//! - Collapsed by default so a chat stays scannable; clicking opens the structured summary
//!   (model body, feature sections, files changed/read, and quoted user messages) plus token
//!   counts and a Copy button.
//! - Expansion state lives in the chat's `open` set via `Handlers::toggle`, never in this view.
//! - Markdown is parsed lazily through [`ReplyCache::compaction`] only while expanded, so closed
//!   compactions cost nothing to rebuild.

use std::rc::Rc;

use gpui::{AnyElement, SharedString, div, prelude::*, px};
use wack_core::compaction::Summary;

use crate::{
    conversation::Compaction,
    copy::CopyButton,
    icon::Icon,
    markdown,
    tags::Tag,
    theme::Theme,
    tooltip::tip,
    transcript::Handlers,
    typography::{MONO_FONT, Type, Typography},
    work::{self, ReplyCache},
};

/// A compaction boundary ready to draw in either the Code or Chat transcript.
#[derive(Clone, Debug)]
pub struct CompactionCard {
    /// Stable toggle key (`compact-{entry}` or `turn-{entry}-compact-{step}`).
    pub key: SharedString,
    /// Context size before compaction, when recorded.
    pub tokens_before: Option<u64>,
    /// Estimated context size after compaction, when recorded.
    pub tokens_after: Option<u64>,
    /// Human-readable Markdown of the summary (copied by Copy, parsed into `blocks` when open).
    pub text: SharedString,
    /// Parsed Markdown blocks when `open` is true; empty while collapsed.
    pub blocks: Rc<Vec<markdown::Block>>,
    pub open: bool,
}

/// Builds a [`CompactionCard`] from a conversation's compaction record.
pub fn from_compaction(compaction: &Compaction, key: &str, open: bool, cache: &mut ReplyCache) -> CompactionCard {
    let text = format_summary(&compaction.summary);
    let blocks = if open && !text.is_empty() {
        cache.compaction(key, &text)
    } else {
        Rc::new(Vec::new())
    };
    CompactionCard {
        key: key.to_string().into(),
        tokens_before: compaction.tokens_before,
        tokens_after: compaction.tokens_after,
        text: text.into(),
        blocks,
        open,
    }
}

/// Renders a [`Summary`] into clean Markdown for reading in the transcript, leaving out the
/// prompt-injection framing line meant only for the model.
pub fn format_summary(summary: &Summary) -> String {
    let mut sections: Vec<String> = Vec::new();
    let body = summary.body.trim();
    if !body.is_empty() {
        sections.push(body.to_string());
    }
    for section in &summary.sections {
        let trimmed = section.trim();
        if !trimmed.is_empty() {
            sections.push(trimmed.to_string());
        }
    }
    if !summary.modified_files.is_empty() {
        sections.push(format!("## Files changed\n{}", file_bullets(&summary.modified_files)));
    }
    if !summary.read_files.is_empty() {
        sections.push(format!("## Files read\n{}", file_bullets(&summary.read_files)));
    }
    if let Some((first, rest)) = summary.user_messages.split_first() {
        let mut parts = vec!["## The user's messages".to_string(), blockquote(first)];
        if summary.omitted_user_messages > 0 {
            let noun = if summary.omitted_user_messages == 1 { "message" } else { "messages" };
            parts.push(format!("*{} earlier {noun} left out to save space*", summary.omitted_user_messages));
        }
        parts.extend(rest.iter().map(|message| blockquote(message)));
        sections.push(parts.join("\n\n"));
    }
    sections.join("\n\n")
}

fn file_bullets(paths: &[String]) -> String {
    paths.iter().map(|path| format!("- {}", inline_code(path))).collect::<Vec<_>>().join("\n")
}

fn inline_code(text: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    if text.starts_with('`') || text.ends_with('`') || text.starts_with(' ') || text.ends_with(' ') {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

fn blockquote(text: &str) -> String {
    text.trim()
        .lines()
        .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Comma-grouped token count: `34_120` → `"34,120"`.
pub fn format_exact(tokens: u64) -> String {
    let digits = tokens.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Compact token delta for the card header: `"34.1k → ≈8.5k tokens"`, `"34.1k before"`, or `None`.
pub fn header_counts(before: Option<u64>, after: Option<u64>) -> Option<String> {
    match (before, after) {
        (Some(before), Some(after)) => Some(format!("{} → ≈{} tokens", work::count(before), work::count(after))),
        (Some(before), None) => Some(format!("{} before", work::count(before))),
        (None, Some(after)) => Some(format!("≈{} tokens", work::count(after))),
        (None, None) => None,
    }
}

/// Code transcript view: a calm rounded disclosure card with token counts and the rendered summary.
pub fn view(card: &CompactionCard, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let toggle = handlers.toggle.clone();
    let key = card.key.clone();
    let counts = header_counts(card.tokens_before, card.tokens_after);

    let head = div()
        .id(card.key.clone())
        .min_h(px(38.))
        .pl(px(10.))
        .pr(px(12.))
        .py(px(7.))
        .flex()
        .items_center()
        .gap(px(9.))
        .cursor_pointer()
        .hover(|row| row.bg(theme.hover))
        .tooltip(tip(if card.open { "Hide compaction summary" } else { "Show compaction summary" }))
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .child(
            div()
                .flex_none()
                .size(px(22.))
                .rounded_full()
                .bg(theme.accent_wash)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::Archive.view(px(12.), theme.accent_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .typography(Type::UiStrong)
                .text_color(if card.open { theme.text } else { theme.text_soft })
                .child("Context compacted"),
        )
        .when_some(counts, |row, counts| {
            row.child(div().flex_none().typography(Type::Small).font_family(MONO_FONT).text_color(theme.text_dim).child(counts))
        })
        .child(if card.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint))
        .tag("compaction-toggle");

    let body = card.open.then(|| {
        let has_counts = card.tokens_before.is_some() || card.tokens_after.is_some();
        let mut copy = (!card.text.is_empty()).then(|| {
            CopyButton::new(format!("{}-copy", card.key), card.text.clone(), "Copy summary", "copy-compaction-summary")
        });
        let counts_row = has_counts.then(|| {
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(24.))
                        .gap_y(px(8.))
                        .when_some(card.tokens_before, |stats, before| {
                            stats.child(stat_pair("Context before", format!("{} tokens", format_exact(before)), theme))
                        })
                        .when_some(card.tokens_after, |stats, after| {
                            stats.child(stat_pair("Estimated context after", format!("≈{} tokens", format_exact(after)), theme))
                        }),
                )
                .children(copy.take())
        });
        let hint_row = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .typography(Type::Small)
                    .text_color(theme.text_dim)
                    .child("Summary carried into the agent’s context. Earlier messages remain in this chat."),
            )
            .children(copy.take());

        div()
            .border_t_1()
            .border_color(theme.border)
            .px(px(16.))
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(counts_row)
            .child(hint_row)
            .child(summary_content(&card.blocks, theme, handlers))
            .tag("compaction-body")
    });

    div()
        .rounded(px(14.))
        .bg(theme.surface)
        .border_1()
        .border_color(if card.open { theme.border_strong } else { theme.border })
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(head)
        .children(body)
        .tag("compaction-card")
        .into_any_element()
}

/// Chat area view: centered pill ("Tidied up the conversation") opening into the summary sheet.
pub fn chat_view(card: &CompactionCard, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let toggle = handlers.toggle.clone();
    let key = card.key.clone();

    let button = div()
        .id(card.key.clone())
        .h(px(28.))
        .px(px(12.))
        .rounded_full()
        .bg(theme.hover)
        .border_1()
        .border_color(if card.open { theme.border_strong } else { theme.border })
        .cursor_pointer()
        .hover(|btn| btn.border_color(theme.border_strong))
        .tooltip(tip(if card.open { "Hide summary" } else { "Show summary" }))
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(Icon::Spark.view(px(11.), theme.accent_text))
        .child(
            div()
                .typography(Type::Small)
                .text_color(if card.open { theme.text_soft } else { theme.text_dim })
                .child("Tidied up the conversation"),
        )
        .child(if card.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))
        .tag("chat-compaction-toggle");

    let body = card.open.then(|| {
        let copy = (!card.text.is_empty()).then(|| {
            CopyButton::new(format!("{}-copy", card.key), card.text.clone(), "Copy summary", "copy-compaction-summary")
        });
        div()
            .w_full()
            .rounded(px(14.))
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .px(px(16.))
            .py(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .typography(Type::Small)
                            .text_color(theme.text_dim)
                            .child("Earlier messages were folded into this summary so the chat can keep going."),
                    )
                    .children(copy),
            )
            .child(summary_content(&card.blocks, theme, handlers))
            .tag("chat-compaction-body")
    });

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .child(button)
        .children(body)
        .tag("chat-compaction")
        .into_any_element()
}

fn stat_pair(label: &'static str, value: String, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(label))
        .child(div().typography(Type::UiStrong).text_color(theme.text).child(value))
}

fn summary_content(blocks: &[markdown::Block], theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    div()
        .pt(px(12.))
        .border_t_1()
        .border_dashed()
        .border_color(theme.border)
        .child(if blocks.is_empty() {
            div()
                .typography(Type::Small)
                .italic()
                .text_color(theme.text_dim)
                .child("No summary text was recorded for this compaction.")
                .into_any_element()
        } else {
            crate::transcript::reply(blocks, theme, handlers).into_any_element()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_summary_with_body_sections_files_and_quoted_user_messages() {
        let summary = Summary {
            body: "## Goal\nFix compaction disclosure.\n\n## Next steps\nAdd gallery scenes.".into(),
            user_messages: vec!["First request".into(), "Line one\n\nLine two".into()],
            omitted_user_messages: 2,
            read_files: vec!["crates/wack-ui/src/work.rs".into()],
            modified_files: vec!["crates/wack-ui/src/transcript.rs".into()],
            sections: vec!["## Todo list\n- #1 completed: Inspect code".into()],
        };
        let md = format_summary(&summary);
        assert!(md.starts_with("## Goal\nFix compaction disclosure."));
        assert!(md.contains("## Todo list\n- #1 completed: Inspect code"));
        assert!(md.contains("## Files changed\n- `crates/wack-ui/src/transcript.rs`"));
        assert!(md.contains("## Files read\n- `crates/wack-ui/src/work.rs`"));
        assert!(md.contains("## The user's messages\n\n> First request\n\n*2 earlier messages left out to save space*\n\n> Line one\n>\n> Line two"));
        assert!(!md.contains("Written by the assistant"), "model-facing guard line stays out of the UI");
    }

    #[test]
    fn exact_and_header_token_counts_format_cleanly() {
        assert_eq!(format_exact(0), "0");
        assert_eq!(format_exact(812), "812");
        assert_eq!(format_exact(34_120), "34,120");
        assert_eq!(format_exact(1_250_000), "1,250,000");

        assert_eq!(header_counts(Some(34_120), Some(8_450)).as_deref(), Some("34.1k → ≈8.4k tokens"));
        assert_eq!(header_counts(Some(34_000), None).as_deref(), Some("34k before"));
        assert_eq!(header_counts(None, Some(8_000)).as_deref(), Some("≈8k tokens"));
        assert_eq!(header_counts(None, None), None);
    }

    #[test]
    fn markdown_parses_only_when_card_is_open() {
        let compaction = Compaction {
            summary: Summary { body: "## Goal\nKeep going.".into(), ..Summary::default() },
            tokens_before: Some(40_000),
            tokens_after: Some(6_000),
        };
        let mut cache = ReplyCache::default();
        let closed = from_compaction(&compaction, "compact-2", false, &mut cache);
        assert!(!closed.open);
        assert!(closed.blocks.is_empty(), "closed cards skip parsing");
        assert!(closed.text.contains("Keep going."));

        let opened = from_compaction(&compaction, "compact-2", true, &mut cache);
        assert!(opened.open);
        assert_eq!(opened.blocks.len(), 2, "heading + paragraph");
    }
}
