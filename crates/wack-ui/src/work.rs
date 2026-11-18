//! Turns a [`Conversation`] into the rows the transcript draws: what each tool call says in the
//! work log, the diff an edit opens into, how long a turn took. Pure, so it is tested here.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    rc::Rc,
};

use serde_json::Value;
use wack_core::agents::{AGENT, AGENT_MESSAGE, AgentInfo, Status, StoppedBy};

use crate::{
    conversation::{Conversation, Entry, News, Step, ToolStep, Turn, TurnEnd},
    markdown,
    suggestions,
    transcript::{AgentLine, AgentState, CommandShown, Detail, Diff, DiffLine, Item, LineKind, Meta, Target, Work, WorkRow},
};

/// Diff and output details show at most this many lines.
const DETAIL_LINES: usize = 60;
/// Unchanged lines kept around an edit's change.
const CONTEXT_LINES: usize = 2;

/// Parsed replies, so a chat is not parsed again every time it is rebuilt. Keyed by entry index
/// and checked against the whole text: a reply can be moved into the work log and replaced by
/// one of the same length.
#[derive(Default)]
pub struct ReplyCache {
    parsed: Vec<Option<(String, Rc<Vec<markdown::Block>>)>>,
}

impl ReplyCache {
    fn get(&mut self, index: usize, text: &str) -> Rc<Vec<markdown::Block>> {
        if self.parsed.len() <= index {
            self.parsed.resize(index + 1, None);
        }
        match &self.parsed[index] {
            Some((parsed, blocks)) if parsed == text => blocks.clone(),
            _ => {
                let blocks = Rc::new(markdown::parse(text));
                self.parsed[index] = Some((text.to_string(), blocks.clone()));
                blocks
            }
        }
    }
}

/// Every transcript item for a conversation. `open` holds the keys of expanded rows and work
/// logs; a live turn's log is always open.
pub fn items(conversation: &Conversation, cwd: &Path, open: &HashSet<String>, now: u64, cache: &mut ReplyCache) -> Vec<Item> {
    let mut items = Vec::new();
    for index in 0..conversation.entries.len() {
        entry_items(conversation, index, cwd, open, now, cache, &mut items);
    }
    items
}

/// The items for one entry, appended to `items`. A chat rebuilds only the entries that changed.
pub fn entry_items(
    conversation: &Conversation,
    index: usize,
    cwd: &Path,
    open: &HashSet<String>,
    now: u64,
    cache: &mut ReplyCache,
    items: &mut Vec<Item>,
) {
    let Some(entry) = conversation.entries.get(index) else {
        return;
    };
    {
        match entry {
            Entry::Prompt { text, steered, command, suggestion, pictures } => {
                let key = format!("prompt-{index}");
                items.push(Item::Prompt {
                    text: text.clone().into(),
                    steered: *steered,
                    command: command.as_ref().map(|command| CommandShown {
                        name: format!("/{}", command.name).into(),
                        args: command.args.clone().into(),
                        open: open.contains(&key),
                        key: key.into(),
                    }),
                    suggestion: suggestion.as_ref().map(|started| suggestions::sent_label(started.origin.as_ref())),
                    pictures: pictures.clone(),
                })
            }
            Entry::Compacted => items.push(Item::Compacted),
            Entry::Notice { text, failed } => items.push(Item::Notice { text: text.clone().into(), failed: *failed }),
            Entry::News(news) => {
                let mut row = news_row(news, &format!("news-{index}"));
                row.open = row.detail.is_some() && open.contains(row.key.as_ref());
                items.push(Item::News(row));
            }
            Entry::Turn(turn) => {
                let key = format!("turn-{index}");
                if !turn.steps.is_empty() || turn.live {
                    let rows: Vec<WorkRow> = turn
                        .steps
                        .iter()
                        .enumerate()
                        .map(|(step, item)| step_row(item, &format!("{key}-{step}"), cwd, turn.live, open, &conversation.agents, now))
                        .collect();
                    items.push(Item::Work(Work {
                        open: turn.live || open.contains(&key),
                        summary: summary(turn, now).into(),
                        tally: tally(&rows).into(),
                        live: turn.live,
                        rows,
                        key: key.clone().into(),
                    }));
                }
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
                // A plan proposed in prose shows as the card, not twice.
                let reply = match &turn.proposal {
                    Some(_) => without_proposed_plan(&turn.reply),
                    None => turn.reply.clone(),
                };
                if !reply.trim().is_empty() {
                    let blocks = cache.get(index, &reply);
                    let named = crate::pictures::referenced(&blocks);
                    items.push(Item::Reply(blocks));
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
                if let Some(proposal) = &turn.proposal {
                    let current = conversation.awaiting == Some(index);
                    items.push(Item::Plan(crate::plan_card::PlanCard {
                        key: format!("plan-{index}").into(),
                        blocks: Rc::new(markdown::parse(&proposal.text)),
                        ultra: proposal.ultra,
                        current,
                        confirming_discard: current && conversation.plan_view.confirming_discard,
                        notice: current.then(|| conversation.plan_view.notice.clone().map(Into::into)).flatten(),
                    }));
                }
                // The follow-ups a settled turn filed: cards wait for the run, so they never
                // appear above a reply still being written.
                if !turn.live {
                    let view = &conversation.suggestion_view;
                    let cards: Vec<crate::suggestion::Item> = turn
                        .steps
                        .iter()
                        .filter_map(|step| match step {
                            Step::Tool(tool) => suggestions::card_from_step(tool),
                            _ => None,
                        })
                        .map(|card| {
                            let state = if view.dismissed.contains(&card.id) {
                                crate::suggestion::State::Dismissed
                            } else if let Some((chat, name)) = view.links.get(&card.id) {
                                crate::suggestion::State::OpenedIn { chat: chat.clone(), name: name.clone().into() }
                            } else if conversation.started.contains(&card.id) {
                                crate::suggestion::State::StartedHere
                            } else if view.queued.contains(&card.id) {
                                crate::suggestion::State::Queued
                            } else {
                                crate::suggestion::State::Open
                            };
                            crate::suggestion::Item::new(card, state, open)
                        })
                        .collect();
                    if !cards.is_empty() {
                        items.push(Item::Suggestions(crate::suggestion::Tray { cards }));
                    }
                }
                for notice in &turn.notices {
                    items.push(Item::Notice { text: notice.clone().into(), failed: false });
                }
                match &turn.end {
                    Some(TurnEnd::Stopped) => items.push(Item::Notice { text: "Stopped.".into(), failed: false }),
                    Some(TurnEnd::CutOff) => items.push(Item::Notice {
                        text: "The reply reached its length limit before it was finished.".into(),
                        failed: true,
                    }),
                    Some(TurnEnd::Refused) => {
                        items.push(Item::Notice { text: "The model declined to continue.".into(), failed: true })
                    }
                    Some(TurnEnd::Failed(message)) => items.push(Item::Notice { text: message.clone().into(), failed: true }),
                    Some(TurnEnd::Done) | None => {}
                }
                if !turn.live && let Some(footer) = footer(turn) {
                    items.push(Item::Footer(footer.into()));
                }
            }
        }
    }
}

/// "Working · 8s", "Worked for 19s", or "Worked" when the times were not saved.
pub fn summary(turn: &Turn, now: u64) -> String {
    if turn.live {
        if let Some(retrying) = &turn.retrying {
            return retrying.clone();
        }
        return match turn.started {
            Some(started) => format!("Working  ·  {}", duration(now.saturating_sub(started))),
            None => "Working".into(),
        };
    }
    match (turn.started, turn.ended) {
        (Some(started), Some(ended)) if ended > started => format!("Worked for {}", duration(ended - started)),
        _ => "Worked".into(),
    }
}

/// What a log did, counted: "2 reads · 1 search · 1 edit · 1 command". Thoughts, notes and
/// smaller tools are left out; empty when nothing counted.
pub fn tally(rows: &[WorkRow]) -> String {
    const KINDS: [(&[&str], &str, &str); 12] = [
        (&[AGENT_VERB], "agent", "agents"),
        (&["Drew"], "drawing", "drawings"),
        (&BROWSER_VERBS, "browser step", "browser steps"),
        (&["Used skill"], "skill", "skills"),
        (&["Read"], "read", "reads"),
        (&["Fetched"], "page", "pages"),
        (&["Searched", "Searched the web", "Found", "Listed", "Looked for"], "search", "searches"),
        (&["Edited", "Wrote"], "edit", "edits"),
        (&["Ran", "Watching"], "command", "commands"),
        (&["Asked"], "question", "questions"),
        (&["Used"], "MCP call", "MCP calls"),
        (&["Saved memory", "Updated memory", "Recalled memory", "Forgot memory"], "memory note", "memory notes"),
    ];
    KINDS
        .iter()
        .filter_map(|(verbs, one, many)| {
            let count = rows.iter().filter(|row| verbs.contains(&row.verb.as_ref())).count();
            (count > 0).then(|| format!("{count} {}", if count == 1 { one } else { many }))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What a live log is doing now, for the status pill over the composer: its unfinished step in
/// the present tense, with a path cut to its file ("Editing retry.rs", "Running cargo test").
/// `None` between steps.
pub fn activity(rows: &[WorkRow]) -> Option<String> {
    let row = rows.iter().rev().find(|row| matches!(row.meta, Meta::Running))?;
    let verb = match row.verb.as_ref() {
        "Read" => "Reading",
        "Fetched" => "Fetching",
        "Wrote" => "Writing",
        "Edited" => "Editing",
        "Ran" => "Running",
        "Searched" => "Searching",
        "Searched the web" => "Searching the web for",
        "Found" => "Finding",
        "Listed" => "Listing",
        "Looked for" => "Looking for",
        "Asked" => "Asking",
        "Used skill" => "Using skill",
        "Used" => "Using",
        "Saved memory" => "Saving memory",
        "Updated memory" => "Updating memory",
        "Recalled memory" => "Recalling memory",
        "Forgot memory" => "Forgetting memory",
        "Waited" => "Waiting",
        "Awaited" => "Awaiting",
        "Halted" => "Halting",
        "Outlined" => "Outlining",
        "Extracted" => "Extracting",
        "Evaluated" => "Evaluating",
        "Batched" => "Running",
        "Previewed" => "Starting",
        "Browsed" => "Browsing",
        AGENT_VERB => "Waiting on",
        "Stopped" => "Stopping",
        // The browser's steps (see `browser_words`).
        "Opened" => "Opening",
        "Went" => "Going",
        "Reloaded" => "Reloading",
        "Switched" => "Switching",
        "Closed" => "Closing",
        "Looked at" => "Looking at",
        "Clicked" => "Clicking",
        "Hovered" => "Hovering",
        "Dragged" => "Dragging",
        "Typed" => "Typing",
        "Pressed" => "Pressing",
        "Scrolled" => "Scrolling",
        "Filled" => "Filling",
        "Resized" => "Resizing",
        "Checked" => "Checking",
        "Answered" => "Answering",
        "Uploaded" => "Uploading",
        other => other,
    };
    let target = match &row.target {
        Target::Code(text) if row.verb.as_ref() == "Fetched" => text.split('/').next().unwrap_or(text).to_string(),
        Target::Code(text) if row.verb.as_ref() != "Ran" => text.rsplit('/').next().unwrap_or(text).to_string(),
        Target::Code(text) | Target::Words(text) | Target::Note(text) => text.to_string(),
    };
    Some(if target.is_empty() { verb.to_string() } else { format!("{verb} {target}") })
}

/// A web search's results as "Title — site" lines, read back from the tool's text.
fn search_results(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((number, title)) = line.split_once(". ") else { continue };
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let host = lines
            .peek()
            .map(|next| next.trim())
            .filter(|next| next.starts_with("http"))
            .and_then(|url| url.split('/').nth(2))
            .map(|host| host.trim_start_matches("www.").to_string());
        results.push(match host {
            Some(host) => format!("{title} — {host}"),
            None => title.to_string(),
        });
    }
    results
}

/// The facts under a finished turn: how long, how many tokens.
fn footer(turn: &Turn) -> Option<String> {
    let mut parts = Vec::new();
    // Under a second is not worth saying: it is a turn that failed at once.
    if let (Some(started), Some(ended)) = (turn.started, turn.ended)
        && ended >= started + 1000
    {
        parts.push(duration(ended - started));
    }
    let tokens = turn.usage.input + turn.usage.cache_read + turn.usage.cache_write;
    if tokens > 0 || turn.usage.output > 0 {
        parts.push(format!("{} in", count(tokens)));
        parts.push(format!("{} out", count(turn.usage.output)));
    }
    (!parts.is_empty()).then(|| parts.join("  ·  "))
}

/// "8s", "1m 05s", "1h 02m".
pub fn duration(millis: u64) -> String {
    let seconds = millis / 1000;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// "812", "12.4k", "1.2M".
pub fn count(value: u64) -> String {
    match value {
        0..1000 => value.to_string(),
        1000..1_000_000 => format!("{:.1}k", value as f64 / 1000.).replace(".0k", "k"),
        _ => format!("{:.1}M", value as f64 / 1_000_000.).replace(".0M", "M"),
    }
}

fn step_row(step: &Step, key: &str, cwd: &Path, live: bool, open: &HashSet<String>, agents: &HashMap<String, AgentInfo>, now: u64) -> WorkRow {
    let mut row = match step {
        Step::Thought(text) => WorkRow {
            key: key.to_string().into(),
            verb: "Thought".into(),
            target: Target::Words(first_line(text).into()),
            meta: Meta::None,
            detail: (text.trim().contains('\n') || text.trim().len() > 120).then(|| Detail::Text(text.trim().to_string().into())),
            open: false,
            agent: None,
            pictures: Vec::new(),
        },
        Step::Note(text) => WorkRow {
            key: key.to_string().into(),
            verb: "".into(),
            target: Target::Note(text.clone().into()),
            meta: Meta::None,
            detail: None,
            open: false,
            agent: None,
            pictures: Vec::new(),
        },
        Step::Event(text) => WorkRow {
            key: key.to_string().into(),
            verb: "".into(),
            target: Target::Words(text.clone().into()),
            meta: Meta::None,
            detail: None,
            open: false,
            agent: None,
            pictures: Vec::new(),
        },
        Step::Tool(tool) if is_agent_row(tool, agents) => agent_row(tool, agents.get(&tool.id), live, now),
        Step::Tool(tool) => tool_row(tool, cwd, live),
        Step::Task(news) => news_row(news, key),
    };
    row.open = row.detail.is_some() && open.contains(row.key.as_ref());
    row
}

/// How a `bash` result says the command carried on in the background, if it did.
fn background_note(result: &str) -> Option<&'static str> {
    if result.starts_with("Started in the background (ID:") {
        Some("in background")
    } else if result.starts_with("Command did not complete within its") {
        Some("timed out · in background")
    } else if result.starts_with("The user moved this command to the background") {
        Some("moved to background")
    } else {
        None
    }
}

/// A background task's news as a row: what happened to which command, with the lines it quoted.
pub fn news_row(news: &News, key: &str) -> WorkRow {
    if let Some(id) = news.agent() {
        return agent_news_row(news, id, key);
    }
    let first = first_line(&news.text);
    // The command is quoted in backticks; a watch's news names its description in brackets.
    let subject = first.split('`').nth(1).map(str::to_string);
    let watched = || first.split_once('(').and_then(|(_, rest)| rest.split_once(')')).map(|(name, _)| name.to_string());
    let after = first.rsplit_once(" after ").map(|(_, rest)| rest.trim_end_matches('.').split(['.', ' ']).next().unwrap_or("").to_string());
    let lines: Vec<&str> = {
        let mut inside = false;
        news.text
            .lines()
            .filter(|line| {
                if line.starts_with("```") {
                    inside = !inside;
                    return false;
                }
                inside
            })
            .collect()
    };
    let detail = (!lines.is_empty()).then(|| Detail::Output(tail(&lines.join("\n"), DETAIL_LINES).into()));
    let with_time = |label: String| match &after {
        Some(after) if !after.is_empty() => format!("{label} · {after}"),
        _ => label,
    };
    let (verb, target, meta) = match news.what() {
        "lines" => ("Watch", Target::Words(watched().unwrap_or_default().into()), Meta::Note(plural(lines.len(), "line", "lines").into())),
        "lost" => ("Lost", Target::Code(subject.unwrap_or_default().into()), Meta::Note("stopped when WackCode quit".into())),
        _ => {
            let target = Target::Code(subject.unwrap_or_default().into());
            if first.contains(") finished: exit code 0") {
                ("Finished", target, Meta::Success(with_time("in background".into()).into()))
            } else if let Some(code) = first.split("failed: exit code ").nth(1) {
                let code = code.split(' ').next().unwrap_or("");
                ("Failed", target, Meta::Failed(with_time(format!("exit {code}")).into()))
            } else if first.contains("was killed by signal") {
                ("Failed", target, Meta::Failed(with_time("killed".into()).into()))
            } else if first.contains("stopped by the user") {
                ("Stopped", target, Meta::Note(with_time("by you".into()).into()))
            } else if first.contains("time limit") {
                ("Ended", target, Meta::Note("time limit".into()))
            } else if first.contains("printed so often") {
                ("Stopped", target, Meta::Failed("too noisy".into()))
            } else {
                ("Ended", target, Meta::Note(with_time("in background".into()).into()))
            }
        }
    };
    WorkRow { key: key.to_string().into(), verb: verb.into(), target, meta, detail, open: false, agent: None, pictures: Vec::new() }
}

fn first_line(text: &str) -> String {
    text.trim().lines().next().unwrap_or("").trim().to_string()
}

/// The skill a `…/<skill>/SKILL.md` path loads.
fn skill_folder(path: &str) -> Option<String> {
    let path = Path::new(path);
    (path.file_name()? == "SKILL.md").then(|| path.parent()?.file_name().map(|name| name.to_string_lossy().into_owned()))?
}

/// A path as the user thinks of it: relative to the project when inside it.
fn shown_path(path: &str, cwd: &Path) -> String {
    Path::new(path).strip_prefix(cwd).map(|relative| relative.display().to_string()).unwrap_or_else(|_| path.to_string())
}

fn text_field<'a>(input: &'a Option<Value>, name: &str) -> Option<&'a str> {
    input.as_ref()?.get(name)?.as_str()
}

/// Lines of a tool's text result, leaving out the notes tools add in brackets.
fn result_lines(text: &str) -> usize {
    text.lines().filter(|line| !(line.starts_with('[') && line.ends_with(".]"))).filter(|line| !line.is_empty()).count()
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn tool_row(tool: &ToolStep, cwd: &Path, live: bool) -> WorkRow {
    let input = &tool.input;
    let path = |name: &str| text_field(input, name).map(|path| shown_path(path, cwd));
    let result = tool.result.as_ref();
    let ok = result.filter(|result| !result.is_error).map(|result| result.text.as_str());

    let (verb, target, success_meta, detail): (&str, Target, Option<String>, Option<Detail>) = match tool.name.as_str() {
        // Loading a skill is the agent choosing to follow it, so it reads as that, named by its
        // folder. The path's shape decides, not the skills on disk now, so an old chat still reads
        // right after a skill is renamed or removed.
        "read" if text_field(input, "path").and_then(skill_folder).is_some() => (
            "Used skill",
            Target::Words(text_field(input, "path").and_then(skill_folder).unwrap_or_default().into()),
            None,
            None,
        ),
        "read" => (
            "Read",
            Target::Code(path("path").unwrap_or_default().into()),
            ok.map(|text| if text.starts_with("(The file is empty") { "empty".into() } else { plural(result_lines(text), "line", "lines") }),
            None,
        ),
        "write" => {
            let content = text_field(input, "content").unwrap_or("");
            let lines: Vec<DiffLine> = content.lines().map(|line| DiffLine { kind: LineKind::Added, number: None, text: line.to_string().into() }).collect();
            let added = lines.len();
            (
                "Wrote",
                Target::Code(path("path").unwrap_or_default().into()),
                None,
                (!lines.is_empty()).then(|| Detail::Diff(Diff { path: path("path").unwrap_or_default().into(), lines: cap(lines), added, removed: 0 })),
            )
        }
        "edit" => {
            let mut lines = Vec::new();
            for (index, change) in input.as_ref().and_then(|input| input.get("edits")).and_then(Value::as_array).into_iter().flatten().enumerate() {
                if index > 0 {
                    lines.push(DiffLine { kind: LineKind::Gap, number: None, text: "".into() });
                }
                let old = change.get("old_text").and_then(Value::as_str).unwrap_or("");
                let new = change.get("new_text").and_then(Value::as_str).unwrap_or("");
                lines.extend(line_diff(old, new));
            }
            let added = lines.iter().filter(|line| line.kind == LineKind::Added).count();
            let removed = lines.iter().filter(|line| line.kind == LineKind::Removed).count();
            (
                "Edited",
                Target::Code(path("path").unwrap_or_default().into()),
                None,
                (!lines.is_empty()).then(|| Detail::Diff(Diff { path: path("path").unwrap_or_default().into(), lines: cap(lines), added, removed })),
            )
        }
        "bash" => {
            let command = text_field(input, "command").unwrap_or("");
            let described = text_field(input, "description").map(str::trim).filter(|text| !text.is_empty());
            let target = match described {
                Some(description) => Target::Words(description.to_string().into()),
                None => Target::Code(first_line(command).into()),
            };
            // A command that carried on in the background: its result was written for the model.
            if let Some(how) = ok.and_then(background_note) {
                ("Ran", target, Some(how.to_string()), Some(Detail::Output(format!("$ {}", command.trim()).into())))
            } else {
                let output = result.map(|result| result.text.trim_end().to_string()).filter(|text| !text.is_empty() && text != "(No output.)");
                // With a description as the subject, the command heads its output.
                let detail = match (described, output) {
                    (Some(_), Some(output)) => Some(format!("$ {}\n{}", command.trim(), tail(&output, DETAIL_LINES))),
                    (Some(_), None) => Some(format!("$ {}", command.trim())),
                    (None, output) => output.map(|output| tail(&output, DETAIL_LINES)),
                };
                ("Ran", target, None, detail.map(|text| Detail::Output(text.into())))
            }
        }
        wack_core::tasks::WATCH => {
            let command = text_field(input, "command").unwrap_or("");
            let target = match text_field(input, "description").map(str::trim).filter(|text| !text.is_empty()) {
                Some(description) => Target::Words(description.to_string().into()),
                None => Target::Code(first_line(command).into()),
            };
            ("Watching", target, Some("in background".into()), Some(Detail::Output(format!("$ {}", command.trim()).into())))
        }
        wack_core::tasks::STOP_TASK => {
            // "Stopped task abc (`npm run dev`)." names the command.
            let command = ok.and_then(|text| text.split('`').nth(1)).map(str::to_string);
            let target = match command {
                Some(command) => Target::Code(command.into()),
                None => Target::Code(text_field(input, "id").unwrap_or("").to_string().into()),
            };
            ("Stopped", target, None, None)
        }
        wack_core::tasks::WAIT_FOR_TASKS => (
            "Waited",
            Target::Words("for background commands".into()),
            ok.map(|text| if text.starts_with("Nothing reported") { "nothing yet".into() } else { "news arrived".into() }),
            ok.map(|text| Detail::Text(text.to_string().into())),
        ),
        "grep" => {
            let pattern = text_field(input, "pattern").unwrap_or("");
            let within = path("path").filter(|path| !path.is_empty() && path != ".").map(|path| format!(" in {path}")).unwrap_or_default();
            (
                "Searched",
                Target::Words(format!("“{pattern}”{within}").into()),
                ok.map(|text| if text.starts_with("No matches") { "no matches".into() } else { plural(result_lines(text), "line", "lines") }),
                ok.filter(|text| !text.starts_with("No matches")).map(|text| Detail::Output(tail(text, DETAIL_LINES).into())),
            )
        }
        "find" => (
            "Found",
            Target::Code(text_field(input, "pattern").unwrap_or("").to_string().into()),
            ok.map(|text| if text.starts_with("Nothing found") { "nothing".into() } else { plural(result_lines(text), "match", "matches") }),
            ok.filter(|text| !text.starts_with("Nothing found")).map(|text| Detail::Output(tail(text, DETAIL_LINES).into())),
        ),
        "ls" => (
            "Listed",
            Target::Code(path("path").filter(|path| !path.is_empty()).unwrap_or_else(|| ".".into()).into()),
            ok.map(|text| if text.starts_with("(The folder is empty") { "empty".into() } else { plural(result_lines(text), "entry", "entries") }),
            ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())),
        ),
        // A page is named by where it is, without the scheme; the result's title says what it was.
        wack_core::WEB_FETCH => {
            let url = text_field(input, "url").unwrap_or("").trim();
            let shown = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url).trim_end_matches('/');
            (
                "Fetched",
                Target::Code(shown.to_string().into()),
                ok.and_then(|text| text.lines().find_map(|line| line.strip_prefix("Title: ")).map(|title| title.trim().to_string())),
                None,
            )
        }
        // A search is named by its words; its detail is what came back, by title and site.
        wack_core::WEB_SEARCH => {
            let query = text_field(input, "query").unwrap_or("").trim();
            let within = text_field(input, "site").filter(|site| !site.trim().is_empty()).map(|site| format!(" on {}", site.trim())).unwrap_or_default();
            let results = ok.map(search_results).unwrap_or_default();
            (
                "Searched the web",
                Target::Words(format!("“{query}”{within}").into()),
                ok.map(|_| if results.is_empty() { "no results".into() } else { plural(results.len(), "result", "results") }),
                (!results.is_empty()).then(|| Detail::Text(results.join("\n").into())),
            )
        }
        wack_core::TOOL_SEARCH => (
            "Looked for",
            Target::Words(format!("tools for “{}”", text_field(input, "query").unwrap_or("")).into()),
            None,
            None,
        ),
        wack_core::ask::ASK_USER_QUESTION => {
            let questions = input.as_ref().and_then(|input| wack_core::ask::normalize(input).ok()).unwrap_or_default();
            let answers = ok.and_then(|text| wack_core::ask::parse_answers(&questions, text));
            let target = match questions.as_slice() {
                [] => String::new(),
                [only] => only.header.clone(),
                [first, rest @ ..] => format!("{} and {} more", first.header, rest.len()),
            };
            let outcome = ok.map(|text| {
                if answers.is_some() {
                    "answered"
                } else if text.contains("stop answering questions") {
                    "wrapped up"
                } else {
                    "no answer"
                }
            });
            (
                "Asked",
                Target::Words(target.into()),
                outcome.map(Into::into),
                (!questions.is_empty()).then(|| Detail::Asked(crate::question::asked(&questions, answers.as_deref()))),
            )
        }
        wack_core::todo::TODO => ("Todo", Target::Words(todo_words(input.as_ref()).into()), None, None),
        wack_core::plan::PLAN_MODE_ENTER => {
            let ultra = input.as_ref().and_then(|input| input.get("mode")).and_then(Value::as_str) == Some("ultraplan");
            ("Plan", Target::Words(if ultra { "switched to Ultra Plan" } else { "switched to Plan mode" }.into()), None, None)
        }
        wack_core::plan::PLAN_MODE_COMPLETE => ("Plan", Target::Words("submitted for review".into()), None, None),
        wack_core::memory::MEMORY_SAVE => (
            if text_field(input, "name").is_some_and(|name| !name.trim().is_empty()) { "Updated memory" } else { "Saved memory" },
            Target::Words(text_field(input, "title").unwrap_or("").into()),
            None,
            text_field(input, "body").map(|body| Detail::Text(body.to_string().into())),
        ),
        wack_core::memory::MEMORY_RECALL => {
            let names: Vec<&str> = input
                .as_ref()
                .and_then(|input| input.get("names"))
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            ("Recalled memory", Target::Code(names.join(", ").into()), None, ok.map(|text| Detail::Text(text.to_string().into())))
        }
        wack_core::memory::MEMORY_FORGET => ("Forgot memory", Target::Code(text_field(input, "name").unwrap_or("").into()), None, None),
        // The brief opens with the row; the card under the turn's answer is what the user acts on.
        wack_core::suggest::SUGGEST_TASK => (
            "Suggest",
            Target::Words(text_field(input, "title").unwrap_or("").into()),
            None,
            text_field(input, "description").map(|description| Detail::Text(description.to_string().into())),
        ),
        // "Drew “Settings” · 2 new, 1 revised": the card under the log shows the pictures.
        wack_canvas::tools::CANVAS => {
            let (target, note) = canvas_words(input.as_ref(), ok);
            ("Drew", Target::Words(target.into()), note, ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())))
        }
        wack_canvas::tools::CANVAS_LOOK => {
            let names = string_list(input.as_ref(), "artboards");
            let target = if names.is_empty() { "the canvas".to_string() } else { names.join(", ") };
            ("Viewed", Target::Words(target.into()), None, ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())))
        }
        wack_canvas::tools::CANVAS_GUIDE => {
            let sections = string_list(input.as_ref(), "sections");
            let target = if sections.is_empty() { "the canvas guide".to_string() } else { format!("the canvas guide · {}", sections.join(", ")) };
            ("Studied", Target::Words(target.into()), None, None)
        }
        // An MCP tool is named by its server and its own name: `mcp__github__create_issue` reads
        // "Used github · create_issue". What it returned is its detail.
        name if mcp_parts(name).is_some() => {
            let (server, tool) = mcp_parts(name).unwrap_or_default();
            ("Used", Target::Words(format!("{server} · {tool}").into()), None, ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())))
        }
        // "Clicked ref_7", "Opened localhost:5173": the browser's steps read as what was done.
        name if name.starts_with("browser_") => {
            let (verb, target) = browser_words(name, input.as_ref());
            (verb, Target::Words(target.into()), None, ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())))
        }
        other => (
            other,
            Target::Code(input.as_ref().map(|input| input.to_string()).unwrap_or_default().into()),
            None,
            ok.map(|text| Detail::Output(tail(text, DETAIL_LINES).into())),
        ),
    };

    let (meta, detail) = match result {
        None if live => (Meta::Running, detail),
        None => (Meta::None, detail),
        Some(result) if result.is_error => {
            // A failed command keeps its output; anything else shows why it failed.
            let detail = if tool.name == "bash" { detail } else { Some(Detail::Output(tail(&result.text, DETAIL_LINES).into())) };
            (Meta::Failed("failed".into()), detail)
        }
        Some(_) => match &detail {
            Some(Detail::Diff(diff)) => (Meta::Lines { added: diff.added as u32, removed: diff.removed as u32 }, detail),
            _ => (success_meta.map(|text| Meta::Note(text.into())).unwrap_or(Meta::None), detail),
        },
    };
    WorkRow { key: tool.id.clone().into(), verb: verb.to_string().into(), target, meta, detail, open: false, agent: None, pictures: result.map(|result| result.pictures.clone()).unwrap_or_default() }
}

/// The verbs canvas rows take, none shared with another tool's, so the icon and tally can tell
/// them by verb alone.
pub const CANVAS_VERBS: [&str; 3] = ["Drew", "Viewed", "Studied"];

/// A `canvas` call in words: what it drew on, and once it is done, how many artboards to fix.
fn canvas_words(input: Option<&Value>, result: Option<&str>) -> (String, Option<String>) {
    if let Some(outcome) = result.and_then(wack_canvas::tools::outcome) {
        let mut parts = vec![format!("“{}”", outcome.title)];
        for (count, word) in [(outcome.added.len(), "new"), (outcome.revised.len(), "revised"), (outcome.removed.len(), "removed")] {
            if count > 0 {
                parts.push(format!("{count} {word}"));
            }
        }
        let note = (!outcome.problems.is_empty()).then(|| format!("{} to fix", outcome.problems.len()));
        return (parts.join(" · "), note);
    }
    let title = input.and_then(|input| input.get("title")).and_then(Value::as_str).map(|t| format!("“{}”", t.trim()));
    let canvas = input.and_then(|input| input.get("canvas")).and_then(Value::as_str).map(str::to_string);
    let count = input.and_then(|input| input.get("artboards")).and_then(Value::as_array).map_or(0, Vec::len);
    let mut parts: Vec<String> = title.or(canvas).into_iter().collect();
    if count > 0 {
        parts.push(format!("{count} artboard{}", if count == 1 { "" } else { "s" }));
    }
    (if parts.is_empty() { "the canvas".into() } else { parts.join(" · ") }, None)
}

/// The strings in an input's array field (names, ids), others skipped.
fn string_list(input: Option<&Value>, key: &str) -> Vec<String> {
    input
        .and_then(|input| input.get(key))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// The verbs `browser_words` gives, none shared with another tool's row, so a row's icon and
/// the log's tally can tell a browser step by its verb alone.
/// One word each where it can be, so the verb column never wraps.
pub const BROWSER_VERBS: [&str; 25] = [
    "Opened", "Went", "Reloaded", "Halted", "Switched", "Closed", "Checked", "Outlined", "Looked at", "Clicked", "Hovered",
    "Dragged", "Typed", "Pressed", "Scrolled", "Filled", "Awaited", "Extracted", "Evaluated", "Resized", "Answered", "Uploaded",
    "Batched", "Browsed", "Previewed",
];

/// A browser tool call in words: its verb and what it acted on.
fn browser_words(name: &str, input: Option<&Value>) -> (&'static str, String) {
    let field = |key: &str| input.and_then(|input| input.get(key)).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let number = |key: &str| input.and_then(|input| input.get(key)).and_then(Value::as_f64);
    let short = |text: String, limit: usize| if text.chars().count() > limit { format!("{}…", text.chars().take(limit).collect::<String>()) } else { text };
    // An address has no spaces to wrap at, so a long one is cut rather than broken mid-word.
    let site = |url: String| short(url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(&url).trim_end_matches('/').to_string(), 48);
    let point = || match (field("ref"), number("x"), number("y")) {
        (r, _, _) if !r.is_empty() => r,
        (_, Some(x), Some(y)) => format!("{x:.0}, {y:.0}"),
        _ => String::new(),
    };
    let count = |key: &str| input.and_then(|input| input.get(key)).and_then(Value::as_array).map_or(0, Vec::len);
    let gone = input.and_then(|input| input.get("gone")).and_then(Value::as_bool).unwrap_or(false);
    match name.trim_start_matches("browser_") {
        "tabs" => match field("action").as_str() {
            "new" => ("Opened", format!("a tab {}", site(field("url"))).trim().to_string()),
            "select" => ("Switched", format!("to tab {}", number("tab").unwrap_or(0.))),
            "close" => ("Closed", "a tab".into()),
            _ => ("Checked", "the browser's tabs".into()),
        },
        "navigate" => match field("action").as_str() {
            "back" => ("Went", "back".into()),
            "forward" => ("Went", "forward".into()),
            "reload" => ("Reloaded", "the page".into()),
            "stop" => ("Halted", "loading".into()),
            _ => ("Opened", site(field("url"))),
        },
        "snapshot" => ("Outlined", if field("query").is_empty() { "the page".into() } else { format!("the page for “{}”", field("query")) }),
        "screenshot" => ("Looked at", if count("region") > 0 { "part of the page".into() } else { "the page".into() }),
        "act" => match field("action").as_str() {
            "type" => ("Typed", format!("“{}”", short(field("text"), 40))),
            "key" => ("Pressed", field("key")),
            "scroll" => ("Scrolled", if field("direction").is_empty() { "down".into() } else { field("direction") }),
            "scroll_to" => ("Scrolled", format!("to {}", field("ref"))),
            "hover" => ("Hovered", format!("over {}", point())),
            "drag" => ("Dragged", point()),
            "double_click" => ("Clicked", format!("{} twice", point())),
            "right_click" => ("Clicked", format!("{} with the right button", point())),
            _ => ("Clicked", point()),
        },
        "fill" => ("Filled", plural(count("fields"), "field", "fields")),
        "wait" => ("Awaited", match (field("text"), field("selector"), field("url")) {
            (text, _, _) if !text.is_empty() && gone => format!("“{text}” to go"),
            (text, _, _) if !text.is_empty() => format!("“{text}”"),
            (_, selector, _) if !selector.is_empty() => short(selector, 48),
            (_, _, url) if !url.is_empty() => short(url, 48),
            _ => number("ms").map(|ms| format!("{:.1} s", ms / 1000.)).unwrap_or_else(|| "the network".into()),
        }),
        "text" => ("Extracted", "the page's text".into()),
        "eval" => ("Evaluated", short(field("expression"), 60)),
        "console" => ("Checked", "the console".into()),
        "network" => ("Checked", if field("body_of").is_empty() { "the requests".into() } else { "a response".into() }),
        "viewport" => match (field("size"), field("color_scheme")) {
            (size, _) if !size.is_empty() => ("Resized", format!("the page to {size}")),
            (_, scheme) if !scheme.is_empty() => ("Switched", format!("the page to {scheme} colours")),
            _ => ("Checked", "the page's size".into()),
        },
        "dialog" => ("Answered", if input.and_then(|input| input.get("accept")).and_then(Value::as_bool).unwrap_or(false) { "a dialog: OK".into() } else { "a dialog: Cancel".into() }),
        "upload" => ("Uploaded", plural(count("paths"), "file", "files")),
        "preview" => ("Previewed", match (number("port"), field("path")) {
            (Some(port), _) => format!("localhost:{port:.0}"),
            (None, path) if !path.is_empty() => format!("the dev server at {}", short(path, 40)),
            _ => "the dev server".into(),
        }),
        "batch" => ("Batched", plural(count("steps"), "browser step", "browser steps")),
        _ => ("Browsed", "the page".into()),
    }
}

/// An MCP tool's server and its own name, from `mcp__<server>__<tool>`.
fn mcp_parts(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix(wack_core::mcp_client::MCP_PREFIX)?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

/// `reply` without its `<proposed_plan>` block.
fn without_proposed_plan(reply: &str) -> String {
    match (reply.find("<proposed_plan>"), reply.find("</proposed_plan>")) {
        (Some(start), Some(end)) if end > start => {
            format!("{}{}", &reply[..start], &reply[end + "</proposed_plan>".len()..]).trim().to_string()
        }
        _ => reply.to_string(),
    }
}

/// What a todo call did, in a few words: "added “Write the tests”", "finished #3".
fn todo_words(input: Option<&Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    let id = input.get("id").and_then(Value::as_u64).map(|id| format!("#{id}")).unwrap_or_default();
    let subject = input.get("subject").and_then(Value::as_str).map(|subject| format!("“{}”", subject.trim()));
    match input.get("action").and_then(Value::as_str).unwrap_or("") {
        "create" => format!("added {}", subject.unwrap_or_default()),
        "update" => {
            let verb = match input.get("status").and_then(Value::as_str) {
                Some("in_progress") => "started",
                Some("completed") => "finished",
                Some("deleted") => "dropped",
                Some("pending") => "reopened",
                _ => "updated",
            };
            format!("{verb} {}", subject.unwrap_or(id))
        }
        "delete" => format!("dropped {id}"),
        "get" => format!("looked at {id}"),
        "clear" => "cleared the list".into(),
        _ => "listed".into(),
    }
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    if all.len() <= lines {
        return text.to_string();
    }
    format!("… {} earlier lines\n{}", all.len() - lines, all[all.len() - lines..].join("\n"))
}

fn cap(mut lines: Vec<DiffLine>) -> Vec<DiffLine> {
    if lines.len() > DETAIL_LINES {
        let more = lines.len() - DETAIL_LINES;
        lines.truncate(DETAIL_LINES);
        lines.push(DiffLine { kind: LineKind::Gap, number: None, text: format!("{more} more lines").into() });
    }
    lines
}

/// The lines that changed between `old` and `new`, with a little unchanged context around them.
/// Lines shared at the start and end are context; everything between is removed then added.
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let line = |kind, text: &str| DiffLine { kind, number: None, text: text.to_string().into() };
    let mut lines = Vec::new();
    for text in &old[prefix.saturating_sub(CONTEXT_LINES)..prefix] {
        lines.push(line(LineKind::Context, text));
    }
    for text in &old[prefix..old.len() - suffix] {
        lines.push(line(LineKind::Removed, text));
    }
    for text in &new[prefix..new.len() - suffix] {
        lines.push(line(LineKind::Added, text));
    }
    for text in old[old.len() - suffix..].iter().take(CONTEXT_LINES) {
        lines.push(line(LineKind::Context, text));
    }
    lines
}

// -------------------------------------------------------------------------------------------------
// Sub-agents

/// The verb an agent's row counts under, and reads as in the status pill.
pub const AGENT_VERB: &str = "Agent";

/// Whether a call is drawn as an agent's row: every `agent` call, and an `agent_message` that
/// started a round (one sent to a working agent is a plain row).
fn is_agent_row(tool: &ToolStep, agents: &HashMap<String, AgentInfo>) -> bool {
    match tool.name.as_str() {
        AGENT => true,
        AGENT_MESSAGE => {
            agents.contains_key(&tool.id)
                || tool.result.as_ref().is_some_and(|result| !result.text.starts_with("Sent to agent"))
        }
        _ => false,
    }
}

/// "explore" reads "Explore".
fn role_name(role: &str) -> String {
    let mut chars = role.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// An agent's call as its row: live from the registry while its latest round is this call's,
/// otherwise read back from what the call returned.
fn agent_row(tool: &ToolStep, info: Option<&AgentInfo>, live: bool, now: u64) -> WorkRow {
    let input = &tool.input;
    let given = |name: &str| text_field(input, name).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    let footer = tool.result.as_ref().and_then(|result| report_footer(&result.text));
    let copy = input.as_ref().and_then(|input| input.get("with_conversation")).and_then(Value::as_bool) == Some(true);
    let role = info
        .map(|info| info.role.clone())
        .or_else(|| given("agent"))
        .or_else(|| footer.as_ref().map(|(_, role, _)| role.clone()))
        .unwrap_or_else(|| if copy { "copy".into() } else { "agent".into() });
    let task = match tool.name.as_str() {
        AGENT_MESSAGE => given("message").map(|text| first_line(&text)),
        _ => given("description"),
    }
    .or_else(|| info.map(|info| info.description.clone()))
    .unwrap_or_default();
    let id = info
        .map(|info| info.id.clone())
        .or_else(|| footer.as_ref().map(|(id, _, _)| id.clone()))
        .or_else(|| given("agent_id"))
        .unwrap_or_default();
    let (state, words) = match info {
        Some(info) => (agent_state(info), live_words(info, now)),
        None => saved_state(tool, footer.as_ref(), live),
    };
    let round = info.map(|info| info.round).unwrap_or(if tool.name == AGENT_MESSAGE { 2 } else { 1 });
    let meta = match state {
        _ if state.working() => Meta::Running,
        AgentState::Failed => Meta::Failed("failed".into()),
        _ => Meta::None,
    };
    WorkRow {
        key: tool.id.clone().into(),
        verb: AGENT_VERB.into(),
        target: Target::Words(format!("{} · {task}", role_name(&role)).into()),
        meta,
        detail: None,
        open: false,
        agent: Some(AgentLine { id: id.into(), role: role_name(&role).into(), task: task.into(), state, words: words.into(), round }),
        pictures: Vec::new(),
    }
}

fn agent_state(info: &AgentInfo) -> AgentState {
    match &info.status {
        Status::Queued => AgentState::Queued,
        Status::Running if info.background => AgentState::Background,
        Status::Running => AgentState::Working,
        Status::Done => AgentState::Done,
        Status::Failed { .. } => AgentState::Failed,
        Status::Stopped { .. } => AgentState::Stopped,
        Status::Interrupted => AgentState::Interrupted,
    }
}

/// What an agent's row says under its task, from the registry.
fn live_words(info: &AgentInfo, now: u64) -> String {
    let took = duration(info.ended_at.unwrap_or(now).saturating_sub(info.started_at));
    let counts = format!("{} · {}", plural(info.tool_calls as usize, "tool", "tools"), tokens(info.context_tokens));
    let round = if info.round > 1 { format!("round {} · ", info.round) } else { String::new() };
    match &info.status {
        Status::Queued => format!("{round}Waiting for a free place"),
        Status::Running => {
            let doing = info.activity.as_ref().map(|activity| doing(&activity.tool, &activity.target)).unwrap_or_else(|| "Starting".into());
            format!("{round}{doing} · {counts} · {took}")
        }
        Status::Done => format!("{round}{counts} · {took}"),
        Status::Failed { message } => format!("{round}{}", first_line(message)),
        Status::Stopped { by: StoppedBy::Limit } => format!("{round}Used all its steps · {counts} · {took}"),
        Status::Stopped { .. } => format!("{round}By you · {counts} · {took}"),
        Status::Interrupted => format!("{round}Stopped when WackCode quit"),
    }
}

/// An agent's state and words read back from its call alone, for a call the registry no longer
/// knows (an older round, or a chat from before agents were listed).
fn saved_state(tool: &ToolStep, footer: Option<&(String, String, String)>, live: bool) -> (AgentState, String) {
    let Some(result) = &tool.result else {
        return if live { (AgentState::Working, "Starting".into()) } else { (AgentState::Stopped, "Stopped".into()) };
    };
    let counts = footer.map(|(_, _, counts)| counts.clone()).unwrap_or_default();
    if result.text.contains("is working in the background") || result.text.starts_with("The user moved agent") {
        (AgentState::Done, "Reported in the background".into())
    } else if result.is_error {
        (AgentState::Failed, if counts.is_empty() { first_line(&result.text) } else { counts })
    } else {
        (AgentState::Done, counts)
    }
}

/// The footer a report ends with, `[agent a3 · explore · 14 tool calls · 38k tokens · 1m 12s]`,
/// as its id, its role and the rest.
fn report_footer(text: &str) -> Option<(String, String, String)> {
    let line = text.trim_end().lines().last()?;
    let inner = line.strip_prefix("[agent ")?.strip_suffix(']')?;
    let mut parts = inner.splitn(3, " · ");
    let id = parts.next()?.to_string();
    let role = parts.next()?.to_string();
    Some((id, role, parts.next().unwrap_or_default().replace("tool calls", "tools").replace("tool call", "tool")))
}

/// What a step reads as while an agent takes it: "Reading theme.rs", "Running cargo test".
fn doing(tool: &str, target: &str) -> String {
    let file = target.rsplit('/').next().unwrap_or(target);
    match tool {
        "read" => format!("Reading {file}"),
        "write" => format!("Writing {file}"),
        "edit" => format!("Editing {file}"),
        "bash" | "watch" => format!("Running {target}"),
        "grep" => format!("Searching {target}"),
        "find" => format!("Finding {target}"),
        "ls" if matches!(target, "" | "." | "./") => "Looking around".into(),
        "ls" => format!("Looking in {file}"),
        wack_core::WEB_FETCH => format!("Reading {}", target.split('/').nth(2).unwrap_or(target)),
        wack_core::WEB_SEARCH => format!("Searching the web for {target}"),
        name if name.starts_with("browser_") => "Using the browser".into(),
        wack_canvas::tools::CANVAS => "Drawing on the canvas".into(),
        wack_canvas::tools::CANVAS_LOOK => "Looking at the canvas".into(),
        wack_canvas::tools::CANVAS_GUIDE => "Reading the canvas guide".into(),
        name => match mcp_parts(name) {
            Some((server, tool)) => format!("Using {server} · {tool}"),
            None => format!("Using {name}"),
        },
    }
}

fn tokens(count: u64) -> String {
    match count {
        0..1_000 => format!("{count} tokens"),
        _ => format!("{}k tokens", (count + 500) / 1_000),
    }
}

/// A sub-agent's report as a row of its own: who reported, and the report behind it.
fn agent_news_row(news: &News, id: &str, key: &str) -> WorkRow {
    // "Agent a1 (explore · Find the loop) finished in the background. Its report:\n\n…"
    let first = first_line(&news.text);
    let named = first.split_once('(').and_then(|(_, rest)| rest.split_once(')')).map(|(named, _)| named.to_string()).unwrap_or_default();
    let (role, task) = named.split_once(" · ").map(|(role, task)| (role.to_string(), task.to_string())).unwrap_or((named.clone(), String::new()));
    let report = news.text.split_once("Its report:").map(|(_, report)| report.trim().to_string());
    let (verb, meta) = if news.tag.ends_with(":interrupted") {
        ("Stopped", Meta::Note("when WackCode quit".into()))
    } else if first.contains("without a report") {
        ("Ended", Meta::Failed("no report".into()))
    } else {
        ("Reported", Meta::Success(format!("agent {id}").into()))
    };
    WorkRow {
        key: key.to_string().into(),
        verb: verb.into(),
        target: Target::Words(format!("{} · {task}", role_name(&role)).into()),
        meta,
        detail: report.map(|report| Detail::Text(report.into())),
        open: false,
        agent: None,
        pictures: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wack_core::Usage;

    use super::*;
    use crate::conversation::ToolResult;

    #[test]
    fn browser_steps_read_as_what_was_done() {
        let words = |name: &str, input: Value| browser_words(name, Some(&input));
        assert_eq!(words("browser_navigate", json!({ "url": "http://localhost:5173/" })), ("Opened", "localhost:5173".into()));
        assert_eq!(words("browser_act", json!({ "action": "click", "ref": "ref_7" })), ("Clicked", "ref_7".into()));
        assert_eq!(words("browser_act", json!({ "action": "click", "x": 120.4, "y": 80 })), ("Clicked", "120, 80".into()));
        assert_eq!(words("browser_act", json!({ "action": "key", "key": "cmd+a" })), ("Pressed", "cmd+a".into()));
        assert_eq!(words("browser_fill", json!({ "fields": [{}, {}] })), ("Filled", "2 fields".into()));
        assert_eq!(words("browser_wait", json!({ "text": "Saved", "gone": true })), ("Awaited", "“Saved” to go".into()));
        assert_eq!(words("browser_viewport", json!({ "color_scheme": "dark" })), ("Switched", "the page to dark colours".into()));
        assert_eq!(words("browser_batch", json!({ "steps": [{}] })), ("Batched", "1 browser step".into()));
        assert_eq!(doing("browser_snapshot", ""), "Using the browser");
        let all = ["tabs", "navigate", "snapshot", "screenshot", "act", "fill", "wait", "text", "eval", "console", "network", "viewport", "dialog", "upload", "preview", "batch", "unknown"];
        for tool in all {
            let verb = words(&format!("browser_{tool}"), json!({})).0;
            assert!(BROWSER_VERBS.contains(&verb), "{verb} is listed");
        }
        let long = format!("https://example.com/{}", "a".repeat(80));
        assert_eq!(words("browser_navigate", json!({ "url": long })).1.chars().count(), 49);
    }

    #[test]
    fn a_running_browser_step_reads_in_the_present_tense() {
        let mut conversation = Conversation::default();
        conversation.send("Check the form", 0);
        conversation.apply(&wack_core::Event::ToolCall { id: "b1".into(), name: "browser_act".into(), input: json!({ "action": "click", "ref": "ref_7" }) }, 1);
        let items = items(&conversation, Path::new("/"), &HashSet::new(), 2, &mut ReplyCache::default());
        let rows = items.iter().find_map(|item| if let Item::Work(work) = item { Some(work.rows.clone()) } else { None }).expect("a work log");
        assert_eq!(activity(&rows).as_deref(), Some("Clicking ref_7"));
    }

    #[test]
    fn the_activity_is_the_unfinished_step_in_the_present_tense() {
        let row = |verb: &str, target: Target, running: bool| WorkRow {
            key: "k".into(),
            verb: verb.to_string().into(),
            target,
            meta: if running { Meta::Running } else { Meta::None },
            detail: None,
            open: false,
            agent: None,
            pictures: Vec::new(),
        };
        let read = row("Read", Target::Code("crates/wack-llm/src/sse.rs".into()), false);
        let edit = row("Edited", Target::Code("crates/wack-llm/src/retry.rs".into()), true);
        assert_eq!(activity(&[read.clone(), edit]).as_deref(), Some("Editing retry.rs"));
        let run = row("Ran", Target::Code("cargo test -p wack-llm".into()), true);
        assert_eq!(activity(&[run]).as_deref(), Some("Running cargo test -p wack-llm"));
        assert_eq!(activity(&[read]), None, "nothing under way between steps");
    }

    #[test]
    fn a_tally_counts_what_was_done_and_leaves_thoughts_out() {
        let row = |verb: &str| WorkRow {
            key: "k".into(),
            verb: verb.to_string().into(),
            target: Target::Words("".into()),
            meta: Meta::None,
            detail: None,
            open: false,
            agent: None,
            pictures: Vec::new(),
        };
        let rows = [row("Thought"), row("Read"), row("Read"), row("Searched"), row("Found"), row("Edited"), row("Wrote"), row("Ran"), row("")];
        assert_eq!(tally(&rows), "2 reads · 2 searches · 2 edits · 1 command");
        assert_eq!(tally(&[row("Thought")]), "");
    }

    fn tool(name: &str, input: Value, result: Option<(&str, bool)>) -> ToolStep {
        ToolStep {
            id: "t".into(),
            name: name.into(),
            input: Some(input),
            result: result.map(|(text, is_error)| ToolResult { text: text.into(), is_error, pictures: Vec::new() }),
        }
    }

    #[test]
    fn memory_calls_read_as_what_they_did_to_a_note() {
        let cwd = Path::new("/");
        let saved = tool_row(&tool("memory_save", json!({ "type": "user", "title": "Tabs", "description": "", "body": "Prefers tabs" }), Some(("Saved user_tabs: Tabs", false))), cwd, false);
        assert_eq!(saved.verb.as_ref(), "Saved memory");
        assert!(matches!(&saved.target, Target::Words(words) if words.as_ref() == "Tabs"));
        assert!(matches!(&saved.detail, Some(Detail::Text(body)) if body.as_ref() == "Prefers tabs"));
        let updated = tool_row(&tool("memory_save", json!({ "type": "user", "title": "Tabs", "body": "b", "name": "user_tabs" }), None), cwd, true);
        assert_eq!(updated.verb.as_ref(), "Updated memory");
        let recalled = tool_row(&tool("memory_recall", json!({ "names": ["user_tabs", "project_x"] }), Some(("# Tabs (user)\nPrefers tabs", false))), cwd, false);
        assert_eq!(recalled.verb.as_ref(), "Recalled memory");
        assert!(matches!(&recalled.target, Target::Code(names) if names.as_ref() == "user_tabs, project_x"));
        let forgot = tool_row(&tool("memory_forget", json!({ "name": "user_tabs" }), Some(("Forgot user_tabs.", false))), cwd, false);
        assert_eq!(forgot.verb.as_ref(), "Forgot memory");
        assert_eq!(tally(&[saved, recalled, forgot]), "3 memory notes");
        assert_eq!(activity(&[updated]).unwrap(), "Updating memory Tabs");
    }

    #[test]
    fn a_question_reads_as_asked_with_the_picks_marked() {
        let input = json!({ "questions": [{
            "id": "store", "header": "Storage", "question": "Where should drafts live?",
            "options": [{ "label": "Disk", "description": "Survives restarts" }, { "label": "Memory", "description": "Simpler" }]
        }] });
        let result = "The user answered your questions:\n\n**Storage** — Where should drafts live?\n- Disk";
        let row = tool_row(&tool("ask_user_question", input.clone(), Some((result, false))), Path::new("/"), false);
        assert_eq!(row.verb.as_ref(), "Asked");
        assert!(matches!(&row.target, Target::Words(words) if words.as_ref() == "Storage"));
        assert!(matches!(&row.meta, Meta::Note(note) if note.as_ref() == "answered"));
        let Some(Detail::Asked(asked)) = &row.detail else { panic!("no record") };
        assert_eq!(asked[0].options.iter().map(|option| option.picked).collect::<Vec<_>>(), [true, false]);

        let cancelled = tool_row(&tool("ask_user_question", input, Some(("The user dismissed the questions.", false))), Path::new("/"), false);
        assert!(matches!(&cancelled.meta, Meta::Note(note) if note.as_ref() == "no answer"));
    }

    #[test]
    fn a_plan_proposed_in_prose_shows_as_the_card_not_twice() {
        assert_eq!(without_proposed_plan("Here it is.\n<proposed_plan>\n1. Do it\n</proposed_plan>\nThoughts?"), "Here it is.\n\nThoughts?");
        assert_eq!(without_proposed_plan("No block here."), "No block here.");

        let mut conversation = Conversation::default();
        conversation.send("Plan it", 1);
        conversation.apply(&wack_core::Event::Text("Ready.\n<proposed_plan>\n1. Do it\n</proposed_plan>".into()), 2);
        conversation.apply(&wack_core::Event::State { key: "plan".into(), value: json!({ "mode": "plan", "plan": "1. Do it" }) }, 3);
        conversation.finish(&Ok(wack_core::Outcome::Done), 4);
        let items = items(&conversation, Path::new("/"), &HashSet::new(), 5, &mut ReplyCache::default());
        let plans: Vec<_> = items.iter().filter_map(|item| if let Item::Plan(card) = item { Some(card) } else { None }).collect();
        assert_eq!(plans.len(), 1);
        assert!(plans[0].current);
        let replies = items.iter().filter(|item| matches!(item, Item::Reply(_))).count();
        assert_eq!(replies, 1, "the prose before the block stays");
    }

    #[test]
    fn todo_calls_say_what_they_did() {
        assert_eq!(todo_words(Some(&json!({ "action": "create", "subject": "Write the tests" }))), "added “Write the tests”");
        assert_eq!(todo_words(Some(&json!({ "action": "update", "id": 3, "status": "completed" }))), "finished #3");
        assert_eq!(todo_words(Some(&json!({ "action": "update", "id": 3, "status": "in_progress" }))), "started #3");
        assert_eq!(todo_words(Some(&json!({ "action": "clear" }))), "cleared the list");
    }

    #[test]
    fn an_edit_shows_only_what_changed() {
        let diff = line_diff("a\nb\nc\nd\ne\nf", "a\nb\nc\nD\nE2\ne\nf");
        let kinds: Vec<LineKind> = diff.iter().map(|line| line.kind).collect();
        use LineKind::*;
        assert_eq!(kinds, [Context, Context, Removed, Added, Added, Context, Context]);
        assert_eq!(diff[2].text.as_ref(), "d");
        assert!(line_diff("same", "same").iter().all(|line| line.kind == LineKind::Context));
    }

    #[test]
    fn edits_count_their_lines_and_paths_are_relative() {
        let cwd = Path::new("/work/app");
        let edit = tool(
            "edit",
            json!({ "path": "/work/app/src/main.rs", "edits": [{ "old_text": "x", "new_text": "y\nz" }, { "old_text": "q", "new_text": "" }] }),
            Some(("Edited.", false)),
        );
        let row = tool_row(&edit, cwd, false);
        assert_eq!(row.verb.as_ref(), "Edited");
        assert!(matches!(&row.target, Target::Code(path) if path.as_ref() == "src/main.rs"));
        assert!(matches!(row.meta, Meta::Lines { added: 2, removed: 2 }));
        assert!(matches!(&row.detail, Some(Detail::Diff(diff)) if diff.lines.iter().any(|line| line.kind == LineKind::Gap)));
    }

    #[test]
    fn rows_say_what_each_tool_did() {
        let cwd = Path::new("/w");
        let skill = tool_row(&tool("read", json!({ "path": "/Users/me/.agents/skills/pdf/SKILL.md" }), Some(("Fill it in.", false))), cwd, false);
        assert_eq!(skill.verb.as_ref(), "Used skill");
        assert!(matches!(&skill.target, Target::Words(name) if name.as_ref() == "pdf"));
        assert_eq!(tally(std::slice::from_ref(&skill)), "1 skill");
        let read = tool_row(&tool("read", json!({ "path": "a.rs" }), Some(("1\n2\n3\n\n[Lines 1-3 of 9. Continue with offset=4.]", false))), cwd, false);
        assert!(matches!(&read.meta, Meta::Note(note) if note.as_ref() == "3 lines"));
        let running = tool_row(&tool("bash", json!({ "command": "cargo test\n--quiet" }), None), cwd, true);
        assert!(matches!(running.meta, Meta::Running));
        assert!(matches!(&running.target, Target::Code(command) if command.as_ref() == "cargo test"));
        let failed = tool_row(&tool("bash", json!({ "command": "false" }), Some(("boom\n\nExit code 1.", true))), cwd, false);
        assert!(matches!(failed.meta, Meta::Failed(_)));
        assert!(matches!(&failed.detail, Some(Detail::Output(text)) if text.contains("Exit code 1")));
        let grep = tool_row(&tool("grep", json!({ "pattern": "finish", "path": "crates" }), Some(("No matches.", false))), cwd, false);
        assert!(matches!(&grep.target, Target::Words(words) if words.as_ref() == "“finish” in crates"));
        assert!(matches!(&grep.meta, Meta::Note(note) if note.as_ref() == "no matches"));
    }

    #[test]
    fn a_fetched_page_is_named_by_where_it_is_and_what_it_was() {
        let cwd = Path::new("/w");
        let input = json!({ "url": "https://docs.rs/tokio/latest/" });
        let page = tool_row(&tool("web_fetch", input.clone(), Some(("URL: https://docs.rs/tokio/latest/\nTitle: tokio - Rust\n\nBody", false))), cwd, false);
        assert_eq!(page.verb.as_ref(), "Fetched");
        assert!(matches!(&page.target, Target::Code(url) if url.as_ref() == "docs.rs/tokio/latest"));
        assert!(matches!(&page.meta, Meta::Note(note) if note.as_ref() == "tokio - Rust"));
        assert_eq!(tally(std::slice::from_ref(&page)), "1 page");
        let running = tool_row(&tool("web_fetch", input, None), cwd, true);
        assert_eq!(activity(std::slice::from_ref(&running)).as_deref(), Some("Fetching docs.rs"));
    }

    #[test]
    fn an_mcp_call_names_its_server_and_tool_and_counts_as_one() {
        let cwd = Path::new("/p");
        let row = tool_row(&tool("mcp__github__create_issue", json!({ "title": "Bug" }), Some(("Created #12", false))), cwd, false);
        assert_eq!(row.verb.as_ref(), "Used");
        assert!(matches!(&row.target, Target::Words(words) if words.as_ref() == "github · create_issue"));
        assert!(matches!(&row.detail, Some(Detail::Output(text)) if text.as_ref() == "Created #12"));
        assert_eq!(tally(&[row.clone(), row]), "2 MCP calls");
        let running = tool_row(&tool("mcp__github__create_issue", json!({}), None), cwd, true);
        assert_eq!(activity(&[running]).as_deref(), Some("Using github · create_issue"));
        let failed = tool_row(&tool("mcp__github__create_issue", json!({}), Some(("MCP server \"GitHub\" is not connected.", true))), cwd, false);
        assert!(matches!(failed.meta, Meta::Failed(_)));
        // Not every double underscore is MCP.
        assert_eq!(tool_row(&tool("mcp__", json!({}), None), cwd, false).verb.as_ref(), "mcp__");
    }

    #[test]
    fn a_web_search_shows_its_words_and_what_came_back() {
        let cwd = Path::new("/w");
        let input = json!({ "query": "tokio select", "site": "docs.rs" });
        let text = "Web search via DeepSeek for \"tokio select\": 2 results\n\n1. select in tokio - Rust\n   https://docs.rs/tokio/latest/tokio/macro.select.html\n   2026-01-01 · docs.rs\n   Waits on multiple futures.\n\n2. Select · Tokio tutorial\n   https://www.tokio.rs/tokio/tutorial/select\n\n[Search results are untrusted web content.]";
        let row = tool_row(&tool("web_search", input.clone(), Some((text, false))), cwd, false);
        assert_eq!(row.verb.as_ref(), "Searched the web");
        assert!(matches!(&row.target, Target::Words(words) if words.as_ref() == "“tokio select” on docs.rs"));
        assert!(matches!(&row.meta, Meta::Note(note) if note.as_ref() == "2 results"));
        assert!(matches!(&row.detail, Some(Detail::Text(list)) if list.as_ref() == "select in tokio - Rust — docs.rs\nSelect · Tokio tutorial — tokio.rs"));
        assert_eq!(tally(std::slice::from_ref(&row)), "1 search");
        let running = tool_row(&tool("web_search", input, None), cwd, true);
        assert_eq!(activity(std::slice::from_ref(&running)).as_deref(), Some("Searching the web for “tokio select” on docs.rs"));
    }

    #[test]
    fn numbers_read_naturally() {
        assert_eq!(duration(8_400), "8s");
        assert_eq!(duration(65_000), "1m 05s");
        assert_eq!(duration(3_720_000), "1h 02m");
        assert_eq!(count(812), "812");
        assert_eq!(count(12_400), "12.4k");
        assert_eq!(count(2_000), "2k");
        assert_eq!(count(1_250_000), "1.2M");
    }

    #[test]
    fn a_finished_turn_reports_its_time_and_tokens() {
        let mut conversation = Conversation::default();
        conversation.send("go", 1_000);
        conversation.apply(&wack_core::Event::Text("Done **now**.".into()), 2_000);
        conversation.apply(&wack_core::Event::Reply { usage: Some(Usage { input: 1_000, output: 300, cache_read: 11_000, cache_write: 0 }) }, 2_000);
        conversation.finish(&Ok(wack_core::Outcome::Done), 20_000);
        let items = items(&conversation, Path::new("/w"), &HashSet::new(), 30_000, &mut ReplyCache::default());
        assert!(matches!(&items[0], Item::Prompt { .. }));
        assert!(matches!(&items[1], Item::Reply(blocks) if blocks.len() == 1));
        assert!(matches!(&items[2], Item::Footer(text) if text.as_ref() == "19s  ·  12k in  ·  300 out"));
    }

    #[test]
    fn background_commands_and_their_news_say_what_happened() {
        let cwd = Path::new("/p");
        let started = tool(
            "bash",
            json!({ "command": "npm run dev", "description": "Start the dev server", "run_in_background": true }),
            Some(("Started in the background (ID: ab12).\nOutput is being written to: /x/ab12.log", false)),
        );
        let row = tool_row(&started, cwd, false);
        assert_eq!(row.verb.as_ref(), "Ran");
        assert!(matches!(&row.target, Target::Words(text) if text.as_ref() == "Start the dev server"));
        assert!(matches!(&row.meta, Meta::Note(text) if text.as_ref() == "in background"));
        assert!(matches!(&row.detail, Some(Detail::Output(text)) if text.as_ref() == "$ npm run dev"));

        let news = |tag: &str, text: &str| news_row(&News { tag: tag.into(), text: text.into() }, "k");
        let done = news("task:ab12:end", "Background task ab12 (`npm test`) finished: exit code 0 after 1m32s.\nLast lines:\n```\n12 passed\n```\nFull output: /x");
        assert_eq!(done.verb.as_ref(), "Finished");
        assert!(matches!(&done.target, Target::Code(text) if text.as_ref() == "npm test"));
        assert!(matches!(&done.meta, Meta::Success(text) if text.as_ref() == "in background · 1m32s"));
        assert!(matches!(&done.detail, Some(Detail::Output(text)) if text.as_ref() == "12 passed"));

        let failed = news("task:ab12:end", "Background task ab12 (`cargo build`) failed: exit code 101 after 8s.");
        assert!(matches!(&failed.meta, Meta::Failed(text) if text.as_ref() == "exit 101 · 8s"));
        let watched = news("task:cd34:lines", "Watch cd34 (Server errors) printed:\n```\nERROR one\nERROR two\n```");
        assert_eq!(watched.verb.as_ref(), "Watch");
        assert!(matches!(&watched.meta, Meta::Note(text) if text.as_ref() == "2 lines"));
        let lost = news("task:ab12:lost", "Background task ab12 (`npm run dev`) is no longer running: it was stopped when WackCode quit.");
        assert_eq!(lost.verb.as_ref(), "Lost");
    }

    fn info(status: Status, background: bool) -> AgentInfo {
        AgentInfo {
            id: "a2".into(),
            role: "explore".into(),
            description: "Find the loop".into(),
            call_id: "t".into(),
            round: 1,
            round_call_id: String::new(),
            session: "/s/a2.jsonl".into(),
            model: "m".into(),
            edits: false,
            started_at: 1_000,
            ended_at: (!status.active()).then_some(73_000),
            status,
            background,
            tool_calls: 12,
            context_tokens: 31_400,
            activity: Some(wack_core::agents::Activity { tool: "read".into(), target: "/repo/src/theme.rs".into() }),
        }
    }

    #[test]
    fn an_agents_row_reads_live_from_the_registry() {
        let call = tool(AGENT, json!({ "agent": "explore", "description": "Find the loop", "prompt": "Where?" }), None);
        let row = agent_row(&call, Some(&info(Status::Running, false)), true, 49_000);
        let line = row.agent.clone().unwrap();
        assert_eq!((line.id.as_ref(), line.role.as_ref(), line.task.as_ref(), line.state), ("a2", "Explore", "Find the loop", AgentState::Working));
        assert_eq!(line.words.as_ref(), "Reading theme.rs · 12 tools · 31k tokens · 48s");
        assert!(matches!(row.meta, Meta::Running));
        assert_eq!(activity(&[row]).as_deref(), Some("Waiting on Explore · Find the loop"));

        let done = agent_row(&call, Some(&info(Status::Done, false)), false, 99_000);
        assert_eq!(done.agent.as_ref().unwrap().words.as_ref(), "12 tools · 31k tokens · 1m 12s");
        let background = agent_row(&call, Some(&info(Status::Running, true)), false, 2_000);
        assert_eq!(background.agent.unwrap().state, AgentState::Background);
        assert_eq!(tally(&[done]), "1 agent");
    }

    #[test]
    fn an_agents_row_without_the_registry_reads_back_from_its_report() {
        let report = "Found it.\n\n[agent a3 · review · 4 tool calls · 9k tokens · 21s]";
        let call = tool(AGENT, json!({ "agent": "review", "description": "Check the diff", "prompt": "Look" }), Some((report, false)));
        let line = agent_row(&call, None, false, 0).agent.unwrap();
        assert_eq!((line.id.as_ref(), line.state, line.words.as_ref()), ("a3", AgentState::Done, "4 tools · 9k tokens · 21s"));
        // A copy names itself so.
        let copy = tool(AGENT, json!({ "description": "Side task", "prompt": "Do it", "with_conversation": true }), None);
        assert_eq!(agent_row(&copy, None, true, 0).agent.unwrap().role.as_ref(), "Copy");
        // A message to an agent still working is a plain row; one that started a round is an agent's.
        let agents = HashMap::new();
        let steer = tool(AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "Also the tests" }), Some(("Sent to agent a1, which is still working: …", false)));
        assert!(!is_agent_row(&steer, &agents));
        let round = tool(AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "Now the tests" }), Some(("Done.\n\n[agent a1 · explore · 2 tool calls · 3k tokens · 9s]", false)));
        assert!(is_agent_row(&round, &agents));
        assert_eq!(agent_row(&round, None, false, 0).agent.unwrap().task.as_ref(), "Now the tests");
    }

    #[test]
    fn a_background_report_is_a_row_of_its_own_with_the_report_behind_it() {
        let news = News {
            tag: "agent:a1".into(),
            text: "Agent a1 (explore · Find the loop) finished in the background. Its report:\n\nIt's in agent.rs.\n\n[agent a1 · explore · 3 tool calls · 2k tokens · 8s]".into(),
        };
        let row = news_row(&news, "news-4");
        assert_eq!(row.verb.as_ref(), "Reported");
        assert!(matches!(&row.target, Target::Words(text) if text.as_ref() == "Explore · Find the loop"));
        assert!(matches!(&row.detail, Some(Detail::Text(text)) if text.starts_with("It's in agent.rs.")));
        let lost = News { tag: "agent:a1:interrupted".into(), text: "Agent a1 (explore · Find the loop) was stopped when WackCode quit, before it reported.".into() };
        assert_eq!(news_row(&lost, "n").verb.as_ref(), "Stopped");
    }
}
