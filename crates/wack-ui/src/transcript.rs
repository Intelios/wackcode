//! The transcript: one chat read as a document.
//!
//! - The user's message is a bubble on the right, its tail corner tucked in.
//! - The agent's work opens with a chip ("Worked for 19s", the duck paddling while it works), a
//!   dotted rule and a count of what it did. Opened, it is a lit log of compact rows strung on a
//!   rail in the accent's sweep: a round mark lit in the accent (a command that passed in the
//!   success colour, a failure in danger's, a step under way breathing), the verb, what it acted
//!   on (in mono for paths and commands), its meta. A row with more to show (a diff, a command's
//!   output, a long thought) opens in place.
//! - Diffs, output and code are sheets: rounded, with a head band naming what they hold (and a
//!   copy button on code) over the code's own ground.
//! - The reply is running text in the UI face, with code in mono.
//!
//! Everything sits on the stage's glass, so grounds are the theme's washes (`hover`, `border`),
//! never solid fills that would cut a hole in the picture.
//!
//! Draws [`Item`]s, one at a time, so the chat can hand them to GPUI's virtual list. Building
//! items from a conversation is `work.rs`; parsing replies is `markdown.rs`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontStyle, FontWeight, Hsla, InteractiveText, SharedString, StrikethroughStyle,
    StyledText, TextRun, UnderlineStyle, Window, div, font, prelude::*, px, relative,
};

use crate::{
    icon::Icon,
    markdown::{self, Span},
    tags::Tag,
    theme::Theme,
    typography::{MONO_FONT, Type, Typography, UI_FONT},
};

#[derive(Clone, Debug)]
pub enum Item {
    /// What the user sent. A slash command shows as `command`, with `text` (the prompt the model
    /// got) behind its name. `suggestion` captions a message that started a suggested task.
    /// `pictures` are the ones the user attached, under the bubble.
    Prompt {
        text: SharedString,
        steered: bool,
        command: Option<CommandShown>,
        suggestion: Option<SharedString>,
        pictures: Vec<crate::pictures::Picture>,
        /// Long pastes sent with it: each file's name and line count.
        files: Vec<(SharedString, usize)>,
    },
    Work(Work),
    /// A reply's prose. `live`: still streaming in, so its newest words are fresh ink.
    Reply { blocks: Rc<Vec<markdown::Block>>, live: bool },
    /// The pictures a reply named (`![…](img:…)`), under it, and the references nothing answered.
    ReplyPictures { key: SharedString, pictures: Vec<crate::pictures::Picture>, missing: Vec<SharedString> },
    /// Facts about a finished turn: time, tokens.
    Footer(SharedString),
    Notice { text: SharedString, failed: bool },
    /// A background task reported while the chat was idle: one log row on its own.
    News(WorkRow),
    Compacted,
    /// A proposed plan: the transcript's hero card.
    Plan(crate::plan_card::PlanCard),
    /// The follow-ups a settled turn filed, under its answer.
    Suggestions(crate::suggestion::Tray),
    /// What a turn drew on a canvas, under its log.
    Canvas(crate::canvas_card::CanvasCard),
    // The Chat area's transcript (`chat_transcript`): a chat read as a messenger reads.
    /// A quiet time divider, by when the conversation resumed: its words are made when drawn.
    ChatDivider(u64),
    /// The duck and the agent's name over a reply.
    ChatSpeaker { live: bool },
    /// Some of a reply's prose; `copy` holds the words of a settled reply's last bubble.
    ChatBubble { blocks: Rc<Vec<markdown::Block>>, copy: Option<SharedString> },
    /// "Thought it over", folded.
    ChatThought { key: SharedString, text: SharedString, open: bool },
    /// What the agent did, as chips.
    ChatChips(Vec<crate::chat_transcript::Chip>),
    /// What the duck is up to while it works.
    ChatTyping(crate::chat_transcript::Mood),
    /// The files a settled reply wrote in the scratchpad, which show in the Finder.
    ChatFiles(Vec<crate::chat_transcript::ScratchFile>),
    /// The conversation was compacted here.
    ChatTidied,
}

/// A slash command as the user typed it.
#[derive(Clone, Debug)]
pub struct CommandShown {
    /// `/pdf`.
    pub name: SharedString,
    pub args: SharedString,
    /// Toggles the prompt sent to the agent open beneath the bubble.
    pub key: SharedString,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub struct Work {
    pub key: SharedString,
    /// "Worked for 19s", or the elapsed time while `live`.
    pub summary: SharedString,
    /// What it did, counted (`work::tally`): beside the chip, past its dotted rule.
    pub tally: SharedString,
    pub live: bool,
    /// The model is reasoning now: its last row is a thought still coming, before any reply.
    pub thinking: bool,
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
    /// A sub-agent's call: drawn as a row of its own, which opens the agent's pane.
    pub agent: Option<AgentLine>,
    /// What the call returned to look at (a screenshot), under the row.
    pub pictures: Vec<crate::pictures::Picture>,
}

/// A sub-agent as its row in the work log shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentLine {
    /// `a1`; empty while the call is still being written.
    pub id: SharedString,
    /// "Explore", or "Copy" for one started from the conversation.
    pub role: SharedString,
    /// What it was asked to do, in the parent's few words.
    pub task: SharedString,
    pub state: AgentState,
    /// Its latest step and its counts: "Reading theme.rs · 12 tools · 31k tokens · 0:48".
    pub words: SharedString,
    /// Rounds after the first: "round 2".
    pub round: u32,
    /// In a Team, which model it runs on: the lead's or the crew's. `None` outside a Team.
    pub tier: Option<wack_core::agents::Tier>,
    /// The files its brief gave it, short: "retry.rs, http.rs +2". Empty when none were named.
    pub files: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    Queued,
    Working,
    /// Working, and the parent carried on: it reports later.
    Background,
    Done,
    Failed,
    Stopped,
    Interrupted,
}

impl AgentState {
    pub fn working(self) -> bool {
        matches!(self, AgentState::Queued | AgentState::Working | AgentState::Background)
    }
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
    /// Something that ended well, in the success colour: a background build that passed.
    Success(SharedString),
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
    /// Reduce Motion is on: what would loop holds still.
    pub still: bool,
}

impl Handlers {
    /// Clicks that do nothing: for scenes.
    pub fn inert() -> Self {
        Self { toggle: Rc::new(|_, _, _| {}), open_link: Rc::new(|_, _, _| {}), still: false }
    }
}

/// A slash command's bubble: its name in the accent (a button that shows the prompt the agent
/// got), then what was typed after it.
fn command_bubble(bubble: gpui::Div, command: &CommandShown, theme: &Theme, handlers: &Handlers) -> gpui::Div {
    let toggle = handlers.toggle.clone();
    let key = command.key.clone();
    bubble.flex().flex_wrap().items_baseline().gap(px(6.)).child(
        div()
            .id(ElementId::Name(format!("{}-name", command.key).into()))
            .flex_none()
            .typography(Type::Body)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.accent_text)
            .hover(|name| name.underline())
            .cursor_pointer()
            .tooltip(crate::tooltip::tip(if command.open { "Hide the prompt sent to the agent" } else { "Show the prompt sent to the agent" }))
            .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
            .child(command.name.clone())
            .tag("command-name"),
    )
    .when(!command.args.is_empty(), |bubble| {
        bubble.child(div().flex_none().text_color(theme.text_faint).child("·")).child(command.args.clone())
    })
}

/// The prompt a slash command sent, opened beneath its bubble.
fn sent_prompt(key: &SharedString, text: &SharedString, theme: &Theme) -> impl IntoElement {
    let copied = text.clone();
    div()
        .max_w(relative(BUBBLE_SHARE))
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(12.))
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .items_center()
                .child(div().flex_1().typography(Type::Label).text_color(theme.text_dim).child("Prompt sent to the agent"))
                .child(crate::copy::CopyButton::new(format!("{key}-prompt"), copied, "Copy prompt", "copy-prompt")),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{key}-text").into()))
                .max_h(px(SENT_PROMPT_MAX))
                .overflow_y_scroll()
                .typography(Type::Mono)
                .text_color(theme.text_soft)
                .child(text.clone()),
        )
        .tag("command-prompt")
}

/// A long skill scrolls inside its box.
const SENT_PROMPT_MAX: f32 = 320.;

/// Space above `item`, given the item before it.
fn gap_above(item: &Item, previous: Option<&Item>) -> f32 {
    if let Some(gap) = crate::chat_transcript::gap_above(item, previous) {
        return gap;
    }
    match (previous, item) {
        (None, _) => 0.,
        (_, Item::Prompt { steered: true, .. }) => 24.,
        (_, Item::Prompt { .. }) => 44.,
        (_, Item::Footer(_)) => 12.,
        (_, Item::Suggestions(_)) => 14.,
        (_, Item::Canvas(_)) => 12.,
        (Some(Item::Prompt { .. }), _) => 16.,
        _ => 18.,
    }
}

/// One transcript item, with the space it keeps from the one before.
pub fn item(item: &Item, previous: Option<&Item>, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if let Some(body) = crate::chat_transcript::item(item, theme, handlers, handlers.still) {
        return div().pt(px(gap_above(item, previous))).child(body).into_any_element();
    }
    let body = match item {
        Item::Prompt { text, steered, command, suggestion, pictures, files } => {
            let bubble = div()
                .max_w(relative(BUBBLE_SHARE))
                .px(px(14.))
                .py(px(10.))
                .rounded_tl(px(BUBBLE_RADIUS))
                .rounded_tr(px(BUBBLE_RADIUS))
                .rounded_bl(px(BUBBLE_RADIUS))
                .rounded_br(px(6.))
                .bg(theme.hover)
                .border_1()
                .border_color(theme.border)
                .typography(Type::Body)
                .text_color(theme.text);
            let bubble = match command {
                None => bubble.child(text.clone()).into_any_element(),
                Some(command) => command_bubble(bubble, command, theme, handlers).into_any_element(),
            };
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(4.))
                .when(*steered, |prompt| {
                    prompt.child(div().pr(px(6.)).typography(Type::Small).text_color(theme.text_faint).child("Sent while working"))
                })
                .when(!pictures.is_empty(), |prompt| {
                    prompt.child(crate::pictures::Strip { pictures: pictures.clone(), key: format!("prompt-{}", pictures[0].key).into(), align_end: true })
                })
                .when(!files.is_empty(), |prompt| {
                    prompt.child(div().flex().flex_wrap().justify_end().gap(px(6.)).children(files.iter().map(|(name, lines)| pasted_chip(name, *lines, theme))))
                })
                .when(!text.is_empty() || command.is_some(), |prompt| prompt.child(bubble))
                .when_some(suggestion.as_ref(), |prompt, caption| {
                    prompt.child(
                        div()
                            .pr(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .typography(Type::Small)
                            .text_color(theme.text_dim)
                            .child(Icon::Bulb.view(px(11.), theme.accent_text))
                            .child(caption.clone()),
                    )
                })
                .when_some(command.as_ref().filter(|command| command.open), |prompt, command| {
                    prompt.child(sent_prompt(&command.key, text, theme))
                })
                .into_any_element()
        }
        Item::Work(work) => work_log(work, theme, handlers).into_any_element(),
        Item::Reply { blocks, live } => reply_inked(blocks, *live && !handlers.still, theme, handlers).into_any_element(),
        Item::ReplyPictures { key, pictures, missing } => div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(!pictures.is_empty(), |column| column.child(crate::pictures::Strip { pictures: pictures.clone(), key: key.clone(), align_end: false }))
            .children(missing.iter().map(|reference| crate::pictures::missing(reference, theme)))
            .into_any_element(),
        Item::Plan(card) => crate::plan_card::plan_card(card, theme, handlers).into_any_element(),
        Item::Suggestions(tray) => crate::suggestion::tray(tray, theme, handlers).into_any_element(),
        Item::Canvas(card) => crate::canvas_card::view(card, theme).into_any_element(),
        Item::Footer(text) => div().typography(Type::Small).text_color(theme.text_faint).child(text.clone()).into_any_element(),
        Item::Notice { text, failed } => div()
            .typography(Type::Ui)
            .text_color(if *failed { theme.danger } else { theme.text_dim })
            .child(text.clone())
            .into_any_element(),
        // Lined up with the work log's marks, which hang on its rail.
        Item::News(row) => div().pl(px(RAIL - ICON / 2.)).child(log_row(row, false, false, theme, handlers)).into_any_element(),
        // Drawn above by `chat_transcript::item`.
        Item::ChatDivider(_)
        | Item::ChatSpeaker { .. }
        | Item::ChatBubble { .. }
        | Item::ChatThought { .. }
        | Item::ChatChips(_)
        | Item::ChatTyping(_)
        | Item::ChatFiles(_)
        | Item::ChatTidied => div().into_any_element(),
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

/// The most of the column a prompt's bubble takes, and its corners.
const BUBBLE_SHARE: f32 = 0.78;
const BUBBLE_RADIUS: f32 = 16.;
/// A log row's round icon, and the space after it and after the verb.
const ICON: f32 = 22.;
const ROW_GAP: f32 = 10.;
const VERB_WIDTH: f32 = 62.;
/// Where a row's sheet starts (past its icon), and where its words start (past the verb too).
const SHEET_INDENT: f32 = ICON + ROW_GAP;
const WORDS_INDENT: f32 = ICON + ROW_GAP + VERB_WIDTH + ROW_GAP;
/// The log's rail drops from the middle of the chip's duck: the chip's padding and edge, then half
/// the duck. The rows' marks are strung on it.
const RAIL: f32 = 8. + 1. + 7.;
/// The middle of a row's mark, down from the row's top: its line is 28 points.
const MARK_MIDDLE: f32 = 14.;

fn work_log(work: &Work, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let toggle = handlers.toggle.clone();
    let key = work.key.clone();
    let chip = div()
        .id(work.key.clone())
        .flex_none()
        .h(px(26.))
        .pl(px(8.))
        .pr(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(theme.hover)
        .border_1()
        .border_color(theme.border)
        .when(!work.live, |chip| {
            chip.cursor_pointer()
                .hover(|chip| chip.border_color(theme.border_strong))
                .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
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
                // While it works, a light runs along its words.
                .child(if work.live {
                    let id = SharedString::from(format!("{}-shine", work.key));
                    crate::motion::shining_text(id, work.summary.clone(), theme.accent_text, theme.text, handlers.still)
                } else {
                    work.summary.clone().into_any_element()
                }),
        )
        .when(!work.live, |chip| {
            chip.child(if work.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint))
        });
    let header = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(chip)
        .child(div().flex_1().h(px(1.)).border_t_1().border_dashed().border_color(theme.border_strong))
        .when(!work.tally.is_empty(), |header| {
            header.child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(work.tally.clone()))
        });

    // The rail runs from under the chip to the last row's mark, measured as the rows are laid out
    // (prepaint is over before anything paints).
    let end = Rc::new(std::cell::Cell::new(None::<gpui::Pixels>));
    let (top, bottom) = match theme.accent_sweep {
        Some(stops) => (stops[0], stops[2]),
        None => (theme.accent, theme.accent),
    };
    let (top, bottom) = (Hsla { a: RAIL_ALPHA, ..top }, Hsla { a: RAIL_ALPHA, ..bottom });
    let rail = {
        let end = end.clone();
        gpui::canvas(
            |_, _, _| (),
            move |area, _, window, _| {
                let Some(end) = end.get() else { return };
                let start = area.origin.y + px(2.);
                if end <= start {
                    return;
                }
                let line = gpui::Bounds::new(gpui::point(area.origin.x + px(ICON / 2. - 1.), start), gpui::size(px(2.), end - start));
                let paint = gpui::linear_gradient(180., gpui::linear_color_stop(top, 0.), gpui::linear_color_stop(bottom, 1.));
                window.paint_quad(gpui::fill(line, paint).corner_radii(px(1.)));
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    let mut rows = div()
        .relative()
        .ml(px(RAIL - ICON / 2.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(4.))
        .on_children_prepainted(move |children, _, _| end.set(children.last().map(|row| row.origin.y + px(MARK_MIDDLE))))
        .child(rail);
    let last = work.rows.len().saturating_sub(1);
    for (index, row) in work.rows.iter().enumerate() {
        rows = rows.child(log_row(row, work.live, work.thinking && index == last, theme, handlers));
    }

    div().flex().flex_col().gap(px(6.)).child(header).when(work.open && !work.rows.is_empty(), |log| log.child(rows))
}

/// How strongly the rail shows behind the marks.
const RAIL_ALPHA: f32 = 0.6;

/// A row's round mark, lit: its icon (by its verb) on the glass's tone, ringed and glowing in the
/// accent, or in success's colour for a command that passed and danger's for a step that failed.
/// A step under way breathes; an event (a retry, a compaction) is a small bead on the rail.
fn row_mark(row: &WorkRow, thinking: bool, theme: &Theme, still: bool) -> AnyElement {
    let Some(icon) = row_icon(&row.verb) else {
        return div()
            .flex_none()
            .size(px(ICON))
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(7.)).rounded_full().bg(theme.glass_tone).border_1().border_color(theme.accent_line))
            .into_any_element();
    };
    let (ink, ring, glow) = match &row.meta {
        Meta::Failed(_) => (theme.danger, Hsla { a: 0.6, ..theme.danger }, theme.danger),
        Meta::Running => (theme.accent_text, theme.accent_line, theme.accent),
        _ if matches!(row.verb.as_ref(), "Ran" | "Finished") => (theme.success, Hsla { a: 0.6, ..theme.success }, theme.success),
        _ => (theme.accent_text, theme.accent_line, theme.accent),
    };
    let mark = div()
        .flex_none()
        .size(px(ICON))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.glass_tone)
        .border_1()
        .border_color(ring)
        .shadow(vec![gpui::BoxShadow {
            color: Hsla { a: 0.3, ..glow },
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(10.),
            spread_radius: px(-2.),
            inset: false,
        }])
        .child(icon.view(px(12.), ink));
    match row.meta {
        _ if thinking => crate::motion::breathing(SharedString::from(format!("{}-breath", row.key)), mark, still),
        Meta::Running => crate::motion::breathing(SharedString::from(format!("{}-breath", row.key)), mark, still),
        _ => mark.into_any_element(),
    }
}

/// A row's icon, by its verb; none for an event.
fn row_icon(verb: &str) -> Option<Icon> {
    match verb {
        "Read" => Some(Icon::File),
        "Searched" | "Found" | "Looked for" => Some(Icon::Search),
        "Listed" => Some(Icon::Folder),
        "Edited" | "Wrote" => Some(Icon::Pencil),
        "Ran" | "Watching" | "Watch" | "Waited" | "Finished" | "Failed" | "Ended" | "Lost" => Some(Icon::Terminal),
        "Stopped" => Some(Icon::Stop),
        "Thought" => Some(Icon::Brain),
        "Asked" => Some(Icon::Question),
        "Used skill" => Some(Icon::Book),
        "Saved memory" | "Updated memory" | "Recalled memory" | "Forgot memory" => Some(Icon::Bookmark),
        "Reported" => Some(Icon::Duck),
        verb if crate::work::CANVAS_VERBS.contains(&verb) => Some(Icon::Board),
        verb if crate::work::BROWSER_VERBS.contains(&verb) => Some(Icon::Globe),
        verb if crate::work::COMPUTER_VERBS.contains(&verb) => Some(Icon::Cursor),
        "" => None,
        _ => Some(Icon::Hammer),
    }
}

/// `live`: the row is in the log of a run still going, so its mark lands as it arrives.
/// `thinking`: it is the thought the model is having now, so it reads "Thinking", a light running
/// along the word, and its mark breathes.
fn log_row(row: &WorkRow, live: bool, thinking: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if let Some(agent) = &row.agent {
        return agent_row(&row.key, agent, theme);
    }
    if let Target::Note(text) = &row.target {
        return div()
            .py(px(4.))
            .pl(px(SHEET_INDENT))
            .typography(Type::Ui)
            .text_color(theme.text_soft)
            .child(text.clone())
            .into_any_element();
    }
    let target = match &row.target {
        Target::Code(text) => div().typography(Type::Mono).text_color(theme.text_body).child(text.clone()),
        Target::Words(text) | Target::Note(text) => {
            div().typography(Type::Ui).italic().text_color(theme.text_dim).child(text.clone())
        }
    };
    let meta = match &row.meta {
        Meta::None => div(),
        Meta::Note(text) => div().text_color(theme.text_faint).child(text.clone()),
        Meta::Success(text) => div().text_color(theme.success).child(text.clone()),
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
                .h(px(28.))
                .pr(px(10.))
                .flex()
                .items_center()
                .gap(px(ROW_GAP))
                .rounded_full()
                .when(openable, |line| {
                    line.cursor_pointer()
                        .hover(move |line| line.bg(hover))
                        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
                })
                .child({
                    let mark = div().flex_none().child(row_mark(row, thinking, theme, handlers.still));
                    if live {
                        crate::motion::land(SharedString::from(format!("{}-land", row.key)), mark, handlers.still)
                    } else {
                        mark.into_any_element()
                    }
                })
                .when(!row.verb.is_empty(), |line| {
                    let verb = if thinking {
                        let id = SharedString::from(format!("{}-thinking", row.key));
                        crate::motion::shining_text(id, "Thinking".into(), theme.text_dim, theme.accent_text, handlers.still)
                    } else {
                        row.verb.clone().into_any_element()
                    };
                    line.child(div().flex_none().w(px(VERB_WIDTH)).typography(Type::Ui).text_color(theme.text_dim).child(verb))
                })
                .child(div().flex_1().min_w_0().truncate().child(target))
                .child(div().flex_none().typography(Type::Small).child(meta)),
        )
        .when(!row.pictures.is_empty(), |line| {
            line.child(div().pl(px(SHEET_INDENT)).pt(px(2.)).pb(px(6.)).child(crate::pictures::Strip {
                pictures: row.pictures.clone(),
                key: format!("{}-shots", row.key).into(),
                align_end: false,
            }))
        })
        .when_some(row.detail.as_ref().filter(|_| row.open), |line, detail| {
            line.child(div().pt(px(4.)).pb(px(8.)).child(detail_view(detail, &row.key, theme)))
        })
        .into_any_element()
}

/// A sub-agent's row: its duck (paddling while it works), its role and task, how it stands, and a
/// line of what it is doing or did. The whole row opens its work in the Agents pane.
pub fn agent_row(key: &SharedString, agent: &AgentLine, theme: &Theme) -> AnyElement {
    // A queued agent hasn't started: its duck waits, dimmed.
    let working = matches!(agent.state, AgentState::Working | AgentState::Background);
    let (word, colour) = agent_state_word(agent.state, theme);
    let duck = div()
        .flex_none()
        .size(px(ICON))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.glass_tone)
        .border_1()
        .border_color(if working { theme.accent_line } else { theme.border_strong })
        .child(if working {
            crate::motion::paddling(div().child(Icon::Duck.view(px(12.), theme.accent_text))).into_any_element()
        } else {
            Icon::Duck.view(px(12.), theme.text_dim).into_any_element()
        });
    let id = agent.id.clone();
    let hover = theme.hover;
    div()
        .id(key.clone())
        .py(px(5.))
        .pr(px(10.))
        .rounded(px(14.))
        .flex()
        .items_start()
        .gap(px(ROW_GAP))
        .when(!agent.id.is_empty(), |row| {
            row.cursor_pointer()
                .hover(move |row| row.bg(hover))
                .tooltip(crate::tooltip::tip("Show this agent's work"))
                .on_click(move |_, window, cx| window.dispatch_action(Box::new(crate::actions::ShowAgent { id: id.clone() }), cx))
        })
        .child(duck)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .h(px(ICON))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_none().typography(Type::UiStrong).text_color(theme.text).child(agent.role.clone()))
                        .children(agent.tier.map(|tier| tier_chip(tier, theme)))
                        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_soft).child(agent.task.clone()))
                        .when(!agent.id.is_empty(), |line| {
                            line.child(div().flex_none().typography(Type::Small).font_family(MONO_FONT).text_color(theme.text_faint).child(agent.id.clone()))
                        })
                        .child(div().flex_none().typography(Type::Small).text_color(colour).child(word))
                        .when(!agent.id.is_empty(), |line| line.child(Icon::ChevronRight.view(px(11.), theme.text_faint))),
                )
                .when(!agent.words.is_empty(), |text| {
                    text.child(div().truncate().typography(Type::Small).text_color(theme.text_faint).child(agent.words.clone()))
                })
                .when(!agent.files.is_empty(), |text| {
                    text.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(Icon::File.view(px(10.), theme.text_faint))
                            .child(div().flex_1().min_w_0().truncate().typography(Type::Small).font_family(MONO_FONT).text_color(theme.text_dim).child(agent.files.clone()))
                            .tag("agent-files"),
                    )
                }),
        )
        .tag("agent-row")
        .into_any_element()
}

/// Which model a Team's worker runs on, as a small pill beside its role: the lead's in the
/// lead's words, the crew's quieter.
pub fn tier_chip(tier: wack_core::agents::Tier, theme: &Theme) -> impl IntoElement + use<> {
    use wack_core::agents::Tier;
    let (word, text, edge) = match tier {
        Tier::Lead => ("lead", theme.text_soft, theme.border_strong),
        Tier::Crew => ("crew", theme.text_dim, theme.border),
    };
    div()
        .flex_none()
        .h(px(17.))
        .px(px(7.))
        .rounded_full()
        .border_1()
        .border_color(edge)
        .flex()
        .items_center()
        .typography(Type::Small)
        .text_color(text)
        .child(word)
        .tag(match tier {
            Tier::Lead => "tier-lead",
            Tier::Crew => "tier-crew",
        })
}

/// How an agent stands, in a word, and its colour: working in the accent, the ends in their
/// fixed meanings.
pub fn agent_state_word(state: AgentState, theme: &Theme) -> (&'static str, Hsla) {
    match state {
        AgentState::Queued => ("Queued", theme.text_faint),
        AgentState::Working => ("Working", theme.accent_text),
        AgentState::Background => ("In background", theme.accent_text),
        AgentState::Done => ("Done", theme.success),
        AgentState::Failed => ("Failed", theme.danger),
        AgentState::Stopped => ("Stopped", theme.text_dim),
        AgentState::Interrupted => ("Interrupted", theme.warning),
    }
}

fn detail_view(detail: &Detail, key: &SharedString, theme: &Theme) -> AnyElement {
    match detail {
        Detail::Diff(diff) => div().pl(px(SHEET_INDENT)).child(diff_sheet(diff, key, theme)).into_any_element(),
        Detail::Output(text) => div()
            .pl(px(SHEET_INDENT))
            .child(
                sheet(theme).child(
                    div()
                        .id(SharedString::from(format!("{key}-output")))
                        .max_h(px(320.))
                        .overflow_y_scroll()
                        .px(px(14.))
                        .py(px(10.))
                        .typography(Type::Code)
                        .text_color(theme.text_soft)
                        .child(text.clone()),
                ),
            )
            .into_any_element(),
        Detail::Asked(asked) => div().pl(px(WORDS_INDENT)).py(px(4.)).child(crate::question::asked_view(asked, theme)).into_any_element(),
        Detail::Text(text) => div()
            .pl(px(WORDS_INDENT))
            .typography(Type::Ui)
            .italic()
            .text_color(theme.text_dim)
            .child(text.clone())
            .into_any_element(),
    }
}

/// A sheet: rounded, edged, on the code's ground.
fn sheet(theme: &Theme) -> gpui::Div {
    div().rounded(px(12.)).border_1().border_color(theme.border).bg(theme.code_ground).overflow_hidden().flex().flex_col()
}

/// A sheet's head band: what it holds, on a wash over the code's ground, a dotted rule beneath.
fn head_band(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .bg(theme.hover)
        .border_b_1()
        .border_dashed()
        .border_color(theme.border_strong)
}

fn diff_sheet(diff: &Diff, key: &SharedString, theme: &Theme) -> impl IntoElement {
    let (folder, name) = match diff.path.rsplit_once('/') {
        Some((folder, name)) => (Some(folder.to_string()), name.to_string()),
        None => (None, diff.path.to_string()),
    };
    let lines = div().flex().flex_col().py(px(6.)).children(diff.lines.iter().map(|line| diff_line(line, theme)));
    sheet(theme)
        .child(
            head_band(theme)
                .child(Icon::File.view(px(13.), theme.text_dim))
                .child(div().flex_none().typography(Type::Mono).text_color(theme.text).child(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .typography(Type::Small)
                        .font_family(crate::typography::MONO_FONT)
                        .text_color(theme.text_faint)
                        .children(folder),
                )
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

pub(crate) fn reply(blocks: &[markdown::Block], theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    reply_inked(blocks, false, theme, handlers)
}

/// A reply; `fresh`, while it streams in, its newest words are fresh ink (`freshen`).
fn reply_inked(blocks: &[markdown::Block], fresh: bool, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let mut column = div().flex().flex_col().gap(px(12.));
    for (index, block) in blocks.iter().enumerate() {
        let last = index + 1 == blocks.len();
        column = column.child(block_view(block, &format!("b{index}"), fresh && last, theme, handlers));
    }
    column
}

/// How many of a streaming reply's newest characters are still fresh ink.
const FRESH_INK: usize = 36;
/// How far the newest character leans to the accent, and how faint it is.
const FRESH_TINT: f32 = 0.55;
const FRESH_FADE: f32 = 0.5;

/// Splits the last [`FRESH_INK`] characters of `text` into runs of their own, each leaning to
/// `accent` and fading the newer it is, so words arrive faintly in the accent and settle into ink
/// as more come after them: version 2's fresh ink, by place rather than by time.
fn freshen(text: &str, runs: Vec<TextRun>, accent: Hsla) -> Vec<TextRun> {
    let count = text.chars().count();
    let fresh = FRESH_INK.min(count);
    if fresh == 0 {
        return runs;
    }
    let tail = text.char_indices().nth(count - fresh).map_or(text.len(), |(at, _)| at);
    let mut out = Vec::with_capacity(runs.len() + fresh);
    let mut at = 0;
    let mut from_end = fresh;
    for run in runs {
        let end = at + run.len;
        if end <= tail {
            at = end;
            out.push(run);
            continue;
        }
        if at < tail {
            out.push(TextRun { len: tail - at, ..run.clone() });
            at = tail;
        }
        for letter in text[at..end].chars() {
            from_end -= 1;
            let newness = 1. - from_end as f32 / fresh as f32;
            let leaned = run.color.blend(Hsla { a: FRESH_TINT * newness, ..accent });
            out.push(TextRun { len: letter.len_utf8(), color: Hsla { a: leaned.a * (1. - FRESH_FADE * newness), ..leaned }, ..run.clone() });
        }
        at = end;
    }
    out
}

/// `fresh`: the block is the newest of a streaming reply, so its tail is fresh ink.
fn block_view(block: &markdown::Block, key: &str, fresh: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
    use markdown::Block;
    match block {
        // A paragraph of nothing but pictures: they show under the reply.
        Block::Paragraph(spans) if spans.iter().all(|span| span.picture.is_some() || span.text.trim().is_empty()) => div().into_any_element(),
        Block::Paragraph(spans) => {
            div().typography(Type::Body).text_color(theme.text_body).child(rich(spans, key, fresh, theme, handlers)).into_any_element()
        }
        Block::Heading(level, spans) => div()
            .pt(px(6.))
            .typography(if *level <= 2 { Type::Heading } else { Type::UiStrong })
            .text_color(theme.text)
            .child(rich(spans, key, fresh, theme, handlers))
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
                    let newest = fresh && index + 1 == items.len() && inner + 1 == item.blocks.len();
                    body = body.child(block_view(block, &format!("{key}-{index}-{inner}"), newest, theme, handlers));
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
                quote = quote.child(block_view(block, &format!("{key}-q{index}"), fresh && index + 1 == blocks.len(), theme, handlers));
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
                            .child(rich(cell, &format!("{row_key}-{index}"), false, theme, handlers)),
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

/// Running text with bold, italic, inline code and links, wrapped as one run of text. `fresh`: its
/// tail is a streaming reply's newest words.
fn rich(spans: &[Span], key: &str, fresh: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
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
    let runs = if fresh { freshen(&text, runs, theme.accent_text) } else { runs };
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

/// A reply's code: a sheet whose head band names the language and copies the code.
fn code_block(language: &str, text: &str, key: &str, theme: &Theme) -> impl IntoElement {
    let copy = crate::copy::CopyButton::new(key.to_string(), text.to_string(), "Copy", "copy-code");
    sheet(theme)
        .child(
            head_band(theme)
                .child(
                    div()
                        .flex_1()
                        .typography(Type::Small)
                        .font_family(crate::typography::MONO_FONT)
                        .text_color(theme.text_faint)
                        .child(if language.is_empty() { "code".to_string() } else { language.to_string() }),
                )
                .child(copy),
        )
        .child(
            div()
                .id(SharedString::from(format!("{key}-code")))
                .overflow_x_scroll()
                .px(px(14.))
                .pt(px(10.))
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
        let handlers = Handlers { still: cx.reduce_motion(), ..Handlers::inert() };
        let mut column = div().w_full().flex().flex_col();
        for (index, current) in self.items.iter().enumerate() {
            let previous = index.checked_sub(1).map(|index| &self.items[index]);
            column = column.child(item(current, previous, &theme, &handlers));
        }
        column
    }
}

/// A long paste that went with a message, as its bubble shows it.
fn pasted_chip(name: &SharedString, lines: usize, theme: &Theme) -> impl IntoElement {
    div()
        .h(px(34.))
        .px(px(12.))
        .rounded(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .bg(theme.hover)
        .border_1()
        .border_color(theme.border)
        .child(Icon::File.view(px(13.), theme.text_dim))
        .child(div().typography(Type::Mono).text_color(theme.text_soft).child(name.clone()))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child(if lines == 1 { "1 line".to_string() } else { format!("{lines} lines") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(len: usize, color: Hsla) -> TextRun {
        TextRun { len, font: font(UI_FONT), color, background_color: None, underline: None, strikethrough: None }
    }

    #[test]
    fn fresh_ink_is_the_newest_words_leaning_to_the_accent_and_faint() {
        let ink = Hsla { h: 0., s: 0., l: 0.9, a: 1. };
        let accent = Hsla { h: 0.9, s: 0.8, l: 0.7, a: 1. };
        let text = "Settled words come first, then the newest ones arrive here at the end.";
        let runs = freshen(text, vec![run(10, ink), run(text.len() - 10, ink)], accent);
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), text.len(), "every byte keeps a run");
        assert_eq!(runs.len(), 2 + FRESH_INK, "the settled part stays whole; each fresh letter is its own run");
        assert_eq!(runs[0].color, ink, "older words are ink");
        let (older, newest) = (runs[runs.len() - FRESH_INK].color, runs.last().unwrap().color);
        assert!(newest.a < older.a, "the newest letter is the faintest");
        assert!((newest.h - accent.h).abs() < (older.h - accent.h).abs() + 1e-3, "and leans furthest to the accent");
        // Shorter than the fresh tail: all of it is fresh, and nothing is lost.
        let short = freshen("Hi.", vec![run(3, ink)], accent);
        assert_eq!(short.len(), 3);
    }
}
