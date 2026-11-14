//! The transcript: one chat read as a document.
//!
//! - The user's message is set in the serif, like a heading for the turn that follows.
//! - The agent's work is a log, not a stack of cards: a verb column ("Read", "Ran", "Edited")
//!   and what it acted on, hung from one hairline. An edit can open into its diff in place.
//! - The reply is running text in the UI face, with code in mono.
//!
//! Renders from plain data ([`Item`]). Grouping tool calls into work, and parsing markdown into
//! [`Block`]s, happen before this view and are tested there.

use gpui::{
    App, FontStyle, FontWeight, Hsla, SharedString, StyledText, TextRun, Window, div, font,
    prelude::*, px,
};

use crate::{
    icon::Icon,
    theme::Theme,
    typography::{MONO_FONT, Type, Typography, UI_FONT},
};

#[derive(Clone, Debug)]
pub enum Item {
    Prompt(SharedString),
    Work(Work),
    Reply(Vec<Block>),
    /// Facts about a finished turn: time, model, tokens.
    Footer(SharedString),
}

#[derive(Clone, Debug)]
pub struct Work {
    /// "Worked for 19s", or the elapsed time while `live`.
    pub summary: SharedString,
    pub live: bool,
    pub open: bool,
    pub rows: Vec<WorkRow>,
}

#[derive(Clone, Debug)]
pub struct WorkRow {
    pub verb: &'static str,
    pub target: Target,
    pub meta: Meta,
    /// Shown under the row when it is open.
    pub diff: Option<Diff>,
}

#[derive(Clone, Debug)]
pub enum Target {
    /// A path or a command, in mono.
    Code(SharedString),
    /// Words: a search, the first line of a thought.
    Words(SharedString),
}

#[derive(Clone, Debug)]
pub enum Meta {
    None,
    Note(SharedString),
    Lines { added: u32, removed: u32 },
    Failed(SharedString),
    Running,
}

#[derive(Clone, Debug)]
pub struct Diff {
    pub path: SharedString,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub number: Option<u32>,
    pub text: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(SharedString),
    Bullets(Vec<Vec<Span>>),
    Code { language: SharedString, lines: Vec<Vec<(Token, &'static str)>> },
}

#[derive(Clone, Debug)]
pub enum Span {
    Text(&'static str),
    Strong(&'static str),
    Code(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Plain,
    Keyword,
    String,
    Number,
    Title,
    Meta,
    Comment,
}

#[derive(IntoElement)]
pub struct Transcript {
    items: Vec<Item>,
}

impl Transcript {
    pub fn new(items: Vec<Item>) -> Self {
        Self { items }
    }
}

impl RenderOnce for Transcript {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let mut column = div().w_full().flex().flex_col();
        let mut first = true;
        for item in self.items {
            let gap = match (&item, first) {
                (_, true) => 0.,
                (Item::Prompt(_), false) => 44.,
                (Item::Footer(_), false) => 14.,
                _ => 18.,
            };
            first = false;
            column = column.child(div().pt(px(gap)).child(match item {
                Item::Prompt(text) => prompt(text, &theme).into_any_element(),
                Item::Work(work) => work_log(work, &theme).into_any_element(),
                Item::Reply(blocks) => reply(blocks, &theme).into_any_element(),
                Item::Footer(text) => div()
                    .typography(Type::Small)
                    .text_color(theme.text_faint)
                    .child(text)
                    .into_any_element(),
            }));
        }
        column
    }
}

fn prompt(text: SharedString, theme: &Theme) -> impl IntoElement {
    div().typography(Type::Prompt).text_color(theme.text).child(text)
}

/// The left edge of the log rows, past the hairline.
const LOG_INDENT: f32 = 22.;
const VERB_WIDTH: f32 = 60.;

fn work_log(work: Work, theme: &Theme) -> impl IntoElement {
    let header = div()
        .h(px(24.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(Icon::Duck.view(
            px(14.),
            if work.live { theme.accent_text } else { theme.text_dim },
        ))
        .child(
            div()
                .typography(if work.live { Type::UiStrong } else { Type::Ui })
                .text_color(if work.live { theme.accent_text } else { theme.text_soft })
                .child(work.summary),
        )
        .when(!work.live, |header| {
            header.child(
                if work.open { Icon::ChevronDown } else { Icon::ChevronRight }
                    .view(px(12.), theme.text_faint),
            )
        });

    let mut rows = div()
        .ml(px(6.5))
        .pl(px(LOG_INDENT - 6.5))
        .border_l_1()
        .border_color(theme.border)
        .flex()
        .flex_col()
        .gap(px(1.))
        .py(px(4.));
    for row in work.rows {
        rows = rows.child(log_row(row, theme));
    }

    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(header)
        .when(work.open || work.live, |log| log.child(rows))
}

fn log_row(row: WorkRow, theme: &Theme) -> impl IntoElement {
    let target = match row.target {
        Target::Code(text) => div()
            .typography(Type::Mono)
            .text_color(theme.text_soft)
            .child(text),
        Target::Words(text) => div()
            .typography(Type::Ui)
            .italic()
            .text_color(theme.text_dim)
            .child(text),
    };
    let meta = match row.meta {
        Meta::None => div(),
        Meta::Note(text) => div().text_color(theme.text_faint).child(text),
        Meta::Failed(text) => div().text_color(theme.danger).child(text),
        Meta::Running => div().text_color(theme.accent_text).child("running"),
        Meta::Lines { added, removed } => div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.diff_added).child(format!("+{added}")))
            .child(div().text_color(theme.diff_removed).child(format!("−{removed}"))),
    };
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(24.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_none()
                        .w(px(VERB_WIDTH))
                        .typography(Type::Ui)
                        .text_color(theme.text_dim)
                        .child(row.verb),
                )
                .child(div().flex_1().min_w_0().truncate().child(target))
                .child(div().flex_none().typography(Type::Small).child(meta)),
        )
        .when_some(row.diff, |row, diff| {
            row.child(div().pt(px(6.)).pb(px(8.)).child(diff_card(diff, theme)))
        })
}

fn diff_card(diff: Diff, theme: &Theme) -> impl IntoElement {
    let added = diff.lines.iter().filter(|line| line.kind == LineKind::Added).count();
    let removed = diff.lines.iter().filter(|line| line.kind == LineKind::Removed).count();
    let mut lines = div().flex().flex_col().py(px(6.));
    for line in diff.lines {
        let (ground, sign, ink) = match line.kind {
            LineKind::Context => (None, " ", theme.text_soft),
            LineKind::Added => (Some(theme.diff_added_ground), "+", theme.text_body),
            LineKind::Removed => (Some(theme.diff_removed_ground), "−", theme.text_body),
        };
        let sign_colour = match line.kind {
            LineKind::Added => theme.diff_added,
            LineKind::Removed => theme.diff_removed,
            LineKind::Context => theme.text_faint,
        };
        lines = lines.child(
            div()
                .h(px(19.))
                .flex()
                .items_center()
                .typography(Type::Code)
                .when_some(ground, |row, ground| row.bg(ground))
                .child(
                    div()
                        .flex_none()
                        .w(px(40.))
                        .pr(px(10.))
                        .text_right()
                        .text_color(theme.text_faint)
                        .child(line.number.map(|n| n.to_string()).unwrap_or_default()),
                )
                .child(div().flex_none().w(px(14.)).text_color(sign_colour).child(sign))
                .child(div().flex_1().min_w_0().truncate().text_color(ink).child(line.text)),
        );
    }
    div()
        .rounded(px(10.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .overflow_hidden()
        .child(
            div()
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_b_1()
                .border_color(theme.border)
                .child(Icon::File.view(px(13.), theme.text_dim))
                .child(
                    div()
                        .flex_1()
                        .typography(Type::Mono)
                        .text_color(theme.text_soft)
                        .child(diff.path),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .child(div().text_color(theme.diff_added).child(format!("+{added}")))
                        .child(div().text_color(theme.diff_removed).child(format!("−{removed}"))),
                ),
        )
        .child(lines)
}

fn reply(blocks: Vec<Block>, theme: &Theme) -> impl IntoElement {
    let mut column = div().flex().flex_col().gap(px(12.));
    for block in blocks {
        column = column.child(match block {
            Block::Paragraph(spans) => div()
                .typography(Type::Body)
                .text_color(theme.text_body)
                .child(rich(&spans, theme))
                .into_any_element(),
            Block::Heading(text) => div()
                .pt(px(6.))
                .typography(Type::Heading)
                .text_color(theme.text)
                .child(text)
                .into_any_element(),
            Block::Bullets(items) => {
                let mut list = div().flex().flex_col().gap(px(6.));
                for spans in items {
                    list = list.child(
                        div()
                            .flex()
                            .typography(Type::Body)
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(18.))
                                    .text_color(theme.text_faint)
                                    .child("–"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(theme.text_body)
                                    .child(rich(&spans, theme)),
                            ),
                    );
                }
                list.into_any_element()
            }
            Block::Code { language, lines } => code_block(language, lines, theme).into_any_element(),
        });
    }
    column
}

/// Running text with bold and inline code, as one wrapped run of text.
fn rich(spans: &[Span], theme: &Theme) -> StyledText {
    let mut text = String::new();
    let mut runs = Vec::new();
    for span in spans {
        let (content, family, weight, colour, ground) = match span {
            Span::Text(content) => (*content, UI_FONT, FontWeight::NORMAL, theme.text_body, None),
            Span::Strong(content) => (*content, UI_FONT, FontWeight::SEMIBOLD, theme.text, None),
            // No ground behind inline code: a run cannot be padded, and a tight box reads as a
            // highlight. The mono face is the cue.
            Span::Code(content) => (*content, MONO_FONT, FontWeight::NORMAL, theme.text, None),
        };
        text.push_str(content);
        let mut face = font(family);
        face.weight = weight;
        runs.push(TextRun {
            len: content.len(),
            font: face,
            color: colour,
            background_color: ground,
            underline: None,
            strikethrough: None,
        });
    }
    StyledText::new(text).with_runs(runs)
}

fn code_block(
    language: SharedString,
    lines: Vec<Vec<(Token, &'static str)>>,
    theme: &Theme,
) -> impl IntoElement {
    let colour = |token: Token| -> Hsla {
        match token {
            Token::Plain => theme.text_body,
            Token::Keyword => theme.code_keyword,
            Token::String => theme.code_string,
            Token::Number => theme.code_number,
            Token::Title => theme.code_title,
            Token::Meta => theme.code_meta,
            Token::Comment => theme.code_comment,
        }
    };
    let mut body = div().flex().flex_col().px(px(14.)).pb(px(12.));
    for line in lines {
        let mut text = String::new();
        let mut runs = Vec::new();
        for (token, content) in line {
            text.push_str(content);
            let mut face = font(MONO_FONT);
            if token == Token::Comment {
                face.style = FontStyle::Italic;
            }
            runs.push(TextRun {
                len: content.len(),
                font: face,
                color: colour(token),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        if text.is_empty() {
            text.push(' ');
            runs.push(TextRun {
                len: 1,
                font: font(MONO_FONT),
                color: theme.text_body,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        body = body.child(
            div()
                .typography(Type::Code)
                .whitespace_nowrap()
                .child(StyledText::new(text).with_runs(runs)),
        );
    }
    div()
        .rounded(px(10.))
        .bg(theme.code_ground)
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(14.))
                .pt(px(9.))
                .pb(px(6.))
                .typography(Type::Small)
                .text_color(theme.text_faint)
                .child(language),
        )
        .child(body)
}
