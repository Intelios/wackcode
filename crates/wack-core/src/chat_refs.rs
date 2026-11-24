//! `@` chat references in the Chat area, ported from version 2 (`src/chat-refs.ts`,
//! `worker/src/builtin/chat-refs.ts`, `src-tauri/src/chat_refs.rs`).
//!
//! The message keeps `@"Chat name"` as plain text, and one generated tail section carries the
//! ids. Nothing from the other chat rides the prompt: the agent calls `read_chat` with an id when
//! that history matters, so referencing a long chat costs a sentence until it is read.
//!
//! ```text
//! …the user's own words…
//!
//! <referenced-chats-v1>
//! Chats the user referenced with @. Call read_chat with a chat id when its history is
//! relevant; nothing from it is included here.
//! {"version":1,"chats":[{"id":"chat-…","name":"Trip ideas"}]}
//! </referenced-chats-v1>
//! ```
//!
//! Only a tail that parses through to the very end of the message counts, the transcript splits
//! it back off ([`split`]), and anything that fails validation stays in the text untouched.
//!
//! Who may read whom is decided when the run starts: a Chat chat reads only the Chat area's other
//! chats ([`Store::readable_chats`]), so a project's material never reaches the Chat area. The
//! tool takes an id, never a path, and reads the session file without writing to it.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wack_agent::{HistoryEntry, Loading, Tool, ToolContext, ToolOutput, ToolSpec, history};
use wack_llm::{Block, Role};

use crate::store::{Chat, Store};

pub const READ_CHAT: &str = "read_chat";
const OPEN: &str = "<referenced-chats-v1>";
const CLOSE: &str = "</referenced-chats-v1>";
/// One message can reference this many chats.
pub const MAX_CHAT_REFS: usize = 16;
/// A chat's name is a label, not content: anything longer is not a name the app saved.
const MAX_NAME_CHARS: usize = 200;
/// The only guidance the model gets about the section, so it is short and complete.
const GUIDANCE: &str = "Chats the user referenced with @. Call read_chat with a chat id when its history is relevant; nothing from it is included here.";
/// One page of a chat's history, in characters.
const PAGE_CHARS: usize = 40_000;
/// A tool result shown with `tools`, at most.
const MAX_RESULT_CHARS: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    chats: Vec<ChatRef>,
}

/// The token the composer and the transcript both use: always quoted, so spaces keep it whole.
pub fn mention(name: &str) -> String {
    format!("@\"{}\"", name.replace(['"', '\n'], ""))
}

/// The message with the referenced-chats section appended. References are de-duplicated by id
/// and capped; with none usable the text is returned as it was.
pub fn encode(text: &str, refs: &[ChatRef]) -> String {
    let mut chats: Vec<ChatRef> = Vec::new();
    for chat in refs {
        let name: String = chat.name.chars().take(MAX_NAME_CHARS).collect();
        if chat.id.is_empty() || name.is_empty() || chats.iter().any(|seen| seen.id == chat.id) || chats.len() >= MAX_CHAT_REFS {
            continue;
        }
        chats.push(ChatRef { id: chat.id.clone(), name });
    }
    if chats.is_empty() {
        return text.to_string();
    }
    let payload = serde_json::to_string(&Payload { version: 1, chats }).unwrap_or_default();
    let section = format!("{OPEN}\n{GUIDANCE}\n{payload}\n{CLOSE}");
    if text.is_empty() { section } else { format!("{text}\n\n{section}") }
}

/// The user's own words and the chats they referenced. A tail that does not parse to the end of
/// the message is left in the words.
pub fn split(text: &str) -> (String, Vec<ChatRef>) {
    let unchanged = || (text.to_string(), Vec::new());
    let Some(close) = text.rfind(CLOSE) else { return unchanged() };
    if !text[close + CLOSE.len()..].trim().is_empty() {
        return unchanged();
    }
    let Some(open) = text[..close].rfind(OPEN) else { return unchanged() };
    let refs = text[open + OPEN.len()..close]
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with('{'))
        .and_then(|line| serde_json::from_str::<Payload>(line).ok())
        .filter(valid);
    match refs {
        Some(payload) => (text[..open].trim_end().to_string(), payload.chats),
        None => unchanged(),
    }
}

fn valid(payload: &Payload) -> bool {
    payload.version == 1
        && !payload.chats.is_empty()
        && payload.chats.len() <= MAX_CHAT_REFS
        && payload.chats.iter().enumerate().all(|(at, chat)| {
            !chat.id.is_empty()
                && !chat.name.is_empty()
                && chat.name.chars().count() <= MAX_NAME_CHARS
                && !payload.chats[..at].iter().any(|seen| seen.id == chat.id)
        })
}

/// The `@word` the caret is in, as a byte range of `text` (the `@` to the end of the word), and
/// what was typed after the `@` up to the caret. Version 2's `activeMention`: an `@` at the start
/// or after a space or an opening bracket; a quoted `@"…` stays open until its closing quote, so
/// a name with spaces can be typed; a finished `@"Name"` before the caret is not a query.
pub fn active_mention(text: &str, caret: usize) -> Option<(std::ops::Range<usize>, String)> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let before = &text[..caret];
    let at_token_start = |index: usize| index == 0 || text[..index].chars().next_back().is_some_and(|c| c.is_whitespace() || "([{\"'`".contains(c));
    if let Some(quote) = before.rfind("@\"")
        && at_token_start(quote)
    {
        let query = &before[quote + 2..];
        if !query.contains(['"', '\n']) {
            let end = if text[caret..].starts_with('"') { caret + 1 } else { caret };
            return Some((quote..end, query.to_string()));
        }
    }
    let start = before.char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(at, c)| at + c.len_utf8());
    let start = if text[start..].starts_with(['(', '[', '{', '"', '\'', '`']) && text[start + 1..].starts_with('@') { start + 1 } else { start };
    if start >= caret || !text[start..].starts_with('@') || !at_token_start(start) {
        return None;
    }
    let query = &before[start + 1..];
    if query.contains(['"', '\'', '`', '(', ')', '[', ']', '{', '}']) {
        return None;
    }
    let end = text[caret..].find(char::is_whitespace).map_or(text.len(), |at| caret + at);
    Some((start..end, query.to_string()))
}

/// How well a chat's name matches what was typed after `@` (lower case); 0 is not at all.
fn score(name: &str, query: &str) -> u32 {
    let name = name.to_lowercase();
    if name == query {
        100
    } else if name.starts_with(query) {
        80
    } else if name.contains(query) {
        60
    } else {
        // Letters scattered across a name match almost anything, so this counts for little.
        let mut letters = name.chars();
        if query.chars().all(|wanted| letters.any(|c| c == wanted)) { 10 } else { 0 }
    }
}

/// The chats an `@` query offers, best first. `chats` come newest first and keep that order
/// between equal matches; with nothing typed yet, the newest `limit` are offered.
pub fn rank(chats: &[ChatRef], query: &str, limit: usize) -> Vec<ChatRef> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return chats.iter().take(limit).cloned().collect();
    }
    let mut scored: Vec<(u32, usize, &ChatRef)> =
        chats.iter().enumerate().map(|(at, chat)| (score(&chat.name, &query), at, chat)).filter(|(score, ..)| *score > 0).collect();
    scored.sort_by_key(|(score, at, _)| (std::cmp::Reverse(*score), *at));
    scored.into_iter().take(limit).map(|(.., chat)| chat.clone()).collect()
}

/// The picked references a message still deserves: deleting a `@"Name"` from the text drops
/// that chat from the section.
pub fn surviving(text: &str, refs: &[ChatRef]) -> Vec<ChatRef> {
    refs.iter().filter(|chat| text.contains(&mention(&chat.name))).cloned().collect()
}

/// A chat a run may read: its name and where its session is.
#[derive(Clone, Debug, PartialEq)]
pub struct Readable {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub archived: bool,
    pub session: PathBuf,
}

impl Store {
    /// The chats `chat`'s run may read with `read_chat`: the Chat area's others, archived ones
    /// too. A Code chat gets none (the tool is the Chat area's).
    pub fn readable_chats(&self, chat: &Chat) -> Vec<Readable> {
        if !chat.is_chat_area() {
            return Vec::new();
        }
        self.all_chats()
            .filter(|other| other.is_chat_area() && other.id != chat.id)
            .map(|other| Readable {
                id: other.id.clone(),
                name: other.title.clone(),
                model: other.choice.model.clone(),
                archived: other.is_archived(),
                session: self.session_path(other),
            })
            .collect()
    }
}

/// `read_chat`: another chat's history by its id, paged.
pub struct ReadChat {
    chats: Arc<Vec<Readable>>,
}

impl ReadChat {
    pub fn new(chats: Vec<Readable>) -> Self {
        Self { chats: Arc::new(chats) }
    }
}

#[async_trait]
impl Tool for ReadChat {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: READ_CHAT.into(),
            description: "Read another chat's history, by the id from the referenced-chats section of the user's message. Returns its messages oldest first, paged; tool calls are named, and their results are included only with tools.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["chat"],
                "properties": {
                    "chat": { "type": "string", "description": "The chat id from the referenced-chats section of the user's message." },
                    "offset": { "type": "integer", "minimum": 0, "description": "Character offset to continue a long chat from, as given by the previous result. Omit to start at the top." },
                    "tools": { "type": "boolean", "description": "Include tool results (truncated), not just the tool calls." }
                }
            }),
            loading: Loading::Deferred,
            parallel_safe: true,
            hint: "read another chat the user referenced with @".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let id = input.get("chat").and_then(Value::as_str).map(str::trim).filter(|id| !id.is_empty()).ok_or("chat must be the id from the referenced-chats section.")?;
        let offset = match input.get("offset") {
            None | Some(Value::Null) => 0,
            Some(value) => value.as_u64().ok_or("offset must be a non-negative integer.")? as usize,
        };
        let tools = input.get("tools").and_then(Value::as_bool).unwrap_or(false);
        let chat = self.chats.iter().find(|chat| chat.id == id).ok_or("There is no such chat this chat may read: Chat chats can only read other Chat chats.")?;
        let path = chat.session.clone();
        let entries = tokio::task::spawn_blocking(move || if path.exists() { history(&path) } else { Ok(Vec::new()) })
            .await
            .map_err(|_| "That chat's saved history could not be read.".to_string())?
            .map_err(|_| "That chat's saved history could not be read.".to_string())?;
        Ok(page(&render(chat, &entries, tools), offset).into())
    }
}

/// A chat's history as the model reads it.
pub fn render(chat: &Readable, entries: &[HistoryEntry], tools: bool) -> String {
    let mut blocks = Vec::new();
    let (mut users, mut assistants) = (0, 0);
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for entry in entries {
        match entry {
            HistoryEntry::Compacted => blocks.push("## The conversation before this point was summarised for the model.".to_string()),
            HistoryEntry::Message { message, .. } => match message.role {
                Role::User => {
                    let results: Vec<String> = message
                        .content
                        .iter()
                        .filter_map(|block| match block {
                            Block::ToolResult { tool_use_id, content, .. } if tools => {
                                let name = names.get(tool_use_id).cloned().unwrap_or_else(|| "tool".into());
                                let cut: String = content.chars().take(MAX_RESULT_CHARS).collect();
                                let more = if cut.len() < content.len() { "\n[… result cut]" } else { "" };
                                Some(format!("[tool result: {name}]\n{}{more}", cut.trim()))
                            }
                            _ => None,
                        })
                        .collect();
                    let text = split(message.text().trim()).0;
                    if !text.trim().is_empty() {
                        users += 1;
                        blocks.push(format!("## User\n\n{}", text.trim()));
                    } else if message.content.iter().any(|block| matches!(block, Block::Image { .. })) {
                        users += 1;
                        blocks.push("## User\n\n[a picture]".to_string());
                    }
                    blocks.extend(results);
                }
                Role::Assistant => {
                    let mut parts = vec![message.text().trim().to_string()];
                    for block in &message.content {
                        if let Block::ToolUse { id, name, .. } = block {
                            names.insert(id.clone(), name.clone());
                            parts.push(format!("[called {name}]"));
                        }
                    }
                    let body = parts.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("\n\n");
                    assistants += 1;
                    blocks.push(format!("## Assistant\n\n{}", if body.is_empty() { "[no text]".into() } else { body }));
                }
            },
            HistoryEntry::Note { .. } | HistoryEntry::State { .. } => {}
        }
    }
    let mut header = vec![format!("# Chat: {}", chat.name)];
    if let Some(model) = &chat.model {
        header.push(format!("Model: {model}"));
    }
    if chat.archived {
        header.push("Archived: yes".into());
    }
    header.push(format!("Messages: {} ({users} user, {assistants} assistant)", users + assistants));
    if blocks.is_empty() {
        blocks.push("[This chat has no saved messages.]".into());
    }
    let mut text = header.join("\n");
    for block in blocks {
        text.push_str("\n\n");
        text.push_str(&block);
    }
    text
}

/// One page from `offset` (in characters), and how to go on.
fn page(full: &str, offset: usize) -> String {
    let total = full.chars().count();
    if offset > 0 && offset >= total {
        return format!("[offset {offset} is past the end of this chat ({total} characters).]");
    }
    let text: String = full.chars().skip(offset).take(PAGE_CHARS).collect();
    let end = offset + text.chars().count();
    if end < total {
        format!("{text}\n\n[Showing characters {offset}-{end} of {total}. Use offset={end} to continue.]")
    } else if offset > 0 {
        format!("{text}\n\n[Showing characters {offset}-{end} of {total}; this is the end of this chat.]")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use wack_llm::Message;

    use super::*;

    #[test]
    fn the_caret_finds_the_mention_it_is_in() {
        assert_eq!(active_mention("@", 1), Some((0..1, String::new())));
        assert_eq!(active_mention("ask @tri about", 8), Some((4..8, "tri".into())));
        assert_eq!(active_mention("ask @trip about", 7), Some((4..9, "tr".into())), "the word runs on past the caret");
        assert_eq!(active_mention("(@tr", 4), Some((1..4, "tr".into())));
        assert_eq!(active_mention("see @\"Trip id", 13), Some((4..13, "Trip id".into())), "a quoted name may have spaces");
        assert_eq!(active_mention("see @\"Trip\" now", 10), Some((4..11, "Trip".into())), "the closing quote is part of it");
        assert_eq!(active_mention("see @\"Trip\" now", 15), None, "a finished mention is not a query");
        assert_eq!(active_mention("me@example.com", 14), None, "an @ inside a word is not a mention");
        assert_eq!(active_mention("plain words", 5), None);
        assert_eq!(active_mention("caf\u{e9} @x", 8), Some((6..8, "x".into())));
    }

    #[test]
    fn chats_rank_by_match_then_recency() {
        let chats: Vec<ChatRef> = ["Lisbon trip", "Trip ideas", "Strip lights", "Tea", "trip"]
            .iter()
            .enumerate()
            .map(|(at, name)| ChatRef { id: format!("c{at}"), name: name.to_string() })
            .collect();
        let names = |query: &str| rank(&chats, query, 3).into_iter().map(|chat| chat.name).collect::<Vec<_>>();
        assert_eq!(names("trip"), ["trip", "Trip ideas", "Lisbon trip"]);
        assert_eq!(names(""), ["Lisbon trip", "Trip ideas", "Strip lights"], "nothing typed offers the newest");
        assert_eq!(names("TEA")[0], "Tea", "a name beats letters scattered across another");
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn deleting_a_mention_drops_its_reference() {
        let refs = vec![ChatRef { id: "a".into(), name: "Trip".into() }, ChatRef { id: "b".into(), name: "Tea \"time\"".into() }];
        assert_eq!(surviving("about @\"Trip\"", &refs), refs[..1]);
        assert_eq!(surviving("@\"Tea time\" and @\"Trip\"", &refs), refs);
        assert!(surviving("nothing", &refs).is_empty());
    }

    fn refs(names: &[(&str, &str)]) -> Vec<ChatRef> {
        names.iter().map(|(id, name)| ChatRef { id: id.to_string(), name: name.to_string() }).collect()
    }

    #[test]
    fn a_tail_round_trips_and_only_a_whole_tail_counts() {
        let sent = encode("Compare with @\"Trip ideas\"", &refs(&[("chat-1", "Trip ideas"), ("chat-1", "Again"), ("", "No id")]));
        assert!(sent.ends_with(CLOSE));
        let (words, chats) = split(&sent);
        assert_eq!(words, "Compare with @\"Trip ideas\"");
        assert_eq!(chats, refs(&[("chat-1", "Trip ideas")]), "de-duplicated, the unusable dropped");
        assert_eq!(encode("plain", &[]), "plain");
        // Words after the tail, or a broken payload, leave the text as sent.
        for broken in [format!("{sent}\nmore"), sent.replace("\"version\":1", "\"version\":2"), sent.replace("chat-1", "")] {
            assert_eq!(split(&broken), (broken.clone(), Vec::new()));
        }
    }

    #[test]
    fn references_are_capped() {
        let many: Vec<ChatRef> = (0..40).map(|at| ChatRef { id: format!("c{at}"), name: format!("n{at}") }).collect();
        assert_eq!(split(&encode("x", &many)).1.len(), MAX_CHAT_REFS);
        assert_eq!(mention("a \"b\"\nc"), "@\"a bc\"");
    }

    #[test]
    fn a_chat_chat_reads_only_the_chat_areas_other_chats() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut store = Store::open(data.path()).unwrap();
        let project = store.add_project(project.path()).unwrap();
        let code = store.create_chat(&project.id, "Code", Default::default(), wack_agent::AgentMode::Build, None).unwrap();
        let me = store.create_area_chat("Me", Default::default()).unwrap();
        let other = store.create_area_chat("Other", Default::default()).unwrap();
        store.set_archived(&other.id, true).unwrap();
        let readable = store.readable_chats(&me);
        assert_eq!(readable.iter().map(|chat| chat.id.as_str()).collect::<Vec<_>>(), [other.id.as_str()]);
        assert!(readable[0].archived);
        assert!(store.readable_chats(&code).is_empty(), "the tool is the Chat area's");
    }

    #[tokio::test]
    async fn read_chat_renders_another_chat_and_refuses_the_rest() {
        let data = tempfile::tempdir().unwrap();
        let session = data.path().join("other.jsonl");
        let mut file = wack_agent::Session::create(&session, data.path()).unwrap();
        file.push(Message::user(encode("Plan a trip to Lisbon", &refs(&[("x", "X")]))), None).unwrap();
        file.push(
            Message {
                role: Role::Assistant,
                content: vec![
                    Block::Text { text: "Here is a plan.".into() },
                    Block::ToolUse { id: "t1".into(), name: "web_search".into(), input: json!({}) },
                ],
            },
            None,
        )
        .unwrap();
        let chat = Readable { id: "chat-2".into(), name: "Lisbon".into(), model: Some("m".into()), archived: false, session };
        let tool = ReadChat::new(vec![chat]);
        let context = ToolContext::detached(data.path());
        let text = tool.run(json!({ "chat": "chat-2" }), &context).await.unwrap().text;
        assert!(text.starts_with("# Chat: Lisbon\nModel: m\nMessages: 2 (1 user, 1 assistant)"), "{text}");
        assert!(text.contains("## User\n\nPlan a trip to Lisbon") && !text.contains("referenced-chats"), "{text}");
        assert!(text.contains("Here is a plan.\n\n[called web_search]"), "{text}");
        assert!(tool.run(json!({ "chat": "chat-9" }), &context).await.unwrap_err().contains("only read other Chat chats"));
        assert!(tool.run(json!({ "chat": "chat-2", "offset": -1 }), &context).await.is_err());
    }

    #[test]
    fn long_chats_page_by_character() {
        let full = "x".repeat(PAGE_CHARS + 10);
        assert!(page(&full, 0).ends_with(&format!("Use offset={PAGE_CHARS} to continue.]")));
        assert!(page(&full, PAGE_CHARS).contains("this is the end of this chat"));
        assert!(page(&full, PAGE_CHARS + 10).contains("past the end"));
    }
}
