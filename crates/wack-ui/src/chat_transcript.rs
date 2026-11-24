//! The Chat area's transcript: a chat read as a messenger reads, over the same conversation the
//! Code transcript draws. Ported from version 2 (`chat-clusters.ts`, `chat-activity.ts`,
//! `chat-scratchpad.ts`, `ChatBubbles.tsx`), its pure parts tested here.
//!
//! - The user's message is the same bubble on the right. Each reply opens with the duck and the
//!   agent's name, then reads as speech: "Thought it over" folded, what it did as chips in plain
//!   words ("Jotted down groceries.md", "Looked up example.com"), and its prose in bubbles.
//! - Three or more settled calls in a row fold into one chip that says what they came to ("Jotted
//!   down 2 files, looked up 3 pages"); a live call, a failure, and a call that brought back a
//!   picture always show on their own.
//! - A quiet time divider ("Today · 14:02") opens the chat and any stretch after half an hour's
//!   silence. It carries the time, not the words, so it reads right after midnight too.
//! - While the agent works and has not started its words, a typing bubble says what it is up to
//!   ("pondering…", "on it…", "tidying up the conversation…").
//! - A settled reply lists the files it wrote in the chat's scratchpad as chips that reveal them in
//!   the Finder. The list is built from what the model wrote, so only paths that resolve inside the
//!   scratchpad are offered, and the app checks again before revealing one.
//!
//! Built one entry at a time, as `work::entry_items` is, so a long chat costs nothing extra while a
//! reply streams: an entry's items depend only on that entry and the ones before it.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use gpui::{AnimationExt, AnyElement, ClipboardItem, ElementId, SharedString, div, prelude::*, px, relative};
use serde_json::Value;
use wack_core::clock;

use crate::{
    conversation::{Conversation, Entry, Step, ToolStep, Turn, TurnEnd},
    icon::Icon,
    tags::Tag,
    theme::Theme,
    transcript::{Handlers, Item},
    typography::{Type, Typography},
    work::ReplyCache,
};

/// A new cluster after this much silence: the agent's name shows again.
pub const GAP_MS: u64 = 3 * 60 * 1000;
/// A time divider only when the conversation resumes after a real pause.
pub const DIVIDER_GAP_MS: u64 = 30 * 60 * 1000;
/// Runs this long or longer fold into one chip once they have all settled.
pub const FOLD_AT: usize = 3;
/// Who speaks for the Chat area. Renaming the agent is not a feature of version 3.
pub const AGENT_NAME: &str = "WackCode";

/// "Today · 14:02", "Yesterday · 09:15", or "Mon 3 Mar · 18:40" for anything older.
pub fn divider_label(time: u64, now: u64) -> String {
    let local = clock::local(time);
    let clock_words = format!("{:02}:{:02}", local.hour, local.minute);
    let today = clock::start_of_day(now, 0);
    if time >= today {
        format!("Today · {clock_words}")
    } else if time >= clock::start_of_day(now, 1) {
        format!("Yesterday · {clock_words}")
    } else {
        let weekday = clock::WEEKDAYS[local.weekday as usize % 7];
        let month = clock::MONTHS[(local.month as usize + 11) % 12];
        format!("{weekday} {} {month} · {clock_words}", local.day)
    }
}

/// When entry `index` happened: a message by its turn's start, a turn by its own.
fn began(conversation: &Conversation, index: usize) -> Option<u64> {
    match conversation.entries.get(index)? {
        Entry::Prompt { .. } => match conversation.entries.get(index + 1) {
            Some(Entry::Turn(turn)) => turn.started,
            _ => None,
        },
        Entry::Turn(turn) => turn.started,
        _ => None,
    }
}

/// When the conversation was last heard before entry `index`.
fn heard_before(conversation: &Conversation, index: usize) -> Option<u64> {
    conversation.entries[..index.min(conversation.entries.len())].iter().rev().find_map(|entry| match entry {
        Entry::Turn(turn) => turn.ended.or(turn.started),
        _ => None,
    })
}

/// The divider entry `index` opens with, if any: the first message of the chat, or one after
/// [`DIVIDER_GAP_MS`] of quiet.
pub fn divider_before(conversation: &Conversation, index: usize) -> Option<u64> {
    // A turn answering a message shares the message's divider.
    if matches!(conversation.entries.get(index), Some(Entry::Turn(_)))
        && matches!(index.checked_sub(1).and_then(|before| conversation.entries.get(before)), Some(Entry::Prompt { .. }))
    {
        return None;
    }
    let at = began(conversation, index)?;
    match heard_before(conversation, index) {
        None => Some(at),
        Some(before) if at.saturating_sub(before) > DIVIDER_GAP_MS => Some(at),
        _ => None,
    }
}

/// Whether the turn at `index` opens a run of the agent's: anything but a turn that follows the
/// agent's own words within [`GAP_MS`].
pub fn opens_cluster(conversation: &Conversation, index: usize) -> bool {
    let Some(Entry::Turn(previous)) = index.checked_sub(1).and_then(|before| conversation.entries.get(before)) else {
        return true;
    };
    match (previous.ended.or(previous.started), began(conversation, index)) {
        (Some(before), Some(at)) => at.saturating_sub(before) > GAP_MS,
        _ => false,
    }
}

/// What the duck is up to while it works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// Nothing to say yet: three dots.
    Typing,
    Thinking,
    Tidying,
    Using,
}

impl Mood {
    pub fn caption(self) -> Option<&'static str> {
        match self {
            Mood::Typing => None,
            Mood::Thinking => Some("pondering…"),
            Mood::Tidying => Some("tidying up the conversation…"),
            Mood::Using => Some("on it…"),
        }
    }
}

/// The typing bubble a live turn shows, if any: none once its words are coming in.
pub fn mood(turn: &Turn) -> Option<Mood> {
    if !turn.live || !turn.reply.trim().is_empty() {
        return None;
    }
    Some(match turn.steps.last() {
        Some(Step::Thought(_)) => Mood::Thinking,
        Some(Step::Event(_)) => Mood::Tidying,
        Some(Step::Tool(tool)) if tool.result.is_none() => Mood::Using,
        _ => Mood::Typing,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Live,
    Done,
    Failed,
}

/// One call, as a chip says it.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub label: SharedString,
    pub icon: Icon,
    pub status: Status,
}

/// A chip in a reply: one call, or a settled run of them folded into what they came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Chip {
    Single(Activity),
    Fold { key: SharedString, label: SharedString, open: bool, activities: Vec<Activity> },
}

/// Tools a reader has no use for: finding a deferred tool, and the canvas's own manual.
fn hidden(name: &str) -> bool {
    matches!(name, wack_core::TOOL_SEARCH | wack_canvas::tools::CANVAS_GUIDE)
}

fn text<'a>(input: &'a Option<Value>, name: &str) -> &'a str {
    input.as_ref().and_then(|input| input.get(name)).and_then(Value::as_str).map(str::trim).unwrap_or("")
}

/// A path's last part: the scratchpad is the chat's own, so folders rarely matter.
pub fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or(trimmed)
}

/// Where a page is: its host, without `www.`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host)
}

/// Words typed by the model, quoted and kept short.
fn quoted(words: &str) -> String {
    let flat = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = flat.chars().take(32).collect();
    if short.len() < flat.len() { format!("“{short}…”") } else { format!("“{short}”") }
}

fn mcp_parts(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(wack_core::mcp_client::MCP_PREFIX)?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

/// What a chip says for one call, and its icon: in the present while it runs, the past after.
pub fn label(tool: &ToolStep, live: bool) -> (String, Icon) {
    let input = &tool.input;
    let pick = |now: &str, then: &str| if live { now.to_string() } else { then.to_string() };
    let join = |verb: String, subject: &str| if subject.is_empty() { verb } else { format!("{verb} {subject}") };
    let name = tool.name.as_str();
    match name {
        "read" => (join(pick("Reading", "Read"), base_name(text(input, "path"))), Icon::File),
        "write" => (join(pick("Jotting down", "Jotted down"), base_name(text(input, "path"))), Icon::Pencil),
        "edit" => (join(pick("Touching up", "Touched up"), base_name(text(input, "path"))), Icon::Pencil),
        "ls" => (pick("Peeking at the scratchpad", "Peeked at the scratchpad"), Icon::Folder),
        "grep" => {
            let pattern = text(input, "pattern");
            (join(pick("Searching for", "Searched for"), &if pattern.is_empty() { String::new() } else { quoted(pattern) }), Icon::Search)
        }
        "find" => {
            let pattern = text(input, "pattern");
            (join(pick("Looking for", "Looked for"), &if pattern.is_empty() { "files".into() } else { quoted(pattern) }), Icon::Search)
        }
        wack_core::WEB_FETCH => {
            let url = text(input, "url");
            (join(pick("Looking up", "Looked up"), if url.is_empty() { "a page" } else { host(url) }), Icon::Globe)
        }
        "web_search" => {
            let query = text(input, "query");
            (join(pick("Searching the web for", "Searched the web for"), &if query.is_empty() { "something".into() } else { quoted(query) }), Icon::Search)
        }
        wack_core::memory::MEMORY_SAVE => {
            let title = text(input, "title");
            (join(pick("Remembering", "Remembered"), &if title.is_empty() { "that".into() } else { quoted(title) }), Icon::Brain)
        }
        wack_core::memory::MEMORY_RECALL => (pick("Recalling a memory", "Recalled a memory"), Icon::Brain),
        wack_core::memory::MEMORY_FORGET => (pick("Forgetting a memory", "Forgot a memory"), Icon::Brain),
        wack_core::ask::ASK_USER_QUESTION => (pick("Asking you", "Asked you"), Icon::Question),
        wack_core::chat_refs::READ_CHAT => (pick("Reading another chat", "Read another chat"), Icon::Comment),
        wack_canvas::tools::CANVAS => (pick("Drawing on a canvas", "Drew on a canvas"), Icon::Board),
        wack_canvas::tools::CANVAS_LOOK => (pick("Looking at the canvas", "Looked at the canvas"), Icon::Eye),
        "list_images" | "read_image" => (pick("Looking at a picture", "Looked at a picture"), Icon::Eye),
        "browser_navigate" => {
            let url = text(input, "url");
            (join(pick("Opening", "Opened"), if url.is_empty() { "a page" } else { host(url) }), Icon::Globe)
        }
        "browser_snapshot" | "browser_text" => (pick("Reading the page", "Read the page"), Icon::Globe),
        "browser_screenshot" => (pick("Snapping the page", "Snapped the page"), Icon::Eye),
        "browser_console" | "browser_network" => (pick("Checking the page", "Checked the page"), Icon::Globe),
        name if name.starts_with("browser_") => (pick("Using the page", "Used the page"), Icon::Cursor),
        name => match mcp_parts(name) {
            Some((server, tool)) => (format!("{} {} ({server})", pick("Using", "Used"), tool.replace('_', " ")), Icon::Link),
            None => (format!("{} {}", pick("Using", "Used"), name.replace('_', " ")), Icon::Spark),
        },
    }
}

/// One activity per call, `live` while the reply is still being written.
pub fn activity(tool: &ToolStep, live: bool) -> Activity {
    let status = match &tool.result {
        Some(result) if result.is_error => Status::Failed,
        Some(_) => Status::Done,
        None if live => Status::Live,
        None => Status::Done,
    };
    let (label, icon) = label(tool, status == Status::Live);
    Activity { label: label.into(), icon, status }
}

/// What a folded run came to, in the chat's casual register: "Jotted down 2 files, looked up 3
/// pages". Writing comes first, then what was looked up, then what was read; the two biggest
/// say their piece and the rest are counted.
pub fn summarize_fold(tools: &[&ToolStep]) -> String {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Bucket {
        Write,
        Edit,
        Fetch,
        Search,
        Browser,
        Mcp,
        Read,
        Look,
        Memory,
        Other,
    }
    struct Acc {
        bucket: Bucket,
        order: usize,
        paths: Vec<String>,
        count: usize,
        recalled: bool,
        saved: bool,
    }
    let tier = |bucket: Bucket| match bucket {
        Bucket::Write | Bucket::Edit => 1,
        Bucket::Fetch | Bucket::Search | Bucket::Browser | Bucket::Mcp => 2,
        _ => 3,
    };
    let mut buckets: Vec<Acc> = Vec::new();
    for tool in tools {
        let bucket = match tool.name.as_str() {
            "write" => Bucket::Write,
            "edit" => Bucket::Edit,
            "read" => Bucket::Read,
            wack_core::WEB_FETCH => Bucket::Fetch,
            "web_search" => Bucket::Search,
            "ls" | "grep" | "find" => Bucket::Look,
            wack_core::memory::MEMORY_SAVE | wack_core::memory::MEMORY_RECALL | wack_core::memory::MEMORY_FORGET => Bucket::Memory,
            name if name.starts_with("browser_") => Bucket::Browser,
            name if mcp_parts(name).is_some() => Bucket::Mcp,
            _ => Bucket::Other,
        };
        let order = buckets.len();
        let at = match buckets.iter().position(|acc| acc.bucket == bucket) {
            Some(at) => at,
            None => {
                buckets.push(Acc { bucket, order, paths: Vec::new(), count: 0, recalled: false, saved: false });
                buckets.len() - 1
            }
        };
        let acc = &mut buckets[at];
        acc.count += 1;
        let path = text(&tool.input, "path");
        if matches!(bucket, Bucket::Write | Bucket::Edit | Bucket::Read) && !path.is_empty() && !acc.paths.iter().any(|seen| seen == path) {
            acc.paths.push(path.to_string());
        }
        if tool.name == wack_core::memory::MEMORY_RECALL {
            acc.recalled = true;
        } else if bucket == Bucket::Memory {
            acc.saved = true;
        }
    }
    let size = |acc: &Acc| if acc.paths.is_empty() { acc.count } else { acc.paths.len() };
    buckets.sort_by(|a, b| tier(a.bucket).cmp(&tier(b.bucket)).then(size(b).cmp(&size(a))).then(a.order.cmp(&b.order)));
    let files = |n: usize| if n == 1 { "file" } else { "files" };
    let phrase = |acc: &Acc| -> String {
        let n = size(acc);
        let single = (acc.paths.len() == 1).then(|| base_name(&acc.paths[0]).to_string());
        match acc.bucket {
            Bucket::Read => match single {
                Some(name) if acc.count == 1 => format!("read {name}"),
                _ => format!("read {n} {}", files(n)),
            },
            Bucket::Write => single.map_or_else(|| format!("jotted down {n} {}", files(n)), |name| format!("jotted down {name}")),
            Bucket::Edit => single.map_or_else(|| format!("updated {n} {}", files(n)), |name| format!("updated {name}")),
            Bucket::Fetch => format!("looked up {n} {}", if n == 1 { "page" } else { "pages" }),
            Bucket::Search => if n == 1 { "searched the web".into() } else { format!("searched the web {n} times") },
            Bucket::Look => if n == 1 { "checked files".into() } else { format!("checked {n} times") },
            Bucket::Memory if acc.recalled && acc.saved => "used memory".into(),
            Bucket::Memory if acc.saved => "updated memory".into(),
            Bucket::Memory => "recalled memory".into(),
            Bucket::Browser => format!("browsed {n} {}", if n == 1 { "step" } else { "steps" }),
            Bucket::Mcp => format!("used {n} {}", if n == 1 { "tool" } else { "tools" }),
            Bucket::Other => format!("did {n} {}", if n == 1 { "thing" } else { "things" }),
        }
    };
    let shown: Vec<String> = buckets.iter().take(2).map(phrase).collect();
    let overflow: usize = buckets.iter().skip(2).map(size).sum();
    let words = if overflow > 0 { format!("{}, +{overflow} more", shown.join(", ")) } else { shown.join(", ") };
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => format!("Did {} things", tools.len()),
    }
}

/// A reply's run of calls as chips: settled runs of [`FOLD_AT`] or more fold under `key`; a live
/// call, a failure and a call that brought back pictures always show.
pub fn chips(tools: &[&ToolStep], live: bool, key: &str, open: &HashSet<String>) -> Vec<Chip> {
    let mut chips = Vec::new();
    let mut run: Vec<&ToolStep> = Vec::new();
    let mut run_start = 0;
    let flush = |run: &mut Vec<&ToolStep>, start: usize, chips: &mut Vec<Chip>| {
        if run.len() >= FOLD_AT {
            let fold_key = format!("{key}-fold-{start}");
            chips.push(Chip::Fold {
                label: summarize_fold(run).into(),
                open: open.contains(&fold_key),
                key: fold_key.into(),
                activities: run.iter().map(|tool| activity(tool, live)).collect(),
            });
        } else {
            chips.extend(run.iter().map(|tool| Chip::Single(activity(tool, live))));
        }
        run.clear();
    };
    for (at, tool) in tools.iter().enumerate() {
        let settled = tool.result.as_ref().is_some_and(|result| !result.is_error && result.pictures.is_empty());
        if settled {
            if run.is_empty() {
                run_start = at;
            }
            run.push(tool);
            continue;
        }
        flush(&mut run, run_start, &mut chips);
        chips.push(Chip::Single(activity(tool, live)));
    }
    flush(&mut run, run_start, &mut chips);
    chips
}

/// A file a reply wrote in the chat's scratchpad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScratchFile {
    /// Inside the scratchpad.
    pub path: PathBuf,
    pub name: SharedString,
    /// The folder inside the scratchpad it is in, "" at its root.
    pub folder: SharedString,
}

/// `raw` (as the model wrote it) against `scratchpad`, lexically; `None` when it lands outside.
pub fn resolve_in(raw: &str, scratchpad: &Path) -> Option<PathBuf> {
    let cleaned = raw.trim().trim_start_matches('@');
    let cleaned = cleaned.strip_prefix("file://").unwrap_or(cleaned);
    if cleaned.is_empty() || cleaned.starts_with('~') {
        return None;
    }
    let joined = if Path::new(cleaned).is_absolute() { PathBuf::from(cleaned) } else { scratchpad.join(cleaned) };
    let mut normal = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    (normal != scratchpad && normal.starts_with(scratchpad)).then_some(normal)
}

/// Each file a reply wrote or edited without error, once, in the order first touched.
pub fn scratchpad_files(tools: &[&ToolStep], scratchpad: &Path) -> Vec<ScratchFile> {
    let mut files: Vec<ScratchFile> = Vec::new();
    for tool in tools {
        if tool.name != "write" && tool.name != "edit" {
            continue;
        }
        if !tool.result.as_ref().is_some_and(|result| !result.is_error) {
            continue;
        }
        let Some(path) = resolve_in(text(&tool.input, "path"), scratchpad) else { continue };
        if files.iter().any(|file| file.path == path) {
            continue;
        }
        let inside = path.strip_prefix(scratchpad).unwrap_or(&path);
        let folder = inside.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
        let name = inside.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        files.push(ScratchFile { path, name: name.into(), folder: folder.into() });
    }
    files
}

/// The items for entry `index` of a Chat chat, appended to `items`. `scratchpad` is the chat's
/// folder; `open` holds the keys of opened thoughts and folds.
pub fn entry_items(conversation: &Conversation, index: usize, scratchpad: &Path, open: &HashSet<String>, cache: &mut ReplyCache, items: &mut Vec<Item>) {
    let Some(entry) = conversation.entries.get(index) else {
        return;
    };
    if let Some(at) = divider_before(conversation, index) {
        items.push(Item::ChatDivider(at));
    }
    match entry {
        Entry::Prompt { text, steered, pictures, .. } => items.push(Item::Prompt {
            text: text.clone().into(),
            steered: *steered,
            command: None,
            suggestion: None,
            pictures: pictures.clone(),
        }),
        Entry::Compacted => items.push(Item::ChatTidied),
        Entry::Notice { text, failed } => items.push(Item::Notice { text: text.clone().into(), failed: *failed }),
        // A Chat chat has no background tasks or sub-agents to hear from.
        Entry::News(_) => {}
        Entry::Turn(turn) => turn_items(conversation, index, turn, scratchpad, open, cache, items),
    }
}

fn turn_items(conversation: &Conversation, index: usize, turn: &Turn, scratchpad: &Path, open: &HashSet<String>, cache: &mut ReplyCache, items: &mut Vec<Item>) {
    let key = format!("turn-{index}");
    let says_something = turn.live || !turn.steps.is_empty() || !turn.reply.trim().is_empty() || turn.end.is_some() || !turn.notices.is_empty();
    if !says_something {
        return;
    }
    if opens_cluster(conversation, index) {
        items.push(Item::ChatSpeaker { live: turn.live });
    }
    let last = turn.steps.len().saturating_sub(1);
    let mut run: Vec<&ToolStep> = Vec::new();
    let mut thought: Option<(usize, String)> = None;
    let mut pieces = 0;
    let flush_run = |run: &mut Vec<&ToolStep>, items: &mut Vec<Item>| {
        if run.is_empty() {
            return;
        }
        // A canvas drawn without fault is its card, below; only one still drawing, or one that
        // failed, is a chip.
        let shown: Vec<&ToolStep> = run
            .iter()
            .copied()
            .filter(|tool| tool.name != wack_canvas::tools::CANVAS || tool.result.as_ref().is_none_or(|result| result.is_error))
            .collect();
        if !shown.is_empty() {
            let first = shown.iter().position(|_| true).unwrap_or(0);
            items.push(Item::ChatChips(chips(&shown, turn.live, &format!("{key}-{first}-{}", run.len()), open)));
        }
        run.clear();
    };
    let flush_thought = |thought: &mut Option<(usize, String)>, items: &mut Vec<Item>| {
        if let Some((at, text)) = thought.take() {
            let thought_key = format!("{key}-thought-{at}");
            items.push(Item::ChatThought { open: open.contains(&thought_key), key: thought_key.into(), text: text.into() });
        }
    };
    for (at, step) in turn.steps.iter().enumerate() {
        match step {
            Step::Thought(text) => {
                // Thinking still under way is the typing bubble's "pondering…".
                if (turn.live && at == last) || text.trim().is_empty() {
                    continue;
                }
                flush_run(&mut run, items);
                match &mut thought {
                    Some((_, gathered)) => {
                        gathered.push_str("\n\n");
                        gathered.push_str(text.trim());
                    }
                    None => thought = Some((at, text.trim().to_string())),
                }
            }
            Step::Note(text) => {
                flush_run(&mut run, items);
                flush_thought(&mut thought, items);
                if !text.trim().is_empty() {
                    items.push(Item::ChatBubble { blocks: cache.piece(index, pieces, text), copy: None });
                    pieces += 1;
                }
            }
            Step::Tool(tool) => {
                if hidden(&tool.name) {
                    continue;
                }
                flush_thought(&mut thought, items);
                run.push(tool);
            }
            Step::Event(_) => {
                flush_run(&mut run, items);
                flush_thought(&mut thought, items);
                // Compacting now is the typing bubble's; done, it is a quiet line.
                if !(turn.live && at == last) {
                    items.push(Item::ChatTidied);
                }
            }
            Step::Task(_) => {}
        }
    }
    flush_run(&mut run, items);
    flush_thought(&mut thought, items);

    let outcomes: Vec<wack_canvas::tools::Outcome> = turn
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Tool(tool) if tool.name == wack_canvas::tools::CANVAS => tool.result.as_ref(),
            _ => None,
        })
        .filter(|result| !result.is_error)
        .filter_map(|result| wack_canvas::tools::outcome(&result.text))
        .collect();
    for card in crate::canvas_card::cards(&key, &outcomes, turn.live) {
        items.push(Item::Canvas(card));
    }

    if !turn.reply.trim().is_empty() {
        let blocks = cache.piece(index, pieces, &turn.reply);
        let named = crate::pictures::referenced(&blocks);
        items.push(Item::ChatBubble { blocks, copy: (!turn.live).then(|| turn.reply.trim().to_string().into()) });
        if !named.is_empty() {
            let (found, missing): (Vec<_>, Vec<_>) = named
                .into_iter()
                .map(|reference| conversation.pictures.iter().find(|picture| picture.answers_to(&reference)).cloned().ok_or(reference))
                .partition(Result::is_ok);
            items.push(Item::ReplyPictures {
                key: format!("reply-pictures-{index}").into(),
                pictures: found.into_iter().flatten().collect(),
                missing: missing.into_iter().filter_map(Result::err).map(Into::into).collect(),
            });
        }
    }
    if let Some(retrying) = &turn.retrying {
        items.push(Item::Notice { text: retrying.clone().into(), failed: false });
    }
    if let Some(mood) = mood(turn) {
        items.push(Item::ChatTyping(mood));
    }
    if !turn.live {
        let tools: Vec<&ToolStep> = turn.steps.iter().filter_map(|step| if let Step::Tool(tool) = step { Some(tool) } else { None }).collect();
        let files = scratchpad_files(&tools, scratchpad);
        if !files.is_empty() {
            items.push(Item::ChatFiles(files));
        }
    }
    for notice in &turn.notices {
        items.push(Item::Notice { text: notice.clone().into(), failed: false });
    }
    match &turn.end {
        Some(TurnEnd::Stopped) => items.push(Item::Notice { text: "Stopped.".into(), failed: false }),
        Some(TurnEnd::CutOff) => items.push(Item::Notice { text: "The reply reached its length limit before it was finished.".into(), failed: true }),
        Some(TurnEnd::Refused) => items.push(Item::Notice { text: "The model declined to continue.".into(), failed: true }),
        Some(TurnEnd::Failed(message)) => items.push(Item::Notice { text: message.clone().into(), failed: true }),
        Some(TurnEnd::Done) | None => {}
    }
}

/// The space a Chat item keeps from the one before, or `None` for the Code transcript's rule.
pub fn gap_above(item: &Item, previous: Option<&Item>) -> Option<f32> {
    let previous = previous?;
    Some(match (previous, item) {
        (_, Item::ChatDivider(_)) => 30.,
        (Item::ChatDivider(_), _) => 14.,
        (_, Item::ChatSpeaker { .. }) => 26.,
        (Item::ChatSpeaker { .. }, _) => 8.,
        (_, Item::Prompt { .. }) => 26.,
        (_, Item::ChatBubble { .. } | Item::ChatChips(_) | Item::ChatThought { .. } | Item::ChatTyping(_) | Item::ChatFiles(_)) => 8.,
        (_, Item::ChatTidied) => 18.,
        _ => return None,
    })
}

/// How far a Chat reply sits in from the column's edge: past the speaker's duck.
const INDENT: f32 = 30.;
/// A bubble's corners, the one nearest the duck tucked in.
const RADIUS: f32 = 18.;

/// Draws a Chat item, or `None` for one the Code transcript draws.
pub fn item(item: &Item, theme: &Theme, handlers: &Handlers, still: bool) -> Option<AnyElement> {
    Some(match item {
        Item::ChatDivider(at) => div()
            .flex()
            .justify_center()
            .child(div().typography(Type::Small).text_color(theme.text_faint).child(divider_label(*at, crate::chat::now_millis())))
            .into_any_element(),
        Item::ChatSpeaker { live } => speaker(*live, still, theme).into_any_element(),
        Item::ChatBubble { blocks, copy } => div()
            .pl(px(INDENT))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(6.))
            .child(
                div()
                    .max_w(relative(0.92))
                    .px(px(16.))
                    .py(px(11.))
                    .rounded_tl(px(6.))
                    .rounded_tr(px(RADIUS))
                    .rounded_bl(px(RADIUS))
                    .rounded_br(px(RADIUS))
                    .bg(theme.hover)
                    .child(crate::transcript::reply(blocks, theme, handlers)),
            )
            .when_some(copy.clone(), |column, copy| column.child(copy_button(copy, theme)))
            .into_any_element(),
        Item::ChatThought { key, text, open } => {
            let toggle = handlers.toggle.clone();
            let click_key = key.clone();
            div()
                .pl(px(INDENT))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .id(ElementId::Name(format!("{key}-toggle").into()))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| toggle(click_key.clone(), window, cx))
                        .child(Icon::Bulb.view(px(12.), theme.text_faint))
                        .child(div().typography(Type::Small).text_color(theme.text_dim).child("Thought it over"))
                        .child(if *open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))
                        .tag("chat-thought"),
                )
                .when(*open, |column| {
                    column.child(
                        div()
                            .ml(px(5.))
                            .pl(px(12.))
                            .border_l_1()
                            .border_color(theme.border_strong)
                            .typography(Type::Small)
                            .text_color(theme.text_dim)
                            .child(text.clone()),
                    )
                })
                .into_any_element()
        }
        Item::ChatChips(chips) => {
            let mut row = div().pl(px(INDENT)).flex().flex_wrap().gap(px(6.));
            for chip in chips {
                match chip {
                    Chip::Single(activity) => row = row.child(activity_chip(activity, still, theme)),
                    Chip::Fold { key, label, open, activities } => {
                        let toggle = handlers.toggle.clone();
                        let click_key = key.clone();
                        row = row.child(
                            chip_frame(theme)
                                .id(ElementId::Name(format!("{key}-chip").into()))
                                .cursor_pointer()
                                .hover(|chip| chip.bg(theme.border))
                                .on_click(move |_, window, cx| toggle(click_key.clone(), window, cx))
                                .child(Icon::Check.view(px(11.), theme.text_faint))
                                .child(div().typography(Type::Small).text_color(theme.text_soft).child(label.clone()))
                                .child(if *open { Icon::ChevronDown } else { Icon::ChevronRight }.view(px(10.), theme.text_faint))
                                .tag("chat-fold"),
                        );
                        if *open {
                            for activity in activities {
                                row = row.child(activity_chip(activity, still, theme));
                            }
                        }
                    }
                }
            }
            row.into_any_element()
        }
        Item::ChatTyping(mood) => div().pl(px(INDENT)).flex().child(typing(*mood, still, theme)).into_any_element(),
        Item::ChatFiles(files) => {
            let mut row = div().pl(px(INDENT)).flex().flex_wrap().gap(px(6.));
            for (at, file) in files.iter().enumerate() {
                let path = file.path.clone();
                let folder = (!file.folder.is_empty()).then(|| file.folder.clone());
                row = row.child(
                    chip_frame(theme)
                        .id(ElementId::Name(format!("scratch-file-{at}-{}", file.name).into()))
                        .cursor_pointer()
                        .border_1()
                        .border_color(theme.border)
                        .hover(|chip| chip.border_color(theme.border_strong))
                        .tooltip(crate::tooltip::tip("Show in Finder"))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::RevealScratchpadFile { path: path.clone() }), cx)
                        })
                        .child(Icon::File.view(px(11.), theme.accent_text))
                        .child(div().typography(Type::Small).text_color(theme.text_soft).child(file.name.clone()))
                        .when_some(folder, |chip, folder| chip.child(div().typography(Type::Small).text_color(theme.text_faint).child(folder)))
                        .tag("scratch-file"),
                );
            }
            row.into_any_element()
        }
        Item::ChatTidied => div()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .typography(Type::Small)
                    .text_color(theme.text_faint)
                    .child(Icon::Spark.view(px(11.), theme.text_faint))
                    .child("Tidied up the conversation"),
            )
            .into_any_element(),
        _ => return None,
    })
}

/// The duck and the agent's name over a reply; the duck paddles while it works.
fn speaker(live: bool, still: bool, theme: &Theme) -> impl IntoElement {
    let duck = div()
        .flex_none()
        .size(px(22.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.accent_wash)
        .border_1()
        .border_color(theme.accent_line)
        .child(Icon::Duck.view(px(12.), theme.accent_text));
    let duck = if live && !still { crate::motion::paddling(duck).into_any_element() } else { duck.into_any_element() };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(duck)
        .child(div().typography(Type::UiStrong).text_color(theme.text).child(AGENT_NAME))
        .tag("chat-speaker")
}

fn chip_frame(theme: &Theme) -> gpui::Div {
    div().h(px(26.)).px(px(10.)).rounded_full().bg(theme.hover).flex().items_center().gap(px(6.))
}

fn activity_chip(activity: &Activity, still: bool, theme: &Theme) -> AnyElement {
    let (icon_colour, words) = match activity.status {
        Status::Live => (theme.accent_text, theme.text),
        Status::Done => (theme.text_faint, theme.text_soft),
        Status::Failed => (theme.danger, theme.text_soft),
    };
    let icon = if activity.status == Status::Failed { Icon::Alert } else { activity.icon };
    let chip = chip_frame(theme)
        .child(icon.view(px(11.), icon_colour))
        .child(div().max_w(px(320.)).truncate().typography(Type::Small).text_color(words).child(activity.label.clone()));
    if activity.status == Status::Live && !still {
        chip.with_animation(
            ElementId::Name(format!("chip-live-{}", activity.label).into()),
            gpui::Animation::new(std::time::Duration::from_millis(1400)).repeat().with_easing(gpui::pulsating_between(0.55, 1.)),
            |chip, t| chip.opacity(t),
        )
        .into_any_element()
    } else {
        chip.into_any_element()
    }
}

/// The typing bubble: three dots that rise in turn, and what the duck is up to.
fn typing(mood: Mood, still: bool, theme: &Theme) -> impl IntoElement {
    let dots = div().flex().items_center().gap(px(4.)).children((0..3).map(|dot| {
        let dot_view = div().size(px(6.)).rounded_full().bg(theme.text_dim);
        if still {
            dot_view.opacity(0.7).into_any_element()
        } else {
            dot_view
                .with_animation(
                    ElementId::Name(format!("typing-dot-{dot}").into()),
                    gpui::Animation::new(std::time::Duration::from_millis(1200)).repeat(),
                    move |dot_view, t| {
                        // Each dot a third of a beat behind the last.
                        let phase = (t - dot as f32 / 3.).rem_euclid(1.);
                        let lift = (phase * std::f32::consts::TAU).sin().max(0.);
                        dot_view.opacity(0.35 + 0.65 * lift).mt(px(-3. * lift))
                    },
                )
                .into_any_element()
        }
    }));
    div()
        .h(px(36.))
        .px(px(14.))
        .rounded_tl(px(6.))
        .rounded_tr(px(RADIUS))
        .rounded_bl(px(RADIUS))
        .rounded_br(px(RADIUS))
        .bg(theme.hover)
        .flex()
        .items_center()
        .gap(px(10.))
        .child(dots)
        .when_some(mood.caption(), |bubble, caption| bubble.child(div().typography(Type::Small).text_color(theme.text_dim).child(caption)))
        .tag("chat-typing")
}

/// Copies a settled reply's words.
fn copy_button(text: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .id(ElementId::Name(format!("copy-reply-{}", text.len()).into()))
        .h(px(22.))
        .px(px(8.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(5.))
        .cursor_pointer()
        .hover(|button| button.bg(theme.hover))
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
        .child(Icon::Copy.view(px(11.), theme.text_faint))
        .child(div().typography(Type::Small).text_color(theme.text_faint).child("Copy"))
        .tag("copy-reply")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::conversation::ToolResult;

    fn tool(name: &str, input: Value, result: Option<(&str, bool)>) -> ToolStep {
        ToolStep {
            id: format!("{name}-id"),
            name: name.into(),
            input: Some(input),
            result: result.map(|(text, is_error)| ToolResult { text: text.into(), is_error, pictures: Vec::new() }),
            writing: None,
        }
    }

    #[test]
    fn calls_read_as_plain_words() {
        let words = |name: &str, input: Value, live: bool| label(&tool(name, input, None), live).0;
        assert_eq!(words("write", json!({ "path": "/s/notes/groceries.md" }), false), "Jotted down groceries.md");
        assert_eq!(words("write", json!({ "path": "groceries.md" }), true), "Jotting down groceries.md");
        assert_eq!(words("web_fetch", json!({ "url": "https://www.example.com/a?b" }), false), "Looked up example.com");
        assert_eq!(words("web_search", json!({ "query": "best   sourdough   starter" }), false), "Searched the web for “best sourdough starter”");
        assert_eq!(words("memory_save", json!({ "title": "Likes tea" }), false), "Remembered “Likes tea”");
        assert_eq!(words("mcp__notion__search_pages", json!({}), false), "Used search pages (notion)");
        assert_eq!(words("canvas", json!({}), true), "Drawing on a canvas");
        assert_eq!(words("browser_click_thing", json!({}), false), "Used the page");
        assert_eq!(words("something_new", json!({}), false), "Used something new");
        assert!(words("grep", json!({ "pattern": "a".repeat(40) }), false).ends_with("…”"));
    }

    #[test]
    fn settled_runs_fold_and_the_rest_show() {
        let done = |name: &str, path: &str| tool(name, json!({ "path": path, "url": "https://a.org/x" }), Some(("ok", false)));
        let tools = [done("write", "a.md"), done("write", "b.md"), done("web_fetch", ""), tool("read", json!({ "path": "c.md" }), Some(("no", true))), done("read", "d.md")];
        let refs: Vec<&ToolStep> = tools.iter().collect();
        let open = HashSet::new();
        let chips = chips(&refs, false, "turn-1", &open);
        assert_eq!(chips.len(), 3, "{chips:?}");
        match &chips[0] {
            Chip::Fold { label, activities, open, .. } => {
                assert_eq!(label.as_ref(), "Jotted down 2 files, looked up 1 page");
                assert_eq!(activities.len(), 3);
                assert!(!open);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&chips[1], Chip::Single(Activity { status: Status::Failed, .. })));
        assert!(matches!(&chips[2], Chip::Single(Activity { status: Status::Done, .. })), "two settled is not a run");

        let running = [done("write", "a.md"), done("read", "b.md"), tool("web_fetch", json!({ "url": "https://x.y" }), None), done("read", "c.md")];
        let refs: Vec<&ToolStep> = running.iter().collect();
        let live = super::chips(&refs, true, "t", &open);
        assert!(live.iter().all(|chip| matches!(chip, Chip::Single(_))), "a live call breaks the run");
        assert!(matches!(&live[2], Chip::Single(Activity { status: Status::Live, .. })));
    }

    #[test]
    fn a_fold_says_its_biggest_pieces_and_counts_the_rest() {
        let done = |name: &str, path: &str| tool(name, json!({ "path": path }), Some(("ok", false)));
        let run = [done("read", "a"), done("read", "b"), done("edit", "a"), done("web_fetch", ""), done("ls", ""), done("ls", ""), done("memory_recall", "")];
        let refs: Vec<&ToolStep> = run.iter().collect();
        assert_eq!(summarize_fold(&refs), "Updated a, looked up 1 page, +5 more");
        let same = [done("read", "a"), done("read", "a"), done("read", "a")];
        let refs: Vec<&ToolStep> = same.iter().collect();
        assert_eq!(summarize_fold(&refs), "Read 1 file", "one file read three times is one file");
    }

    #[test]
    fn only_files_written_inside_the_scratchpad_are_offered() {
        let root = Path::new("/data/scratch/chat-1");
        assert_eq!(resolve_in("notes/a.md", root), Some(root.join("notes/a.md")));
        assert_eq!(resolve_in("file:///data/scratch/chat-1/b.md", root), Some(root.join("b.md")));
        assert_eq!(resolve_in("../chat-2/x.md", root), None);
        assert_eq!(resolve_in("/data/scratch/chat-10/x.md", root), None, "a sibling with a longer name is outside");
        assert_eq!(resolve_in("~/x.md", root), None);
        assert_eq!(resolve_in(".", root), None, "the scratchpad itself is no file");
        let tools = [
            tool("write", json!({ "path": "notes/a.md" }), Some(("ok", false))),
            tool("edit", json!({ "path": "/data/scratch/chat-1/notes/a.md" }), Some(("ok", false))),
            tool("write", json!({ "path": "b.md" }), Some(("refused", true))),
            tool("write", json!({ "path": "../escape.md" }), Some(("ok", false))),
            tool("read", json!({ "path": "c.md" }), Some(("ok", false))),
        ];
        let refs: Vec<&ToolStep> = tools.iter().collect();
        let files = scratchpad_files(&refs, root);
        assert_eq!(files.len(), 1, "{files:?}");
        assert_eq!((files[0].name.as_ref(), files[0].folder.as_ref()), ("a.md", "notes"));
    }

    #[test]
    fn the_typing_bubble_says_what_the_duck_is_up_to() {
        let mut turn = Turn::default();
        turn.live = true;
        assert_eq!(mood(&turn), Some(Mood::Typing));
        turn.steps.push(Step::Thought("hmm".into()));
        assert_eq!(mood(&turn), Some(Mood::Thinking));
        turn.steps.push(Step::Tool(tool("web_fetch", json!({}), None)));
        assert_eq!(mood(&turn), Some(Mood::Using));
        turn.steps.push(Step::Event("Compacting".into()));
        assert_eq!(mood(&turn), Some(Mood::Tidying));
        turn.reply = "Here".into();
        assert_eq!(mood(&turn), None, "its words are coming in");
        turn.live = false;
        turn.reply.clear();
        assert_eq!(mood(&turn), None);
    }

    #[test]
    fn dividers_open_the_chat_and_every_long_pause() {
        let now = clock::start_of_day(1_790_000_000_000, 0) + 15 * 3_600_000;
        let mut conversation = Conversation::default();
        conversation.send("hi", now - 3 * 3_600_000);
        conversation.finish(&Ok(wack_core::Outcome::Done), now - 3 * 3_600_000 + 5_000);
        conversation.send("and again", now - 3 * 3_600_000 + 60_000);
        conversation.finish(&Ok(wack_core::Outcome::Done), now - 3 * 3_600_000 + 65_000);
        conversation.send("later", now);
        let dividers: Vec<usize> = (0..conversation.entries.len()).filter(|&index| divider_before(&conversation, index).is_some()).collect();
        assert_eq!(dividers, [0, 4], "the first message, and the one after hours of quiet");
        assert!(opens_cluster(&conversation, 1));
        assert!(divider_label(now - 60_000, now).starts_with("Today · "));
        assert!(divider_label(now - 86_400_000, now).starts_with("Yesterday · "));
        let old = divider_label(now - 9 * 86_400_000, now);
        assert!(clock::WEEKDAYS.iter().any(|day| old.starts_with(day)), "{old}");
    }

    #[test]
    fn building_a_chat_entry_by_entry_matches_building_it_whole() {
        // Events only change the last entry or add after it, so items built as a run goes must
        // be the items a fresh build of the finished conversation makes.
        let root = Path::new("/s");
        let open = HashSet::new();
        let whole = |conversation: &Conversation| {
            let mut cache = ReplyCache::default();
            let mut items = Vec::new();
            for index in 0..conversation.entries.len() {
                entry_items(conversation, index, root, &open, &mut cache, &mut items);
            }
            format!("{items:?}")
        };
        let mut conversation = Conversation::default();
        let mut cache = ReplyCache::default();
        let mut spans: Vec<Vec<Item>> = Vec::new();
        let events = [
            wack_core::Event::Thinking("Let me look.".into()),
            wack_core::Event::ToolStart { id: "1".into(), name: "write".into() },
            wack_core::Event::ToolCall { id: "1".into(), name: "write".into(), input: json!({ "path": "a.md", "content": "x" }) },
            wack_core::Event::ToolResult { id: "1".into(), name: "write".into(), content: "ok".into(), is_error: false, images: Vec::new() },
            wack_core::Event::Text("Done — ".into()),
            wack_core::Event::Text("saved.".into()),
        ];
        conversation.send("write a note", 1_000);
        let rebuild = |conversation: &Conversation, spans: &mut Vec<Vec<Item>>, cache: &mut ReplyCache| {
            let from = spans.len().saturating_sub(1);
            spans.truncate(from);
            for index in from..conversation.entries.len() {
                let mut fresh = Vec::new();
                entry_items(conversation, index, root, &open, cache, &mut fresh);
                spans.push(fresh);
            }
        };
        rebuild(&conversation, &mut spans, &mut cache);
        for event in &events {
            conversation.apply(event, 2_000);
            rebuild(&conversation, &mut spans, &mut cache);
            assert_eq!(format!("{:?}", spans.concat()), whole(&conversation));
        }
        conversation.finish(&Ok(wack_core::Outcome::Done), 3_000);
        rebuild(&conversation, &mut spans, &mut cache);
        let items = spans.concat();
        assert_eq!(format!("{items:?}"), whole(&conversation));
        assert!(items.iter().any(|item| matches!(item, Item::ChatFiles(files) if files[0].name.as_ref() == "a.md")), "{items:?}");
        assert!(items.iter().any(|item| matches!(item, Item::ChatBubble { copy: Some(_), .. })));
    }
}
