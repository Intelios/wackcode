//! Turns a [`Conversation`] into the rows the transcript draws: what each tool call says in the
//! work log, the diff an edit opens into, how long a turn took. Pure, so it is tested here.

use std::{collections::HashSet, path::Path, rc::Rc};

use serde_json::Value;

use crate::{
    conversation::{Conversation, Entry, Step, ToolStep, Turn, TurnEnd},
    markdown,
    transcript::{Detail, Diff, DiffLine, Item, LineKind, Meta, Target, Work, WorkRow},
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
            Entry::Prompt { text, steered } => items.push(Item::Prompt { text: text.clone().into(), steered: *steered }),
            Entry::Compacted => items.push(Item::Compacted),
            Entry::Notice { text, failed } => items.push(Item::Notice { text: text.clone().into(), failed: *failed }),
            Entry::Turn(turn) => {
                let key = format!("turn-{index}");
                if !turn.steps.is_empty() || turn.live {
                    let rows: Vec<WorkRow> = turn
                        .steps
                        .iter()
                        .enumerate()
                        .map(|(step, item)| step_row(item, &format!("{key}-{step}"), cwd, turn.live, open))
                        .collect();
                    items.push(Item::Work(Work {
                        open: turn.live || open.contains(&key),
                        summary: summary(turn, now).into(),
                        tally: tally(&rows).into(),
                        live: turn.live,
                        rows,
                        key: key.into(),
                    }));
                }
                // A plan proposed in prose shows as the card, not twice.
                let reply = match &turn.proposal {
                    Some(_) => without_proposed_plan(&turn.reply),
                    None => turn.reply.clone(),
                };
                if !reply.trim().is_empty() {
                    items.push(Item::Reply(cache.get(index, &reply)));
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
    const KINDS: [(&[&str], &str, &str); 5] = [
        (&["Read"], "read", "reads"),
        (&["Searched", "Found", "Listed", "Looked for"], "search", "searches"),
        (&["Edited", "Wrote"], "edit", "edits"),
        (&["Ran"], "command", "commands"),
        (&["Asked"], "question", "questions"),
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

fn step_row(step: &Step, key: &str, cwd: &Path, live: bool, open: &HashSet<String>) -> WorkRow {
    let mut row = match step {
        Step::Thought(text) => WorkRow {
            key: key.to_string().into(),
            verb: "Thought".into(),
            target: Target::Words(first_line(text).into()),
            meta: Meta::None,
            detail: (text.trim().contains('\n') || text.trim().len() > 120).then(|| Detail::Text(text.trim().to_string().into())),
            open: false,
        },
        Step::Note(text) => WorkRow {
            key: key.to_string().into(),
            verb: "".into(),
            target: Target::Note(text.clone().into()),
            meta: Meta::None,
            detail: None,
            open: false,
        },
        Step::Event(text) => WorkRow {
            key: key.to_string().into(),
            verb: "".into(),
            target: Target::Words(text.clone().into()),
            meta: Meta::None,
            detail: None,
            open: false,
        },
        Step::Tool(tool) => tool_row(tool, cwd, live),
    };
    row.open = row.detail.is_some() && open.contains(row.key.as_ref());
    row
}

fn first_line(text: &str) -> String {
    text.trim().lines().next().unwrap_or("").trim().to_string()
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
            let output = result.map(|result| result.text.trim_end().to_string()).filter(|text| !text.is_empty() && text != "(No output.)");
            (
                "Ran",
                Target::Code(first_line(command).into()),
                None,
                output.map(|text| Detail::Output(tail(&text, DETAIL_LINES).into())),
            )
        }
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
    WorkRow { key: tool.id.clone().into(), verb: verb.to_string().into(), target, meta, detail, open: false }
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wack_core::Usage;

    use super::*;
    use crate::conversation::ToolResult;

    #[test]
    fn a_tally_counts_what_was_done_and_leaves_thoughts_out() {
        let row = |verb: &str| WorkRow {
            key: "k".into(),
            verb: verb.to_string().into(),
            target: Target::Words("".into()),
            meta: Meta::None,
            detail: None,
            open: false,
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
            result: result.map(|(text, is_error)| ToolResult { text: text.into(), is_error }),
        }
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
}
