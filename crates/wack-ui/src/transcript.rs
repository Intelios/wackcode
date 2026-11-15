//! The transcript: one chat read as a document.
//!
//! - The user's message leads its turn: larger, heavier and brighter than the reply, with no
//!   bubble around it.
//! - The agent's work is a log, not a stack of cards: a verb column ("Read", "Ran", "Edited")
//!   and what it acted on, hung from one hairline. A row with more to show (a diff, a command's
//!   output, a long thought) opens in place.
//! - The reply is running text in the UI face, with code in mono.
//!
//! Draws [`Item`]s, one at a time, so the chat can hand them to GPUI's virtual list. Building
//! items from a conversation is `work.rs`; parsing replies is `markdown.rs`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, FontStyle, FontWeight, Hsla, InteractiveText, SharedString, StrikethroughStyle, StyledText,
    TextRun, UnderlineStyle, Window, div, font, prelude::*, px,
};

use crate::{
    icon::Icon,
    markdown::{self, Span},
    theme::Theme,
    typography::{MONO_FONT, Type, Typography, UI_FONT},
};

#[derive(Clone, Debug)]
pub enum Item {
    Prompt { text: SharedString, steered: bool },
    Work(Work),
    Reply(Rc<Vec<markdown::Block>>),
    /// Facts about a finished turn: time, tokens.
    Footer(SharedString),
    Notice { text: SharedString, failed: bool },
    Compacted,
}

#[derive(Clone, Debug)]
pub struct Work {
    pub key: SharedString,
    /// "Worked for 19s", or the elapsed time while `live`.
    pub summary: SharedString,
    pub live: bool,
    pub open: bool,
    pub rows: Vec<WorkRow>,
}

#[derive(Clone, Debug)]
pub struct WorkRow {
    pub key: SharedString,
    pub verb: SharedString,
    pub target: Target,
    pub meta: Meta,
    pub detail: Option<Detail>,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub enum Target {
    /// A path or a command, in mono.
    Code(SharedString),
    /// Words: a search, the first line of a thought.
    Words(SharedString),
    /// What the model said between steps, shown whole.
    Note(SharedString),
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
pub enum Detail {
    Diff(Diff),
    /// A command's output or a tool's error, in mono.
    Output(SharedString),
    /// A thought in full.
    Text(SharedString),
    /// What the agent asked, with the user's picks.
    Asked(Vec<crate::question::Asked>),
}

#[derive(Clone, Debug)]
pub struct Diff {
    pub path: SharedString,
    pub lines: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
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
    /// A break between separate changes, or a note that lines were left out.
    Gap,
}

pub type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// What clicking does, supplied by the chat that owns the transcript.
#[derive(Clone)]
pub struct Handlers {
    /// Opens or closes a work log or a row, by key.
    pub toggle: Handler<SharedString>,
    /// Opens a link from a reply.
    pub open_link: Handler<String>,
}

impl Handlers {
    /// Clicks that do nothing: for scenes.
    pub fn inert() -> Self {
        Self { toggle: Rc::new(|_, _, _| {}), open_link: Rc::new(|_, _, _| {}) }
    }
}

/// Space above `item`, given the item before it.
fn gap_above(item: &Item, previous: Option<&Item>) -> f32 {
    match (previous, item) {
        (None, _) => 0.,
        (_, Item::Prompt { steered: true, .. }) => 24.,
        (_, Item::Prompt { .. }) => 44.,
        (_, Item::Footer(_)) => 12.,
        (Some(Item::Prompt { .. }), _) => 16.,
        _ => 18.,
    }
}

/// One transcript item, with the space it keeps from the one before.
pub fn item(item: &Item, previous: Option<&Item>, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let body = match item {
        Item::Prompt { text, steered } => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(*steered, |prompt| {
                prompt.child(div().typography(Type::Small).text_color(theme.text_faint).child("Sent while working"))
            })
            .child(div().typography(Type::Prompt).text_color(theme.text).child(text.clone()))
            .into_any_element(),
        Item::Work(work) => work_log(work, theme, handlers).into_any_element(),
        Item::Reply(blocks) => reply(blocks, theme, handlers).into_any_element(),
        Item::Footer(text) => div().typography(Type::Small).text_color(theme.text_faint).child(text.clone()).into_any_element(),
        Item::Notice { text, failed } => div()
            .typography(Type::Ui)
            .text_color(if *failed { theme.danger } else { theme.text_dim })
            .child(text.clone())
            .into_any_element(),
        Item::Compacted => div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex_1().h(px(1.)).bg(theme.border))
            .child(div().typography(Type::Small).text_color(theme.text_faint).child("Earlier messages were summarised"))
            .child(div().flex_1().h(px(1.)).bg(theme.border))
            .into_any_element(),
    };
    div().pt(px(gap_above(item, previous))).child(body).into_any_element()
}

/// The left edge of the log rows, past the hairline.
const LOG_INDENT: f32 = 22.;
const VERB_WIDTH: f32 = 60.;

fn work_log(work: &Work, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let toggle = handlers.toggle.clone();
    let key = work.key.clone();
    let header = div()
        .id(work.key.clone())
        .h(px(24.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(!work.live, |header| {
            header.cursor_pointer().on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        })
        .child(if work.live {
            crate::motion::paddling(div().child(Icon::Duck.view(px(14.), theme.accent_text))).into_any_element()
        } else {
            Icon::Duck.view(px(14.), theme.text_dim).into_any_element()
        })
        .child(
            div()
                .typography(if work.live { Type::UiStrong } else { Type::Ui })
                .text_color(if work.live { theme.accent_text } else { theme.text_soft })
                .child(work.summary.clone()),
        )
        .when(!work.live, |header| {
            header.child(if work.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(12.), theme.text_faint))
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
    for row in &work.rows {
        rows = rows.child(log_row(row, theme, handlers));
    }

    div().flex().flex_col().gap(px(4.)).child(header).when(work.open && !work.rows.is_empty(), |log| log.child(rows))
}

fn log_row(row: &WorkRow, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if let Target::Note(text) = &row.target {
        return div().py(px(3.)).typography(Type::Ui).text_color(theme.text_soft).child(text.clone()).into_any_element();
    }
    let target = match &row.target {
        Target::Code(text) => div().typography(Type::Mono).text_color(theme.text_soft).child(text.clone()),
        Target::Words(text) | Target::Note(text) => {
            div().typography(Type::Ui).italic().text_color(theme.text_dim).child(text.clone())
        }
    };
    let meta = match &row.meta {
        Meta::None => div(),
        Meta::Note(text) => div().text_color(theme.text_faint).child(text.clone()),
        Meta::Failed(text) => div().text_color(theme.danger).child(text.clone()),
        Meta::Running => div().text_color(theme.accent_text).child("running"),
        Meta::Lines { added, removed } => div()
            .flex()
            .gap(px(6.))
            .child(div().text_color(theme.diff_added).child(format!("+{added}")))
            .child(div().text_color(theme.diff_removed).child(format!("−{removed}"))),
    };
    let openable = row.detail.is_some();
    let toggle = handlers.toggle.clone();
    let key = row.key.clone();
    let hover = theme.hover;
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .id(row.key.clone())
                .h(px(24.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(5.))
                .when(openable, |line| {
                    line.cursor_pointer()
                        .hover(move |line| line.bg(hover))
                        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
                })
                .child(div().flex_none().w(px(VERB_WIDTH)).typography(Type::Ui).text_color(theme.text_dim).child(row.verb.clone()))
                .child(div().flex_1().min_w_0().truncate().child(target))
                .child(div().flex_none().pr(px(4.)).typography(Type::Small).child(meta)),
        )
        .when_some(row.detail.as_ref().filter(|_| row.open), |line, detail| {
            line.child(div().pt(px(6.)).pb(px(8.)).child(detail_view(detail, &row.key, theme)))
        })
        .into_any_element()
}

fn detail_view(detail: &Detail, key: &SharedString, theme: &Theme) -> AnyElement {
    match detail {
        Detail::Diff(diff) => diff_card(diff, key, theme).into_any_element(),
        Detail::Output(text) => div()
            .id(SharedString::from(format!("{key}-output")))
            .max_h(px(320.))
            .overflow_y_scroll()
            .rounded(px(10.))
            .bg(theme.code_ground)
            .px(px(14.))
            .py(px(10.))
            .typography(Type::Code)
            .text_color(theme.text_soft)
            .child(text.clone())
            .into_any_element(),
        Detail::Asked(asked) => div().pl(px(VERB_WIDTH + 10.)).py(px(4.)).child(crate::question::asked_view(asked, theme)).into_any_element(),
        Detail::Text(text) => div()
            .pl(px(VERB_WIDTH + 10.))
            .typography(Type::Ui)
            .italic()
            .text_color(theme.text_dim)
            .child(text.clone())
            .into_any_element(),
    }
}

fn diff_card(diff: &Diff, key: &SharedString, theme: &Theme) -> impl IntoElement {
    let lines = div().flex().flex_col().py(px(6.)).children(diff.lines.iter().map(|line| diff_line(line, theme)));
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
                .child(div().flex_1().truncate().typography(Type::Mono).text_color(theme.text_soft).child(diff.path.clone()))
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .typography(Type::Small)
                        .child(div().text_color(theme.diff_added).child(format!("+{}", diff.added)))
                        .child(div().text_color(theme.diff_removed).child(format!("−{}", diff.removed))),
                ),
        )
        .child(div().id(SharedString::from(format!("{key}-diff"))).max_h(px(420.)).overflow_y_scroll().child(lines))
}

/// One line of a diff: number, sign and text, on the added or removed ground. Shared by edit
/// diffs in the transcript and the Changes pane.
pub fn diff_line(line: &DiffLine, theme: &Theme) -> AnyElement {
    if line.kind == LineKind::Gap {
        return div()
            .h(px(19.))
            .pl(px(54.))
            .flex()
            .items_center()
            .typography(Type::Small)
            .text_color(theme.text_faint)
            .child(if line.text.is_empty() { SharedString::from("⋯") } else { line.text.clone() })
            .into_any_element();
    }
    let (ground, sign, ink, sign_colour) = match line.kind {
        LineKind::Added => (Some(theme.diff_added_ground), "+", theme.text_body, theme.diff_added),
        LineKind::Removed => (Some(theme.diff_removed_ground), "−", theme.text_body, theme.diff_removed),
        _ => (None, " ", theme.text_soft, theme.text_faint),
    };
    div()
        .min_h(px(19.))
        .flex()
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
        .child(div().flex_1().min_w_0().pr(px(12.)).text_color(ink).child(line.text.clone()))
        .into_any_element()
}

fn reply(blocks: &[markdown::Block], theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let mut column = div().flex().flex_col().gap(px(12.));
    for (index, block) in blocks.iter().enumerate() {
        column = column.child(block_view(block, &format!("b{index}"), theme, handlers));
    }
    column
}

fn block_view(block: &markdown::Block, key: &str, theme: &Theme, handlers: &Handlers) -> AnyElement {
    use markdown::Block;
    match block {
        Block::Paragraph(spans) => {
            div().typography(Type::Body).text_color(theme.text_body).child(rich(spans, key, theme, handlers)).into_any_element()
        }
        Block::Heading(level, spans) => div()
            .pt(px(6.))
            .typography(if *level <= 2 { Type::Heading } else { Type::UiStrong })
            .text_color(theme.text)
            .child(rich(spans, key, theme, handlers))
            .into_any_element(),
        Block::List { start, items } => {
            let mut list = div().flex().flex_col().gap(px(6.));
            for (index, item) in items.iter().enumerate() {
                let marker = match (item.task, start) {
                    (Some(true), _) => "☑".to_string(),
                    (Some(false), _) => "☐".to_string(),
                    (None, Some(first)) => format!("{}.", first + index as u64),
                    (None, None) => "–".to_string(),
                };
                let mut body = div().flex_1().min_w_0().flex().flex_col().gap(px(6.));
                for (inner, block) in item.blocks.iter().enumerate() {
                    body = body.child(block_view(block, &format!("{key}-{index}-{inner}"), theme, handlers));
                }
                list = list.child(
                    div()
                        .flex()
                        .typography(Type::Body)
                        .child(
                            div()
                                .flex_none()
                                .w(px(if start.is_some() { 24. } else { 18. }))
                                .text_color(theme.text_faint)
                                .child(marker),
                        )
                        .child(body),
                );
            }
            list.into_any_element()
        }
        Block::Code { language, text } => code_block(language, text, key, theme).into_any_element(),
        Block::Quote(blocks) => {
            let mut quote = div().pl(px(14.)).border_l_2().border_color(theme.border_strong).flex().flex_col().gap(px(8.));
            for (index, block) in blocks.iter().enumerate() {
                quote = quote.child(block_view(block, &format!("{key}-q{index}"), theme, handlers));
            }
            quote.into_any_element()
        }
        Block::Rule => div().h(px(1.)).my(px(6.)).bg(theme.border).into_any_element(),
        Block::Table { header, rows } => {
            let row = |cells: &Vec<Vec<Span>>, row_key: String, strong: bool| {
                let mut line = div().flex().border_b_1().border_color(theme.border);
                for (index, cell) in cells.iter().enumerate() {
                    line = line.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px(px(8.))
                            .py(px(5.))
                            .typography(if strong { Type::UiStrong } else { Type::Ui })
                            .text_color(if strong { theme.text } else { theme.text_body })
                            .child(rich(cell, &format!("{row_key}-{index}"), theme, handlers)),
                    );
                }
                line
            };
            let mut table = div().flex().flex_col().child(row(header, format!("{key}-h"), true));
            for (index, cells) in rows.iter().enumerate() {
                table = table.child(row(cells, format!("{key}-r{index}"), false));
            }
            table.into_any_element()
        }
    }
}

/// Running text with bold, italic, inline code and links, wrapped as one run of text.
fn rich(spans: &[Span], key: &str, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut links = Vec::new();
    for span in spans {
        let start = text.len();
        text.push_str(&span.text);
        let mut face = font(if span.code { MONO_FONT } else { UI_FONT });
        face.weight = if span.bold { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        if span.italic {
            face.style = FontStyle::Italic;
        }
        let colour: Hsla = if span.link.is_some() {
            theme.accent_text
        } else if span.code || span.bold {
            theme.text
        } else {
            theme.text_body
        };
        runs.push(TextRun {
            len: span.text.len(),
            font: face,
            color: colour,
            background_color: None,
            underline: span.link.as_ref().map(|_| UnderlineStyle { color: Some(colour), thickness: px(1.), wavy: false }),
            strikethrough: span.strike.then_some(StrikethroughStyle { color: Some(colour), thickness: px(1.) }),
        });
        if let Some(link) = &span.link {
            links.push((start..text.len(), link.clone()));
        }
    }
    if text.is_empty() {
        return div().into_any_element();
    }
    let styled = StyledText::new(text).with_runs(runs);
    if links.is_empty() {
        return styled.into_any_element();
    }
    let open = handlers.open_link.clone();
    let ranges: Vec<_> = links.iter().map(|(range, _)| range.clone()).collect();
    InteractiveText::new(SharedString::from(format!("{key}-text")), styled)
        .on_click(ranges, move |index, window, cx| open(links[index].1.clone(), window, cx))
        .into_any_element()
}

fn code_block(language: &str, text: &str, key: &str, theme: &Theme) -> impl IntoElement {
    div()
        .rounded(px(10.))
        .bg(theme.code_ground)
        .flex()
        .flex_col()
        .when(!language.is_empty(), |block| {
            block.child(div().px(px(14.)).pt(px(9.)).typography(Type::Small).text_color(theme.text_faint).child(language.to_string()))
        })
        .child(
            div()
                .id(SharedString::from(format!("{key}-code")))
                .overflow_x_scroll()
                .px(px(14.))
                .pt(px(if language.is_empty() { 10. } else { 4. }))
                .pb(px(12.))
                .child(
                    div()
                        .typography(Type::Code)
                        .whitespace_nowrap()
                        .text_color(theme.text_body)
                        .child(crate::highlight::highlighted(language, text, theme)),
                ),
        )
}

/// Every item, stacked: for scenes and other places that draw a whole transcript at once.
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
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let handlers = Handlers::inert();
        let mut column = div().w_full().flex().flex_col();
        for (index, current) in self.items.iter().enumerate() {
            let previous = index.checked_sub(1).map(|index| &self.items[index]);
            column = column.child(item(current, previous, &theme, &handlers));
        }
        column
    }
}
