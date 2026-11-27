//! The transcript: one chat read as a document, quietly (direction D, chosen with the owner on
//! 2026-11-27 after Claude Desktop and Antigravity).
//!
//! - Two inks: the model's words in the body ink; everything about the work in grey. Nothing
//!   about a step has a box: only the user's bubble and code, diffs and output have a ground.
//! - The user's message is a bubble on the right, evenly rounded. Under it, while the pointer is
//!   over it, Copy, Edit, Rewind and Retry (`rewind.rs`); right-clicking it offers the same.
//!   Being edited, it is a field in place of the bubble.
//! - A turn reads as the model wrote it: its words between steps are running text, never folded
//!   away, and each run of steps between them is a group. Settled, a group is one grey line of
//!   what it did, its chevron after the words ("Read 3 files and ran a command ›"); opened, its
//!   rows hang under it, indented: a faint icon, the verb in grey, what it acted on a shade
//!   lighter (a file's name, then its folder faint; a command in mono), then what came of it
//!   (+/− in the diff colours, a pass in success's, a failure in danger's). A row with more to
//!   show (a diff, a command's output) opens in place.
//! - Live, a group's line is a breathing dot and what it is doing now, shining; its newest rows
//!   come under it, older ones behind "N earlier steps", the step under way shining in the
//!   present tense. The accent is only ever on what is live and on actions.
//! - A thought is the duck and "Thought for 12.3s ›"; live, the duck paddles and "Thinking"
//!   shines, its clock counting. Opened, its words are plain grey text under the line in a body
//!   of capped height that scrolls inside, so a long thought never runs the page on.
//! - A turn the user has since answered folds all its work behind "Worked for 1m 12s ›" over
//!   its answer (`Item::Earlier`); the latest turn stays open.
//! - A call still being written says what is coming ("Writing the plan · 6.4k characters").
//! - Diffs, output and code are sheets: rounded, a hairline round the code's own ground. A
//!   reply's code adds a head naming its language, with a copy button.
//! - The reply is running text in the UI face, with code in mono on a faint wash.
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
        /// Which of the user's messages it is (from 0), as the session counts them.
        number: usize,
        text: SharedString,
        steered: bool,
        command: Option<CommandShown>,
        suggestion: Option<SharedString>,
        pictures: Vec<crate::pictures::Picture>,
        /// Attached text files sent with it, expandable to show the exact text the model got.
        files: Vec<AttachedFileShown>,
    },
    Work(Work),
    /// A turn the user has since answered, its work folded to one line ("Worked for 1m 12s")
    /// over its answer. Opened, its work follows the line as it was.
    Earlier { key: SharedString, label: SharedString, open: bool },
    /// A `git commit` call, lifted out of the work log as its own milestone row.
    Commit(crate::commit::CommitRow),
    /// A reply's prose. `live`: still streaming in, so its newest words are fresh ink.
    Reply { blocks: Rc<Vec<markdown::Block>>, live: bool },
    /// The pictures a reply named (`![…](img:…)`), under it, and the references nothing answered.
    ReplyPictures { key: SharedString, pictures: Vec<crate::pictures::Picture>, missing: Vec<SharedString> },
    /// Facts about a finished turn: time, tokens.
    Footer(SharedString),
    Notice { text: SharedString, failed: bool },
    /// A background task reported while the chat was idle: one log row on its own.
    News(WorkRow),
    /// A compaction boundary whose summary opens in place.
    Compacted(crate::compaction_card::CompactionCard),
    /// A proposed plan: the transcript's hero card.
    Plan(crate::plan_card::PlanCard),
    /// The files a settled turn edited or created, with inline diffs and a link to Changes.
    Changes(crate::changes_card::ChangesCard),
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
    /// The conversation was compacted here; clicking opens the summary.
    ChatTidied(crate::compaction_card::CompactionCard),
    /// A rewind just made, which Undo takes back until anything runs. `files`: it put files
    /// back too.
    Rewound { files: bool },
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

/// An attached text file on a user message, expandable to show the exact text the model received.
#[derive(Clone, Debug)]
pub struct AttachedFileShown {
    pub key: SharedString,
    pub name: SharedString,
    pub text: SharedString,
    pub lines: usize,
    pub open: bool,
}

/// One run of steps between the model's words: tool calls and the reasoning around them.
/// Settled, it folds to one sentence of what it did (`label`); live, it shows its newest rows as
/// they come, the older ones behind "N earlier steps".
#[derive(Clone, Debug)]
pub struct Work {
    pub key: SharedString,
    /// "Working · 8s" while `live`, "Worked for 19s" after: the composer's status reads it.
    pub summary: SharedString,
    /// What it did, counted (`work::tally`).
    pub tally: SharedString,
    /// What a settled group says folded: "Read 3 files, edited a file and ran 2 commands"
    /// (`work::sentence`).
    pub label: SharedString,
    pub live: bool,
    /// The model is reasoning now: its last row is a thought still coming, before any reply.
    pub thinking: bool,
    pub open: bool,
    pub rows: Vec<WorkRow>,
    /// While live, how many of the first rows are folded behind "N earlier steps".
    pub hidden: usize,
}

impl Work {
    /// A group of reasoning alone draws as its thoughts, with no sentence over them.
    pub fn only_thoughts(&self) -> bool {
        !self.rows.is_empty() && self.rows.iter().all(WorkRow::is_thought)
    }
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

impl WorkRow {
    pub fn is_thought(&self) -> bool {
        matches!(self.target, Target::Thought(_) | Target::Stream(_))
    }
}

#[derive(Clone, Debug)]
pub enum Target {
    /// A path or a command, in mono.
    Code(SharedString),
    /// Words: a search, what a call does.
    Words(SharedString),
    /// What the model said between steps, shown whole.
    Note(SharedString),
    /// A settled thought: its first line, after "Thought for 12s".
    Thought(SharedString),
    /// A thought still coming: the newest of it, flattened to one line that runs into a caret.
    Stream(SharedString),
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
    /// Running, for this long: "12s".
    Clock(SharedString),
    /// A thought under way since `started` (milliseconds since 1970): its clock counts tenths
    /// as it is drawn (`thinking_clock`), so it moves smoothly between the chat's rebuilds.
    /// `at` is when the row was built, the time a scene shows.
    Thinking { started: u64, at: u64 },
}

impl Meta {
    pub fn running(&self) -> bool {
        matches!(self, Meta::Running | Meta::Clock(_) | Meta::Thinking { .. })
    }
}

/// A live thought's time: now, in a live chat; when the row was built, in a scene.
fn thinking_clock(started: u64, at: u64, handlers: &Handlers) -> SharedString {
    let now = if handlers.clock { crate::chat::now_millis().max(at) } else { at };
    crate::work::thought_time(now.saturating_sub(started)).into()
}

#[derive(Clone, Debug)]
pub enum Detail {
    Diff(Diff),
    /// A command's output or a tool's error, in mono.
    Output(SharedString),
    /// A thought in full.
    Text(SharedString),
    /// A thought, in a body of capped height: the newest of it while `live` (pinned to its
    /// end), all of it once settled (scrolling inside the body).
    Thought { text: SharedString, live: bool },
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// What the user's messages offer, and the one being edited or asked about.
    pub prompts: crate::rewind::Prompts,
    /// A live chat: clocks read the time as they are drawn. A scene's keep the time its items
    /// were built with.
    pub clock: bool,
}

impl Handlers {
    /// Clicks that do nothing: for scenes.
    pub fn inert() -> Self {
        Self { toggle: Rc::new(|_, _, _| {}), open_link: Rc::new(|_, _, _| {}), still: false, prompts: Default::default(), clock: false }
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
        bubble.child(div().flex_none().text_color(theme.text_faint).child("·")).child(prompt_text(&command.args, theme))
    })
}

/// A user message's words, with path-like and quoted `@` mentions tinted in mono.
fn prompt_text(text: &SharedString, theme: &Theme) -> AnyElement {
    let segments = wack_core::files::split_mentions(text);
    if !segments.iter().any(|segment| segment.mention) {
        return text.clone().into_any_element();
    }
    let runs: Vec<TextRun> = segments
        .into_iter()
        .map(|segment| TextRun {
            len: segment.text.len(),
            font: font(if segment.mention { MONO_FONT } else { UI_FONT }),
            color: if segment.mention { theme.accent_text } else { theme.text },
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect();
    StyledText::new(text.clone()).with_runs(runs).into_any_element()
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
        (_, Item::Prompt { .. }) => 40.,
        (_, Item::Footer(_)) => 12.,
        (_, Item::Changes(_)) => 16.,
        (_, Item::Suggestions(_)) => 14.,
        (_, Item::Canvas(_)) => 12.,
        (Some(Item::Commit(_)), Item::Commit(_)) => 6.,
        (Some(Item::Work(_)), Item::Commit(_)) => 8.,
        (Some(Item::Commit(_)), Item::Work(_)) => 8.,
        (Some(Item::Work(_)), Item::Work(_)) => 6.,
        (Some(Item::Earlier { .. }), Item::Reply { .. }) => 10.,
        (Some(Item::Earlier { .. }), _) => 6.,
        (Some(Item::Work(_)), Item::Compacted(_)) => 12.,
        (Some(Item::Compacted(_)), Item::Work(_)) => 12.,
        // Under the row of actions every message keeps.
        (Some(Item::Prompt { .. }), _) => 2.,
        (_, Item::Rewound { .. }) => 20.,
        // A line about the work sits close to the words around it.
        (Some(Item::Reply { .. }), Item::Work(_)) => 12.,
        (Some(Item::Work(_)), Item::Reply { .. }) => 10.,
        _ => 16.,
    }
}

/// One transcript item, with the space it keeps from the one before.
pub fn item(item: &Item, previous: Option<&Item>, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if let Some(body) = crate::chat_transcript::item(item, theme, handlers, handlers.still) {
        return div().pt(px(gap_above(item, previous))).child(body).into_any_element();
    }
    let body = match item {
        Item::Prompt { number, text, steered, command, suggestion, pictures, files } => {
            let number = *number;
            let editing = handlers.prompts.editing.as_ref().filter(|(editing, _)| *editing == number).map(|(_, field)| field);
            let card = handlers.prompts.card.as_ref().filter(|card| card.number == number);
            let group = SharedString::from(format!("prompt-{number}"));
            let copy: SharedString = match command {
                Some(command) if command.args.is_empty() => command.name.clone(),
                Some(command) => format!("{} {}", command.name, command.args).into(),
                None => text.clone(),
            };
            let bubble = div()
                .max_w(relative(BUBBLE_SHARE))
                .px(px(16.))
                .py(px(10.))
                .rounded(px(BUBBLE_RADIUS))
                .bg(theme.hover)
                .border_1()
                .border_color(theme.border)
                .typography(Type::Body)
                .text_color(theme.text);
            let bubble = match (editing, command) {
                (Some(field), _) => crate::rewind::editor(field, BUBBLE_SHARE, theme),
                (None, None) => bubble.child(prompt_text(text, theme)).into_any_element(),
                (None, Some(command)) => command_bubble(bubble, command, theme, handlers).into_any_element(),
            };
            div()
                .id(ElementId::NamedInteger("prompt".into(), number as u64))
                .group(group.clone())
                .flex()
                .flex_col()
                .items_end()
                .gap(px(4.))
                .when(handlers.prompts.chat, |prompt| {
                    prompt.on_mouse_down(gpui::MouseButton::Right, move |event, window, cx| {
                        cx.stop_propagation();
                        window.dispatch_action(Box::new(crate::actions::OpenPromptMenu { number, position: event.position }), cx);
                    })
                })
                .when(*steered, |prompt| {
                    prompt.child(div().pr(px(6.)).typography(Type::Small).text_color(theme.text_faint).child("Sent while working"))
                })
                .when(!pictures.is_empty(), |prompt| {
                    prompt.child(crate::pictures::Strip { pictures: pictures.clone(), key: format!("prompt-{}", pictures[0].key).into(), align_end: true })
                })
                .when(!files.is_empty(), |prompt| {
                    prompt.child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(6.))
                            .children(files.iter().map(|file| file_chip(file, theme, handlers))),
                    )
                })
                .when(!text.is_empty() || command.is_some() || editing.is_some(), |prompt| prompt.child(bubble))
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
                .map(|prompt| match card {
                    Some(card) => prompt.child(crate::rewind::card(card, theme)),
                    // The editor's own buttons stand where the actions would.
                    None if editing.is_some() => prompt.child(div().h(px(10.))),
                    None => prompt.child(crate::rewind::actions(number, copy, &handlers.prompts, group, theme)),
                })
                .tag("prompt")
                .into_any_element()
        }
        Item::Rewound { files } => crate::rewind::rewound(*files, theme),
        Item::Work(work) => work_log(work, theme, handlers),
        Item::Earlier { key, label, open } => fold_line(key, label, *open, theme, handlers),
        Item::Commit(commit) => crate::commit::view(commit, theme, handlers),
        Item::Reply { blocks, live } => reply_inked(blocks, *live && !handlers.still, theme, handlers).into_any_element(),
        Item::ReplyPictures { key, pictures, missing } => div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(!pictures.is_empty(), |column| column.child(crate::pictures::Strip { pictures: pictures.clone(), key: key.clone(), align_end: false }))
            .children(missing.iter().map(|reference| crate::pictures::missing(reference, theme)))
            .into_any_element(),
        Item::Plan(card) => crate::plan_card::plan_card(card, theme, handlers).into_any_element(),
        Item::Changes(card) => crate::changes_card::view(card, theme, handlers).into_any_element(),
        Item::Suggestions(tray) => crate::suggestion::tray(tray, theme, handlers).into_any_element(),
        Item::Canvas(card) => crate::canvas_card::view(card, theme).into_any_element(),
        Item::Footer(text) => div().typography(Type::Small).text_color(theme.text_faint).child(text.clone()).into_any_element(),
        Item::Notice { text, failed } => div()
            .typography(Type::Ui)
            .text_color(if *failed { theme.danger } else { theme.text_dim })
            .child(text.clone())
            .into_any_element(),
        Item::News(row) => log_row(row, false, theme, handlers),
        // Drawn above by `chat_transcript::item`.
        Item::ChatDivider(_)
        | Item::ChatSpeaker { .. }
        | Item::ChatBubble { .. }
        | Item::ChatThought { .. }
        | Item::ChatChips(_)
        | Item::ChatTyping(_)
        | Item::ChatFiles(_)
        | Item::ChatTidied(_) => div().into_any_element(),
        Item::Compacted(card) => crate::compaction_card::view(card, theme, handlers),
    };
    div().pt(px(gap_above(item, previous))).child(body).into_any_element()
}

/// The room a thought's clock keeps.
const CLOCK_WIDTH: f32 = 34.;

/// The most of the column a prompt's bubble takes, and its corners.
const BUBBLE_SHARE: f32 = 0.78;
const BUBBLE_RADIUS: f32 = 20.;
/// A step's icon, and the space after it.
const GLYPH: f32 = 14.;
const GLYPH_GAP: f32 = 8.;
/// How far an opened group's rows sit in from its line.
const INDENT: f32 = 20.;
/// A line about the work, and a row under an opened one.
const LINE: f32 = 24.;
const ROW: f32 = 27.;
/// An openable line's hover ground reaches this far past its words, without moving them.
const HOVER_PAD: f32 = 6.;

/// A group of steps. Settled, one grey line of what it did ("Read 3 files and ran a command"),
/// its chevron after the words, which opens to its rows; live, a line saying what it is doing
/// now over its newest rows as they come, the older ones behind "N earlier steps". Reasoning
/// alone is just its thoughts, flush with the words around them.
fn work_log(work: &Work, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if work.live && work.rows.is_empty() {
        return starting(work, theme, handlers);
    }
    if work.only_thoughts() {
        return div()
            .flex()
            .flex_col()
            .children(work.rows.iter().map(|row| log_row(row, work.live, theme, handlers)))
            .into_any_element();
    }
    let shown = &work.rows[work.hidden.min(work.rows.len())..];
    let head = if work.live {
        Some(live_line(work, theme, handlers))
    } else if work.label.is_empty() {
        None
    } else {
        Some(fold_line(&work.key, &work.label, work.open, theme, handlers))
    };
    let indented = head.is_some();
    div()
        .flex()
        .flex_col()
        .children(head)
        .when(work.open || head_less(work), |log| {
            log.child(
                div()
                    .flex()
                    .flex_col()
                    .when(indented, |rows| rows.pl(px(INDENT)))
                    .when(work.hidden > 0, |rows| rows.child(earlier_steps(work, theme, handlers)))
                    .children(shown.iter().map(|row| log_row(row, work.live, theme, handlers))),
            )
        })
        .into_any_element()
}

/// A settled group with nothing countable to name (only notes or events) shows its rows bare.
fn head_less(work: &Work) -> bool {
    !work.live && work.label.is_empty()
}

/// A settled group's line, or an earlier turn's: what it did, grey, the chevron after the words.
fn fold_line(key: &SharedString, label: &SharedString, open: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let toggle = handlers.toggle.clone();
    let id = key.clone();
    let key = key.clone();
    let (hover, bright) = (theme.hover, theme.text_soft);
    div()
        .flex()
        .child(
            div()
                .id(id)
                .h(px(LINE))
                .min_w_0()
                .ml(px(-HOVER_PAD))
                .px(px(HOVER_PAD))
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded(px(7.))
                .cursor_pointer()
                .typography(Type::Ui)
                .text_color(theme.text_dim)
                .hover(move |line| line.bg(hover).text_color(bright))
                .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
                .child(div().min_w_0().truncate().child(label.clone()))
                .child(div().flex_none().child(if open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint)))
                .tag("work-line"),
        )
        .into_any_element()
}

/// A live group's line: a breathing dot, and what it is doing now, with a light passing along
/// the words. The step under way says it too, so the line names the kind of work, not the step.
fn live_line(work: &Work, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let id = SharedString::from(format!("{}-doing", work.key));
    let words: SharedString = crate::work::doing_now(&work.rows).into();
    div()
        .h(px(LINE))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(live_dot(&work.key, theme, handlers))
        .child(div().typography(Type::Ui).child(crate::motion::shining_text(id, words, theme.text_dim, theme.accent_text, handlers.still)))
        .tag("work-line")
        .into_any_element()
}

/// A small dot in the accent, breathing while something is under way.
fn live_dot(key: &SharedString, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let dot = div().flex_none().size(px(6.)).rounded_full().bg(theme.accent);
    crate::motion::breathing(SharedString::from(format!("{key}-dot")), dot, handlers.still)
}

/// A live turn before its first step: the duck paddles and "Working" shines.
fn starting(work: &Work, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let id = SharedString::from(format!("{}-starting", work.key));
    div()
        .h(px(LINE))
        .flex()
        .items_center()
        .gap(px(7.))
        .child(crate::motion::paddling(div().flex_none().child(Icon::Duck.view(px(13.), theme.text_soft))))
        .child(div().typography(Type::Ui).child(crate::motion::shining_text(id, "Working".into(), theme.text_dim, theme.accent_text, handlers.still)))
        .into_any_element()
}

/// "28 earlier steps", over a live group's newest rows: shows them all.
fn earlier_steps(work: &Work, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let toggle = handlers.toggle.clone();
    let key = SharedString::from(format!("{}-all", work.key));
    let hover = theme.text_soft;
    let words = if work.hidden == 1 { "1 earlier step".to_string() } else { format!("{} earlier steps", work.hidden) };
    div().flex().child(
        div()
            .id(key.clone())
            .h(px(ROW))
            .flex()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            .typography(Type::Small)
            .text_color(theme.text_faint)
            .hover(move |line| line.text_color(hover))
            .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
            .child(words)
            .child(Icon::ChevronRight.view(px(10.), theme.text_faint)),
    )
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
        "Writing" | "Editing" => Some(Icon::Pencil),
        "Preparing" => Some(Icon::Hammer),
        "Asked" => Some(Icon::Question),
        "Used skill" => Some(Icon::Book),
        "Saved memory" | "Updated memory" | "Recalled memory" | "Forgot memory" => Some(Icon::Bookmark),
        "Reported" => Some(Icon::Duck),
        "Searched the web" | "Fetched" => Some(Icon::Globe),
        verb if crate::work::CANVAS_VERBS.contains(&verb) => Some(Icon::Board),
        verb if crate::work::BROWSER_VERBS.contains(&verb) => Some(Icon::Globe),
        verb if crate::work::COMPUTER_VERBS.contains(&verb) => Some(Icon::Cursor),
        "" => None,
        _ => Some(Icon::Hammer),
    }
}

/// Verbs whose subject is a path, shown as its file's name with its folder after it, faint.
fn names_a_path(verb: &str) -> bool {
    matches!(verb, "Read" | "Edited" | "Wrote" | "Listed" | "Writing" | "Editing")
}

/// A path as a file's name, then its folder, faint: "retry.rs crates/wack-llm/src".
fn path_words(path: &SharedString, theme: &Theme) -> AnyElement {
    let trimmed = path.trim_end_matches('/');
    let (folder, name) = match trimmed.rsplit_once('/') {
        Some((folder, name)) => (Some(folder.to_string()), name.to_string()),
        None => (None, trimmed.to_string()),
    };
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .overflow_hidden()
        .child(div().flex_none().typography(Type::UiStrong).text_color(theme.text_soft).child(name))
        .children(folder.map(|folder| div().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_faint).child(folder)))
        .into_any_element()
}

/// A row under an opened group: its icon, faint; its verb, grey; what it acted on, lighter; then
/// what came of it, and a chevron when it opens. No mark, no box. A step under way shines in its
/// verb, in the present tense, and its icon takes the accent; a failure says so in danger's
/// colour, and only there. `live`: the row is in a run still going, so it lands as it arrives.
fn log_row(row: &WorkRow, live: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
    if let Some(agent) = &row.agent {
        return agent_row(&row.key, agent, theme);
    }
    if row.is_thought() {
        return thought_line(row, live, theme, handlers);
    }
    if let Target::Note(text) = &row.target {
        return div().py(px(4.)).typography(Type::Ui).text_color(theme.text_soft).child(text.clone()).into_any_element();
    }
    let running = row.meta.running();
    // A command the model described ("Run the test suite") says so in its own words: "Ran"
    // before them would only repeat them. Under way, the words themselves shine.
    let described = row.verb.as_ref() == "Ran" && matches!(row.target, Target::Words(_));
    let target = match &row.target {
        Target::Words(text) if described && running => {
            let id = SharedString::from(format!("{}-doing", row.key));
            div().min_w_0().truncate().typography(Type::Ui).child(crate::motion::shining_text(id, text.clone(), theme.text_soft, theme.accent_text, handlers.still)).into_any_element()
        }
        Target::Code(text) if names_a_path(&row.verb) => path_words(text, theme),
        Target::Code(text) => div().min_w_0().truncate().typography(Type::Mono).text_color(theme.text_soft).child(text.clone()).into_any_element(),
        Target::Words(text) => div().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_soft).child(text.clone()).into_any_element(),
        Target::Note(_) | Target::Thought(_) | Target::Stream(_) => div().into_any_element(),
    };
    let small = |text: SharedString, colour: Hsla| div().flex_none().typography(Type::Small).text_color(colour).child(text);
    let marked = |icon: Icon, text: SharedString, colour: Hsla| {
        div().flex_none().flex().items_center().gap(px(4.)).child(icon.view(px(11.), colour)).child(small(text, colour))
    };
    let meta: Option<AnyElement> = match &row.meta {
        Meta::None | Meta::Running => None,
        Meta::Note(text) => Some(small(text.clone(), theme.text_faint).into_any_element()),
        Meta::Success(text) => Some(marked(Icon::Check, text.clone(), theme.success).into_any_element()),
        Meta::Failed(text) => Some(marked(Icon::Close, text.clone(), theme.danger).into_any_element()),
        Meta::Clock(time) => Some(small(time.clone(), theme.text_faint).into_any_element()),
        Meta::Thinking { started, at } => Some(small(thinking_clock(*started, *at, handlers), theme.text_faint).into_any_element()),
        // Only the sides that moved: a new file is "+12", not "+12 −0".
        Meta::Lines { added: 0, removed: 0 } => None,
        Meta::Lines { added, removed } => Some(
            div()
                .flex_none()
                .flex()
                .gap(px(5.))
                .typography(Type::Small)
                .when(*added > 0, |lines| lines.child(div().text_color(theme.diff_added).child(format!("+{added}"))))
                .when(*removed > 0, |lines| lines.child(div().text_color(theme.diff_removed).child(format!("−{removed}"))))
                .into_any_element(),
        ),
    };
    let verb = if running {
        let id = SharedString::from(format!("{}-doing", row.key));
        let words = SharedString::from(crate::work::present(&row.verb).to_string());
        crate::motion::shining_text(id, words, theme.text_dim, theme.accent_text, handlers.still)
    } else {
        row.verb.clone().into_any_element()
    };
    let openable = row.detail.is_some();
    let toggle = handlers.toggle.clone();
    let key = row.key.clone();
    let hover = theme.hover;
    let line = div()
        .id(row.key.clone())
        .h(px(ROW))
        .min_w_0()
        .ml(px(-HOVER_PAD))
        .px(px(HOVER_PAD))
        .flex()
        .items_center()
        .gap(px(GLYPH_GAP))
        .rounded(px(7.))
        .when(openable, |line| {
            line.cursor_pointer().hover(move |line| line.bg(hover)).on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        })
        .children(row_icon(&row.verb).map(|icon| icon.view(px(GLYPH), if running { theme.accent_text } else { theme.text_faint })))
        .when(!row.verb.is_empty() && !described, |line| line.child(div().flex_none().typography(Type::Ui).text_color(theme.text_dim).child(verb)))
        .child(target)
        .children(meta)
        .when(openable, |line| line.child(div().flex_none().child(if row.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint))));
    let line = div().flex().child(line);
    let line = if live && running { crate::motion::land(SharedString::from(format!("{}-land", row.key)), line, handlers.still) } else { line.into_any_element() };
    div()
        .flex()
        .flex_col()
        .child(line)
        .when(!row.pictures.is_empty(), |column| {
            column.child(div().pl(px(GLYPH + GLYPH_GAP)).pt(px(2.)).pb(px(6.)).child(crate::pictures::Strip {
                pictures: row.pictures.clone(),
                key: format!("{}-shots", row.key).into(),
                align_end: false,
            }))
        })
        .when_some(row.detail.as_ref().filter(|_| row.open), |column, detail| {
            column.child(div().pt(px(4.)).pb(px(8.)).child(detail_view(detail, &row.key, theme)))
        })
        .into_any_element()
}

/// A thought: the duck, then "Thought for 12.3s", grey, its chevron after. While the model is
/// having it, the duck paddles and "Thinking" shines, its clock counting. Either opens to the
/// thought itself, plain grey words under the line.
fn thought_line(row: &WorkRow, live: bool, theme: &Theme, handlers: &Handlers) -> AnyElement {
    let streaming = matches!(row.target, Target::Stream(_));
    let openable = row.detail.is_some();
    let toggle = handlers.toggle.clone();
    let key = row.key.clone();
    let (hover, bright) = (theme.hover, theme.text_soft);
    let label = if streaming {
        let id = SharedString::from(format!("{}-thinking", row.key));
        crate::motion::shining_text(id, "Thinking".into(), theme.text_dim, theme.accent_text, handlers.still)
    } else {
        row.verb.clone().into_any_element()
    };
    let duck = div().flex_none().child(Icon::Duck.view(px(13.), if streaming { theme.text_soft } else { theme.text_dim }));
    let duck = if streaming { crate::motion::paddling(duck) } else { duck };
    let clock = match &row.meta {
        Meta::Clock(time) => Some(time.clone()),
        Meta::Thinking { started, at } => Some(thinking_clock(*started, *at, handlers)),
        _ => None,
    };
    let line = div()
        .id(row.key.clone())
        .h(px(LINE))
        .ml(px(-HOVER_PAD))
        .px(px(HOVER_PAD))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .typography(Type::Ui)
        .text_color(theme.text_dim)
        .when(openable, |line| {
            line.cursor_pointer().hover(move |line| line.bg(hover).text_color(bright)).on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        })
        .child(duck)
        .child(div().flex_none().child(label))
        .when_some(clock, |line, time| {
            // Wide enough for "88.8s", so what follows holds still as it counts.
            line.child(div().flex_none().min_w(px(CLOCK_WIDTH)).typography(Type::Small).text_color(theme.text_faint).child(time))
        })
        .when(openable, |line| line.child(div().flex_none().child(if row.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(11.), theme.text_faint))))
        .tag("thought-line");
    let line = div().flex().child(line);
    let line = if live && streaming { crate::motion::land(SharedString::from(format!("{}-land", row.key)), line, handlers.still) } else { line.into_any_element() };
    div()
        .flex()
        .flex_col()
        .child(line)
        .when_some(row.detail.as_ref().filter(|_| row.open), |column, detail| {
            column.child(div().pt(px(2.)).pb(px(8.)).child(detail_view(detail, &row.key, theme)))
        })
        .into_any_element()
}

/// A sub-agent's row: its duck (paddling while it works), its role and task, how it stands, and a
/// line of what it is doing or did. The whole row opens its work in the Agents pane.
pub fn agent_row(key: &SharedString, agent: &AgentLine, theme: &Theme) -> AnyElement {
    // A queued agent hasn't started: its duck waits, dimmed.
    let working = matches!(agent.state, AgentState::Working | AgentState::Background);
    let (word, colour) = agent_state_word(agent.state, theme);
    let duck = div().flex_none().w(px(GLYPH)).h(px(ROW)).flex().items_center().justify_center().child(if working {
        crate::motion::paddling(div().child(Icon::Duck.view(px(13.), theme.accent_text))).into_any_element()
    } else {
        Icon::Duck.view(px(13.), theme.text_faint).into_any_element()
    });
    let id = agent.id.clone();
    let hover = theme.hover;
    div()
        .id(key.clone())
        .ml(px(-HOVER_PAD))
        .px(px(HOVER_PAD))
        .pb(px(3.))
        .rounded(px(8.))
        .flex()
        .items_start()
        .gap(px(GLYPH_GAP))
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
                .child(
                    div()
                        .h(px(ROW))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_none().typography(Type::UiStrong).text_color(theme.text_soft).child(agent.role.clone()))
                        .children(agent.tier.map(|tier| tier_chip(tier, theme)))
                        .child(div().flex_1().min_w_0().truncate().typography(Type::Ui).text_color(theme.text_dim).child(agent.task.clone()))
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

/// What a row opens into, under it: a diff or output in a sheet, the questions it asked, or a
/// thought's words.
fn detail_view(detail: &Detail, key: &SharedString, theme: &Theme) -> AnyElement {
    match detail {
        Detail::Diff(diff) => diff_sheet(diff, key, theme).into_any_element(),
        Detail::Output(text) => sheet(theme)
            .child(
                div()
                    .id(SharedString::from(format!("{key}-output")))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .px(px(14.))
                    .py(px(10.))
                    .typography(Type::Code)
                    .text_color(theme.text_dim)
                    .child(text.clone()),
            )
            .into_any_element(),
        Detail::Asked(asked) => div().pl(px(GLYPH + GLYPH_GAP)).py(px(4.)).child(crate::question::asked_view(asked, theme)).into_any_element(),
        Detail::Thought { text, live } => thought_body(text, *live, key, theme),
        Detail::Text(text) => div()
            .pl(px(GLYPH + GLYPH_GAP))
            .typography(Type::Ui)
            .text_color(theme.text_dim)
            .child(text.clone())
            .into_any_element(),
    }
}

/// The tallest a thought's open body grows; the rest scrolls inside it, so opening a long
/// thought never sends the page scrolling endlessly (version 2 capped it the same way).
const THOUGHT_BODY: f32 = 320.;

/// A thought opened: its paragraphs, grey, indented under the duck's line, in a body of capped
/// height. Live, the body holds to its end so the newest reasoning shows as it comes; settled,
/// it scrolls.
fn thought_body(text: &SharedString, live: bool, key: &SharedString, theme: &Theme) -> AnyElement {
    let paragraphs = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .typography(Type::Ui)
        .text_color(if live { theme.text_soft } else { theme.text_dim })
        .children(text.split("\n\n").map(str::trim).filter(|paragraph| !paragraph.is_empty()).map(|paragraph| div().child(SharedString::from(paragraph.to_string()))));
    let frame = div().pl(px(INDENT - 1.)).pr(px(12.)).max_h(px(THOUGHT_BODY));
    if live {
        frame.overflow_hidden().flex().flex_col().justify_end().child(paragraphs).into_any_element()
    } else {
        frame.id(SharedString::from(format!("{key}-body"))).overflow_y_scroll().child(paragraphs).into_any_element()
    }
}

/// A sheet: code, a diff or output, on the code's ground inside a hairline. No band, no seam:
/// the row above it already says what it holds.
fn sheet(theme: &Theme) -> gpui::Div {
    div().rounded(px(10.)).border_1().border_color(theme.border).bg(theme.code_ground).overflow_hidden().flex().flex_col()
}

/// A reply's code block head: what it holds and its copy button, over a hairline.
fn head_band(theme: &Theme) -> gpui::Div {
    div().flex_none().h(px(30.)).px(px(12.)).flex().items_center().gap(px(8.)).border_b_1().border_color(theme.border)
}

fn diff_sheet(diff: &Diff, key: &SharedString, theme: &Theme) -> impl IntoElement {
    let lines = div().flex().flex_col().py(px(6.)).children(diff.lines.iter().map(|line| diff_line(line, theme)));
    sheet(theme).child(div().id(SharedString::from(format!("{key}-diff"))).max_h(px(420.)).overflow_y_scroll().child(lines))
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
            // Inline code sits on a faint wash, as a word set apart.
            background_color: span.code.then_some(theme.hover),
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
    prompts: crate::rewind::Prompts,
}

impl Transcript {
    pub fn new(items: Vec<Item>) -> Self {
        Self { items, prompts: Default::default() }
    }

    /// What the user's messages offer, as a chat's do.
    pub fn prompts(mut self, prompts: crate::rewind::Prompts) -> Self {
        self.prompts = prompts;
        self
    }
}

impl RenderOnce for Transcript {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::get(cx).clone();
        let handlers = Handlers { still: cx.reduce_motion(), prompts: self.prompts, ..Handlers::inert() };
        let mut column = div().w_full().flex().flex_col();
        for (index, current) in self.items.iter().enumerate() {
            let previous = index.checked_sub(1).map(|index| &self.items[index]);
            column = column.child(item(current, previous, &theme, &handlers));
        }
        column
    }
}

const ATTACHED_FILE_MAX: f32 = 260.;

/// An attached text file that went with a user message: clicking expands the exact text sent.
fn file_chip(file: &AttachedFileShown, theme: &Theme, handlers: &Handlers) -> impl IntoElement {
    let toggle = handlers.toggle.clone();
    let key = file.key.clone();
    let line_words = if file.lines == 1 { "1 line".to_string() } else { format!("{} lines", file.lines) };
    let toggle_head = div()
        .id(ElementId::Name(format!("{}-head", file.key).into()))
        .h(px(34.))
        .px(px(12.))
        .when(file.open, |head| head.flex_1().min_w_0())
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .hover(|head| head.bg(theme.hover))
        .on_click(move |_, window, cx| toggle(key.clone(), window, cx))
        .child(Icon::File.view(px(13.), theme.text_dim))
        .child(div().min_w_0().truncate().typography(Type::Mono).text_color(theme.text_soft).child(file.name.clone()))
        .child(div().flex_none().typography(Type::Small).text_color(theme.text_faint).child(line_words))
        .child(if file.open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))
        .tag("attached-file-chip");
    let header = div()
        .flex()
        .items_center()
        .child(toggle_head)
        .when(file.open, |row| {
            let copy = crate::copy::CopyButton::new(format!("{}-copy", file.key), file.text.clone(), "Copy", "copy-attached-file");
            row.child(div().flex_none().pr(px(10.)).child(copy))
        });
    div()
        .max_w(relative(BUBBLE_SHARE))
        .when(file.open, |chip| chip.w_full())
        .rounded(px(10.))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border)
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(header)
        .when(file.open, |chip| {
            chip.child(
                div()
                    .id(ElementId::Name(format!("{}-body", file.key).into()))
                    .max_h(px(ATTACHED_FILE_MAX))
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .px(px(12.))
                    .py(px(10.))
                    .typography(Type::Mono)
                    .text_color(theme.text_soft)
                    .child(file.text.clone())
                    .tag("attached-file-content"),
            )
        })
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
