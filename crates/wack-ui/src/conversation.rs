//! A chat as the transcript shows it: built from the saved history, then kept current from a
//! run's events. Pure data, no drawing, so it is tested here.
//!
//! The rules both paths share:
//! - A user's message starts a turn. A message sent while the agent works ("steering") ends the
//!   current turn and starts the next, so the transcript reads in the order things happened.
//! - Inside a turn, thinking, tool calls and any text written before a tool call are the turn's
//!   work. Text after the last tool call is the reply. While a reply streams it is the reply; if
//!   a tool call follows, it moves into the work as a note.

use serde_json::Value;
use wack_core::{Event, HistoryEntry, Message, Outcome, RetryNotice, Role, Usage, llm::Block};

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Prompt { text: String, steered: bool },
    Turn(Turn),
    /// The older conversation was summarised here.
    Compacted,
    /// Something about the chat rather than a turn: a manual compaction under way or failed.
    Notice { text: String, failed: bool },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub steps: Vec<Step>,
    /// Text since the last tool call.
    pub reply: String,
    /// Milliseconds since 1970, when known.
    pub started: Option<u64>,
    pub ended: Option<u64>,
    pub usage: Usage,
    pub live: bool,
    pub end: Option<TurnEnd>,
    /// A retry is pending: what went wrong and when it tries again.
    pub retrying: Option<String>,
    /// Live thinking deltas join the last thought until anything else happens.
    thinking_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Thought(String),
    /// Text the model wrote between tool calls.
    Note(String),
    Tool(ToolStep),
    /// Something the loop did on its own, such as compacting.
    Event(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolStep {
    pub id: String,
    pub name: String,
    /// `None` while the call is still being written.
    pub input: Option<Value>,
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub text: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TurnEnd {
    Done,
    Stopped,
    CutOff,
    Refused,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub entries: Vec<Entry>,
}

impl Turn {
    fn new(started: Option<u64>, live: bool) -> Self {
        Self { started, live, ..Self::default() }
    }

    /// Moves the reply so far into the work, because a tool call or thinking follows it.
    fn settle_reply(&mut self) {
        let text = std::mem::take(&mut self.reply);
        if !text.trim().is_empty() {
            self.steps.push(Step::Note(text.trim().to_string()));
        }
    }

    fn push_text(&mut self, text: &str) {
        self.thinking_open = false;
        self.retrying = None;
        self.reply.push_str(text);
    }

    fn push_thinking(&mut self, text: &str, streaming: bool) {
        self.retrying = None;
        self.settle_reply();
        match self.steps.last_mut() {
            Some(Step::Thought(thought)) if streaming && self.thinking_open => thought.push_str(text),
            _ => self.steps.push(Step::Thought(text.to_string())),
        }
        self.thinking_open = streaming;
    }

    fn tool(&mut self, id: &str) -> Option<&mut ToolStep> {
        self.steps.iter_mut().rev().find_map(|step| match step {
            Step::Tool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
    }

    fn push_tool(&mut self, id: &str, name: &str, input: Option<Value>) {
        self.thinking_open = false;
        self.retrying = None;
        self.settle_reply();
        match self.tool(id) {
            Some(tool) => {
                if input.is_some() {
                    tool.input = input;
                }
            }
            None => self.steps.push(Step::Tool(ToolStep { id: id.into(), name: name.into(), input, result: None })),
        }
    }

    fn push_result(&mut self, id: &str, text: &str, is_error: bool) {
        self.thinking_open = false;
        if let Some(tool) = self.tool(id) {
            tool.result = Some(ToolResult { text: text.to_string(), is_error });
        }
    }

    fn add_usage(&mut self, usage: Option<Usage>) {
        self.thinking_open = false;
        if let Some(usage) = usage {
            self.usage.input += usage.input;
            self.usage.output += usage.output;
            self.usage.cache_read += usage.cache_read;
            self.usage.cache_write += usage.cache_write;
        }
    }

    /// Tool calls still waiting for a result.
    pub fn running_tools(&self) -> impl Iterator<Item = &ToolStep> {
        self.steps.iter().filter_map(|step| match step {
            Step::Tool(tool) if tool.result.is_none() => Some(tool),
            _ => None,
        })
    }
}

/// "Rate limited: trying again in 4s (attempt 2)".
fn retry_text(notice: &RetryNotice) -> String {
    let reason = notice.reason.trim_end_matches('.');
    format!("{reason}: trying again in {}s (attempt {})", notice.wait.as_secs().max(1), notice.attempt)
}

impl Conversation {
    /// The transcript of a saved chat.
    pub fn from_history(history: &[HistoryEntry]) -> Self {
        let mut conversation = Self::default();
        for entry in history {
            match entry {
                HistoryEntry::Compacted => {
                    conversation.close_turn(None);
                    conversation.entries.push(Entry::Compacted);
                }
                HistoryEntry::Message { message, usage, at } => conversation.replay(message, *usage, *at),
            }
        }
        conversation.close_turn(None);
        conversation
    }

    fn replay(&mut self, message: &Message, usage: Option<Usage>, at: Option<u64>) {
        match message.role {
            Role::User => {
                let results: Vec<_> = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        Block::ToolResult { tool_use_id, content, is_error } => Some((tool_use_id, content, *is_error)),
                        _ => None,
                    })
                    .collect();
                let text = message.text();
                if results.is_empty() {
                    self.begin(&text, false, at, false);
                    return;
                }
                if let Some(turn) = self.current() {
                    for (id, content, is_error) in results {
                        turn.push_result(id, content, is_error);
                    }
                    if at.is_some() {
                        turn.ended = at;
                    }
                }
                // Text beside tool results was sent while the agent worked.
                if !text.trim().is_empty() {
                    self.begin(&text, true, at, false);
                }
            }
            Role::Assistant => {
                let Some(turn) = self.current() else {
                    return;
                };
                for block in &message.content {
                    match block {
                        Block::Text { text } => turn.push_text(text),
                        Block::Thinking { text, .. } if !text.trim().is_empty() => turn.push_thinking(text, false),
                        Block::ToolUse { id, name, input } => turn.push_tool(id, name, Some(input.clone())),
                        _ => {}
                    }
                }
                turn.add_usage(usage);
                if at.is_some() {
                    turn.ended = at;
                }
            }
        }
    }

    /// The turn still being written to, if the last entry is one.
    fn current(&mut self) -> Option<&mut Turn> {
        match self.entries.last_mut() {
            Some(Entry::Turn(turn)) => Some(turn),
            _ => None,
        }
    }

    pub fn last_turn(&self) -> Option<&Turn> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Turn(turn) => Some(turn),
            _ => None,
        })
    }

    /// Ends the current turn as finished, unless it already ended.
    fn close_turn(&mut self, at: Option<u64>) {
        if let Some(turn) = self.current() {
            turn.live = false;
            turn.thinking_open = false;
            turn.retrying = None;
            if at.is_some() {
                turn.ended = at;
            }
            if turn.end.is_none() {
                turn.end = Some(TurnEnd::Done);
            }
        }
    }

    fn begin(&mut self, text: &str, steered: bool, at: Option<u64>, live: bool) {
        self.close_turn(if steered { at } else { None });
        self.entries.push(Entry::Prompt { text: text.to_string(), steered });
        self.entries.push(Entry::Turn(Turn::new(at, live)));
    }

    /// The user sent a message and a run is starting.
    pub fn send(&mut self, text: &str, now: u64) {
        self.begin(text, false, Some(now), true);
    }

    pub fn apply(&mut self, event: &Event, now: u64) {
        if let Event::Steered(text) = event {
            self.begin(text, true, Some(now), true);
            return;
        }
        let Some(turn) = self.current() else {
            return;
        };
        match event {
            Event::Text(text) => turn.push_text(text),
            Event::Thinking(text) => turn.push_thinking(text, true),
            Event::ToolStart { id, name } => turn.push_tool(id, name, None),
            Event::ToolInput { .. } => {}
            Event::ToolCall { id, name, input } => turn.push_tool(id, name, Some(input.clone())),
            Event::ToolResult { id, content, is_error, .. } => turn.push_result(id, content, *is_error),
            Event::Reply { usage } => turn.add_usage(*usage),
            Event::Retrying(notice) => turn.retrying = Some(retry_text(notice)),
            Event::Compacting => {
                turn.settle_reply();
                turn.steps.push(Step::Event("Compacting the conversation to make room".into()));
            }
            Event::Compacted { .. } => {
                if let Some(Step::Event(text)) = turn.steps.last_mut() {
                    *text = "Compacted the conversation to make room".into();
                }
            }
            Event::Steered(_) => {}
        }
    }

    /// The run ended.
    pub fn finish(&mut self, result: &Result<Outcome, String>, now: u64) {
        let end = match result {
            Ok(Outcome::Done) => TurnEnd::Done,
            Ok(Outcome::Stopped) => TurnEnd::Stopped,
            Ok(Outcome::CutOff) => TurnEnd::CutOff,
            Ok(Outcome::Refused) => TurnEnd::Refused,
            Err(message) => TurnEnd::Failed(message.clone()),
        };
        if let Some(turn) = self.current() {
            turn.end = Some(end);
        }
        self.close_turn(Some(now));
    }

    /// A compaction the user asked for has started.
    pub fn begin_compaction(&mut self) {
        self.entries.push(Entry::Notice { text: "Compacting the conversation…".into(), failed: false });
    }

    pub fn finish_compaction(&mut self, result: &Result<Outcome, String>) {
        if matches!(self.entries.last(), Some(Entry::Notice { failed: false, .. })) {
            self.entries.pop();
        }
        match result {
            Ok(Outcome::Done) => self.entries.push(Entry::Compacted),
            Ok(_) => {}
            Err(message) => self.entries.push(Entry::Notice { text: message.clone(), failed: true }),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn assistant(blocks: Vec<Block>) -> Message {
        Message { role: Role::Assistant, content: blocks }
    }

    fn results(pairs: &[(&str, &str)], steered: Option<&str>) -> Message {
        let mut content: Vec<Block> = pairs
            .iter()
            .map(|(id, text)| Block::ToolResult { tool_use_id: (*id).into(), content: (*text).into(), is_error: false })
            .collect();
        if let Some(text) = steered {
            content.push(Block::Text { text: text.into() });
        }
        Message { role: Role::User, content }
    }

    fn at(message: Message, ms: u64, usage: Option<Usage>) -> HistoryEntry {
        HistoryEntry::Message { message, usage, at: Some(ms) }
    }

    fn turns(conversation: &Conversation) -> Vec<&Turn> {
        conversation.entries.iter().filter_map(|entry| match entry {
            Entry::Turn(turn) => Some(turn),
            _ => None,
        }).collect()
    }

    #[test]
    fn history_folds_narration_into_work_and_keeps_the_last_text_as_the_reply() {
        let usage = Usage { input: 10, output: 5, cache_read: 0, cache_write: 0 };
        let history = vec![
            at(Message::user("fix it"), 1_000, None),
            at(
                assistant(vec![
                    Block::Thinking { text: "look first".into(), signature: None },
                    Block::Text { text: "I'll read the file.".into() },
                    Block::ToolUse { id: "a".into(), name: "read".into(), input: json!({ "path": "x.rs" }) },
                ]),
                3_000,
                Some(usage),
            ),
            at(results(&[("a", "contents")], None), 3_500, None),
            at(assistant(vec![Block::Text { text: "Done.".into() }]), 9_000, Some(usage)),
        ];
        let conversation = Conversation::from_history(&history);
        assert!(matches!(&conversation.entries[0], Entry::Prompt { text, steered: false } if text == "fix it"));
        let turn = turns(&conversation)[0];
        assert!(matches!(&turn.steps[0], Step::Thought(text) if text == "look first"));
        assert!(matches!(&turn.steps[1], Step::Note(text) if text == "I'll read the file."));
        assert!(matches!(&turn.steps[2], Step::Tool(tool) if tool.result.as_ref().unwrap().text == "contents"));
        assert_eq!(turn.reply, "Done.");
        assert_eq!((turn.started, turn.ended), (Some(1_000), Some(9_000)));
        assert_eq!(turn.usage.output, 10);
        assert_eq!(turn.end, Some(TurnEnd::Done));
        assert!(!turn.live);
    }

    #[test]
    fn a_message_sent_mid_run_starts_the_next_turn() {
        let history = vec![
            at(Message::user("first"), 0, None),
            at(assistant(vec![Block::ToolUse { id: "a".into(), name: "bash".into(), input: json!({}) }]), 1, None),
            at(results(&[("a", "ok")], Some("actually, b.txt")), 2, None),
            at(assistant(vec![Block::Text { text: "Renamed.".into() }]), 3, None),
        ];
        let conversation = Conversation::from_history(&history);
        let kinds: Vec<&str> = conversation
            .entries
            .iter()
            .map(|entry| match entry {
                Entry::Prompt { steered: true, .. } => "steered",
                Entry::Prompt { .. } => "prompt",
                Entry::Turn(_) => "turn",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["prompt", "turn", "steered", "turn"]);
        assert_eq!(turns(&conversation)[1].reply, "Renamed.");
    }

    #[test]
    fn live_events_stream_into_the_same_shape() {
        let mut conversation = Conversation::default();
        conversation.send("go", 100);
        for event in [
            Event::Thinking("hm".into()),
            Event::Thinking("m".into()),
            Event::Text("Checking".into()),
            Event::Text(" now.".into()),
            Event::ToolStart { id: "t".into(), name: "bash".into() },
            Event::ToolCall { id: "t".into(), name: "bash".into(), input: json!({ "command": "ls" }) },
        ] {
            conversation.apply(&event, 200);
        }
        {
            let turn = conversation.last_turn().unwrap();
            assert!(turn.live);
            assert!(matches!(&turn.steps[0], Step::Thought(text) if text == "hmm"));
            assert!(matches!(&turn.steps[1], Step::Note(text) if text == "Checking now."));
            assert_eq!(turn.running_tools().count(), 1);
            assert!(turn.reply.is_empty());
        }
        conversation.apply(&Event::Retrying(RetryNotice { attempt: 2, wait: std::time::Duration::from_secs(2), reason: "Rate limited.".into() }), 250);
        assert_eq!(conversation.last_turn().unwrap().retrying.as_deref(), Some("Rate limited: trying again in 2s (attempt 2)"));
        conversation.apply(&Event::ToolResult { id: "t".into(), name: "bash".into(), content: "a\nb".into(), is_error: false }, 300);
        conversation.apply(&Event::Text("All good.".into()), 400);
        conversation.finish(&Ok(Outcome::Stopped), 500);
        let turn = conversation.last_turn().unwrap();
        assert_eq!(turn.reply, "All good.");
        assert_eq!(turn.end, Some(TurnEnd::Stopped));
        assert_eq!((turn.started, turn.ended), (Some(100), Some(500)));
        assert!(!turn.live && turn.retrying.is_none());
    }

    #[test]
    fn a_failed_run_keeps_its_message_and_compaction_marks_its_place() {
        let mut conversation = Conversation::default();
        conversation.send("go", 0);
        conversation.finish(&Err("The key was refused.".into()), 1);
        assert_eq!(conversation.last_turn().unwrap().end, Some(TurnEnd::Failed("The key was refused.".into())));

        conversation.begin_compaction();
        conversation.finish_compaction(&Ok(Outcome::Done));
        assert_eq!(conversation.entries.last(), Some(&Entry::Compacted));
        conversation.begin_compaction();
        conversation.finish_compaction(&Err("There is nothing to compact yet.".into()));
        assert!(matches!(conversation.entries.last(), Some(Entry::Notice { failed: true, .. })));
    }
}
