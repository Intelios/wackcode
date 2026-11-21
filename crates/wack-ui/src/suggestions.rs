//! Suggested tasks: the follow-ups the agent filed with `suggest_task` (wack-agent's
//! `suggest.rs`), the card each one draws under the turn's final message, and the message a
//! Start action sends.
//!
//! Ported from version 2's `src/suggestions.ts` with its rules. The tool call's arguments are
//! the card: title, kind and a self-contained brief. The session saves them with the call, so a
//! saved chat and a cold reopen need nothing extra. A settled, successful call is a card; a
//! refused or unfinished one stays an ordinary tool row inside the turn's work.
//!
//! A Start action sends the suggestion as a user message with one generated tail section:
//!
//! ```text
//! Dismiss button stays disabled after a failed save
//!
//! **Where:** `src/components/Composer.tsx` … (the agent's own brief)
//!
//! <suggested-task-v1>
//! Started from a suggested task "Dismiss button stays disabled after a failed save" in the
//! chat "Settings cleanup".
//! {"version":1,"id":"call-abc","kind":"bug","title":"…","origin":{"chatId":"t1","chatName":"…"}}
//! </suggested-task-v1>
//! ```
//!
//! The id is the tool-call id, which is the suggestion's identity everywhere: the origin chat's
//! card finds the new chat through `Chat::origin`, and "started here" is read back out of this
//! chat's own later messages, so a card the user acted on stays marked. Only a tail that parses
//! through to the end of the message counts, and anything that fails validation stays in the
//! text untouched — the transcript then shows the words as sent.

use std::collections::{HashMap, HashSet};

use gpui::SharedString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wack_core::{Chat, suggest::SUGGEST_TASK};

use crate::conversation::ToolStep;

pub const SUGGESTED_TASK_OPEN: &str = "<suggested-task-v1>";
pub const SUGGESTED_TASK_CLOSE: &str = "</suggested-task-v1>";
/// One message's brief is bounded by the tool itself; anything longer is not a suggestion.
const MAX_BRIEF_CHARS: usize = 4_000;
const MAX_TITLE_CHARS: usize = 80;
const MAX_ID_CHARS: usize = 128;
const MAX_NAME_CHARS: usize = 200;

/// How `suggest_task` classified a follow-up: broken, rough, or merely missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Bug,
    Polish,
    Improvement,
}

impl Kind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "bug" => Self::Bug,
            "polish" => Self::Polish,
            "improvement" => Self::Improvement,
            _ => return None,
        })
    }

    /// The word each kind wears on its card.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bug => "Bug",
            Self::Polish => "Polish",
            Self::Improvement => "Improvement",
        }
    }
}

/// A suggestion as the tray draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// The `suggest_task` tool-call id: the suggestion's identity everywhere.
    pub id: String,
    pub title: String,
    pub kind: Kind,
    /// The agent's Markdown brief, meant to be readable on its own.
    pub description: String,
}

/// One title, as both the card and the duplicate read it: one line, one space between words.
fn clean_title(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One `suggest_task` call as a card: its arguments plus the settled result that says the call
/// was accepted. `None` while it runs, when it was refused, or when its arguments do not
/// parse — the ordinary tool row already says what happened.
pub fn card_from_step(step: &ToolStep) -> Option<Card> {
    if step.name != SUGGEST_TASK || step.id.is_empty() {
        return None;
    }
    let result = step.result.as_ref()?;
    if result.is_error {
        return None;
    }
    let input = step.input.as_ref()?;
    let title = input["title"].as_str().map(clean_title).unwrap_or_default();
    let description = input["description"].as_str().map(str::trim).unwrap_or_default();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS || description.is_empty() || description.chars().count() > MAX_BRIEF_CHARS {
        return None;
    }
    let kind = input["kind"].as_str().and_then(Kind::parse)?;
    Some(Card { id: step.id.clone(), title, kind, description: description.to_string() })
}

/// The origin of a started suggestion, as the tail carries it.
#[derive(Clone, Debug, PartialEq)]
pub struct Started {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// A chat the suggestion started elsewhere named, for the "from …" caption.
    pub origin: Option<Origin>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub chat: String,
    pub name: String,
}

/// The title as the tail's guidance sentence spells it: one line, no quotes that end it.
fn tail_title(title: &str) -> String {
    title
        .split(|character: char| character.is_whitespace() || character == '"')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect()
}

/// The message a Start action sends. `origin` is present only for a new chat: it names the chat
/// the suggestion was found in, which the card that sent it can link back to.
pub fn compose(card: &Card, origin: Option<Origin>) -> String {
    let title = tail_title(&card.title);
    let brief = card.description.trim();
    let words = [title.as_str(), brief].iter().filter(|part| !part.is_empty()).cloned().collect::<Vec<_>>().join("\n\n");
    let sentence = match &origin {
        Some(origin) => format!("Started from a suggested task \"{title}\" in the chat \"{}\".", tail_title(&origin.name)),
        None => format!("Started from a suggested task \"{title}\"."),
    };
    let mut payload = json!({ "version": 1, "id": card.id, "kind": kind_name(card.kind), "title": title });
    if let Some(origin) = origin {
        payload["origin"] = json!({ "chatId": origin.chat, "chatName": origin.name });
    }
    format!("{words}\n\n{SUGGESTED_TASK_OPEN}\n{sentence}\n{payload}\n{SUGGESTED_TASK_CLOSE}")
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Bug => "bug",
        Kind::Polish => "polish",
        Kind::Improvement => "improvement",
    }
}

/// The section's payload: the last line that parses as a versioned record — the guidance is prose.
fn parse_section(body: &str) -> Option<Started> {
    let line = body.lines().rev().find(|line| line.trim().starts_with('{'))?.trim();
    let record: Value = serde_json::from_str(line).ok()?;
    let record = record.as_object()?;
    if record.get("version")?.as_u64() != Some(1) {
        return None;
    }
    let id = record.get("id")?.as_str()?;
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
        return None;
    }
    let title = record.get("title")?.as_str()?;
    if title.trim().is_empty() {
        return None;
    }
    let kind = record.get("kind")?.as_str().and_then(Kind::parse)?;
    let origin = match record.get("origin") {
        None => None,
        Some(origin) => {
            let chat = origin.get("chatId")?.as_str()?;
            let name = origin.get("chatName")?.as_str()?;
            if chat.is_empty() || name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
                return None;
            }
            Some(Origin { chat: chat.to_string(), name: name.to_string() })
        }
    };
    Some(Started { id: id.to_string(), kind, title: title.to_string(), origin })
}

/// A message's own words and the suggestion it started, or the text unchanged when it carries
/// no tail. A tail anywhere at the end counts, so a message edited down to its section still
/// splits.
pub fn split(text: &str) -> (String, Option<Started>) {
    let Some(close) = text.rfind(SUGGESTED_TASK_CLOSE) else {
        return (text.to_string(), None);
    };
    // Anything after the closing tag means this was not a generated tail at all.
    if !text[close + SUGGESTED_TASK_CLOSE.len()..].trim().is_empty() {
        return (text.to_string(), None);
    }
    let Some(open) = text[..close].rfind(SUGGESTED_TASK_OPEN) else {
        return (text.to_string(), None);
    };
    let inner = &text[open + SUGGESTED_TASK_OPEN.len()..close];
    let Some(suggestion) = parse_section(inner) else {
        return (text.to_string(), None);
    };
    let body = text[..open].trim_end().to_string();
    if body.is_empty() {
        return (text.to_string(), None);
    }
    (body, Some(suggestion))
}

/// What a card's state comes from besides the transcript: the chat's record and its queue.
/// "Started here" is not in here — the conversation reads it from its own messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// Suggestion ids the user dismissed in this chat (`Chat::dismissed_suggestions`).
    pub dismissed: HashSet<String>,
    /// Suggestion ids queued to send to this chat once the run ends.
    pub queued: HashSet<String>,
    /// Suggestion ids that opened a chat of their own: suggestion → (chat id, chat title).
    pub links: HashMap<String, (String, String)>,
}

/// Every chat in `chats` started from a suggestion in the chat `from`, keyed by suggestion id.
/// The origin card reads this to say "Opened in …"; a chat the user deleted simply isn't here,
/// which re-opens the card.
pub fn links(chats: &[&Chat], from: &str) -> HashMap<String, (String, String)> {
    chats
        .iter()
        .filter_map(|chat| match &chat.origin {
            Some(origin) if origin.chat == from => Some((origin.suggestion.clone(), (chat.id.clone(), chat.title.clone()))),
            _ => None,
        })
        .collect()
}

/// The caption under a message that started a suggested task.
pub fn sent_label(origin: Option<&Origin>) -> SharedString {
    match origin {
        Some(origin) => format!("Suggested task · from “{}”", origin.name).into(),
        None => "Suggested task".into(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn card() -> Card {
        Card {
            id: "call-1".into(),
            title: "Dismiss button stays disabled".into(),
            kind: Kind::Bug,
            description: "**Where:** `src/Composer.tsx` …".into(),
        }
    }

    #[test]
    fn a_composed_tail_splits_back() {
        let message = compose(&card(), None);
        let (text, suggestion) = split(&message);
        assert_eq!(text, "Dismiss button stays disabled\n\n**Where:** `src/Composer.tsx` …");
        let suggestion = suggestion.unwrap();
        assert_eq!(suggestion.id, "call-1");
        assert_eq!(suggestion.kind, Kind::Bug);
        assert!(suggestion.origin.is_none());
    }

    #[test]
    fn the_origin_rides_the_tail() {
        let message = compose(&card(), Some(Origin { chat: "chat-1".into(), name: "Settings cleanup".into() }));
        let (_, suggestion) = split(&message);
        let origin = suggestion.unwrap().origin.unwrap();
        assert_eq!((origin.chat.as_str(), origin.name.as_str()), ("chat-1", "Settings cleanup"));
        assert!(message.contains("in the chat \"Settings cleanup\""));
        assert!(!message.contains("read_chat"));
    }

    #[test]
    fn only_a_tail_at_the_end_counts() {
        let message = compose(&card(), None);
        // Text after the closing tag means the words are just text.
        let (text, suggestion) = split(&format!("{message}\nAfterthought"));
        assert!(suggestion.is_none());
        assert!(text.contains(SUGGESTED_TASK_CLOSE));
        // So does a payload that fails validation.
        let broken = message.replace("\"kind\":\"bug\"", "\"kind\":\"idea\"");
        assert!(split(&broken).1.is_none());
        // And a message with no body before the section.
        let section = format!("{SUGGESTED_TASK_OPEN}\nStarted from a suggested task \"x\".\n{{\"version\":1,\"id\":\"a\",\"kind\":\"bug\",\"title\":\"x\"}}\n{SUGGESTED_TASK_CLOSE}");
        assert_eq!(split(&section).0, section);
    }

    #[test]
    fn a_message_edited_down_to_its_section_still_splits() {
        let message = compose(&card(), None);
        let tail = &message[message.find(SUGGESTED_TASK_OPEN).unwrap()..];
        // A body is still required.
        let (_, suggestion) = split(&format!("words\n{tail}"));
        assert_eq!(suggestion.unwrap().id, "call-1");
    }

    #[test]
    fn a_call_is_a_card_only_once_accepted() {
        let running = ToolStep { id: "call-1".into(), name: SUGGEST_TASK.into(), input: Some(json!({ "title": "t", "kind": "bug", "description": "d" })), result: None, writing: None };
        assert!(card_from_step(&running).is_none());
        let mut refused = running.clone();
        refused.result = Some(crate::conversation::ToolResult { text: "no".into(), is_error: true, pictures: Vec::new() });
        assert!(card_from_step(&refused).is_none());
        let mut accepted = running;
        accepted.result = Some(crate::conversation::ToolResult { text: "ok".into(), is_error: false, pictures: Vec::new() });
        assert_eq!(card_from_step(&accepted).unwrap().title, "t");
        // An input still being written, or not ours, is not a card.
        assert!(card_from_step(&ToolStep { id: "x".into(), name: "bash".into(), input: None, result: accepted.result.clone(), writing: None }).is_none());
    }

    #[test]
    fn links_find_only_this_chats_started_chats() {
        let chats = vec![
            Chat {
                id: "a".into(),
                project: "p".into(),
                title: "Started one".into(),
                session: "a.jsonl".into(),
                created: 0,
                updated: 0,
                choice: Default::default(),
                mode: Default::default(),
                layout: None,
                browser_tabs: None,
                dismissed_suggestions: vec![],
                origin: Some(wack_core::SuggestionOrigin { chat: "origin".into(), suggestion: "s1".into() }),
            },
            Chat {
                id: "b".into(),
                project: "p".into(),
                title: "Someone else's".into(),
                session: "b.jsonl".into(),
                created: 0,
                updated: 0,
                choice: Default::default(),
                mode: Default::default(),
                layout: None,
                browser_tabs: None,
                dismissed_suggestions: vec![],
                origin: Some(wack_core::SuggestionOrigin { chat: "other".into(), suggestion: "s2".into() }),
            },
        ];
        let refs: Vec<&Chat> = chats.iter().collect();
        let map = links(&refs, "origin");
        assert_eq!(map.len(), 1);
        assert_eq!(map["s1"], ("a".to_string(), "Started one".to_string()));
    }
}
