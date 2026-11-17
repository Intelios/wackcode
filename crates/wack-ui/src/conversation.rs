//! A chat as the transcript shows it: built from the saved history, then kept current from a
//! run's events. Pure data, no drawing, so it is tested here.
//!
//! The rules both paths share:
//! - A user's message starts a turn. A message sent while the agent works ("steering") ends the
//!   current turn and starts the next, so the transcript reads in the order things happened.
//! - Inside a turn, thinking, tool calls and any text written before a tool call are the turn's
//!   work. Text after the last tool call is the reply. While a reply streams it is the reply; if
//!   a tool call follows, it moves into the work as a note.
//! - Feature state saved in the session (`plan`, `todo`) is folded in as it comes, so the mode in
//!   force, the plan awaiting review and the todo list read the same after a relaunch as live.
//!   A plan is shown on the turn that proposed it; only the one still awaiting a decision
//!   (`awaiting`) offers actions. A new message, Discard or Approve clears it.
//! - A background task's news (a note tagged `task:`) is shown where the model read it: in the
//!   work of a turn still working, or, when the model had finished its reply (the chat was idle
//!   and woke), as its own entry followed by the turn that answers it, with no prompt before it.

use serde_json::Value;
use wack_core::{
    AgentMode, Event, HistoryEntry, Message, Outcome, RetryNotice, Role, Usage,
    llm::Block,
    plan::{PLAN_STATE, PlanState},
    skills::CommandUse,
    tasks::NOTE_PREFIX,
    agents::{AgentInfo, NOTE_PREFIX as AGENT_NOTE_PREFIX},
    mcp_client::NOTE_PREFIX as MCP_NOTE_PREFIX,
    todo::{TODO_STATE, TodoList},
};

use crate::suggestions::{self, Started, View as SuggestionView};

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// What the user sent. For a slash command `text` is the expanded prompt the model got and
    /// `command` what the user typed, which the transcript shows instead. `suggestion` is the
    /// suggested task the message started, its tail already split from `text`.
    Prompt { text: String, steered: bool, command: Option<CommandUse>, suggestion: Option<Started> },
    Turn(Turn),
    /// The older conversation was summarised here.
    Compacted,
    /// Something about the chat rather than a turn: a manual compaction under way or failed.
    Notice { text: String, failed: bool },
    /// A background task reported while the chat was idle.
    News(News),
}

/// A background task's news as the model read it.
#[derive(Clone, Debug, PartialEq)]
pub struct News {
    /// `task:<id>:<what>`, or `agent:<id>` for a sub-agent's report (`agent:<id>:interrupted`
    /// for one a quit stopped).
    pub tag: String,
    pub text: String,
}

impl News {
    /// The sub-agent this is from, if it is a sub-agent's.
    pub fn agent(&self) -> Option<&str> {
        self.tag.strip_prefix(AGENT_NOTE_PREFIX).map(|rest| rest.split(':').next().unwrap_or_default())
    }

    /// The task's id, from the tag.
    pub fn task(&self) -> &str {
        self.tag.strip_prefix(NOTE_PREFIX).and_then(|rest| rest.split(':').next()).unwrap_or_default()
    }

    /// What happened: `end`, `lines` or `lost`.
    pub fn what(&self) -> &str {
        self.tag.rsplit(':').next().unwrap_or_default()
    }
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
    /// The plan this turn proposed for review.
    pub proposal: Option<Proposal>,
    /// Things the app told the user during the turn, such as a plan it could not read. Shown
    /// after the reply; not saved, so they do not come back after a relaunch.
    pub notices: Vec<String>,
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
    /// A background task reported while the turn worked.
    Task(News),
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

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub text: String,
    /// Proposed in Ultra Plan: drawn in the prism.
    pub ultra: bool,
}

/// The plan card's passing state: a Discard waiting to be confirmed, a word after Copy or Save.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanView {
    pub confirming_discard: bool,
    pub notice: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub entries: Vec<Entry>,
    /// The mode in force and the plan awaiting review, as the session last recorded them.
    pub plan: PlanState,
    /// The entry whose turn holds the plan awaiting review.
    pub awaiting: Option<usize>,
    pub todos: TodoList,
    pub plan_view: PlanView,
    /// The chat's sub-agents as its registry has them now, by the call that started each one's
    /// latest round. Kept by the chat, not folded from events: they work outside the run.
    pub agents: std::collections::HashMap<String, AgentInfo>,
    /// Suggested tasks this chat's own messages started, by suggestion id. Read from the
    /// transcript the way version 2 did, so it is right after a relaunch too.
    pub started: std::collections::HashSet<String>,
    /// What cards' state comes from besides the transcript — dismissals, queue and opened
    /// chats — kept by the chat, which knows the store and its queue.
    pub suggestion_view: SuggestionView,
    /// MCP notes saved before the message of the run they belong to, held for its turn.
    held_notices: Vec<String>,
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
                HistoryEntry::Message { message, usage, at, command } => conversation.replay(message, *usage, *at, command.clone()),
                HistoryEntry::State { key, value } => conversation.apply_state(key, value),
                HistoryEntry::Note { tag, text, at } if tag.starts_with(NOTE_PREFIX) || tag.starts_with(AGENT_NOTE_PREFIX) => {
                    conversation.news(News { tag: tag.clone(), text: text.clone() }, *at, false)
                }
                // A server that failed or stopped: the model was told, and the transcript says so.
                HistoryEntry::Note { tag, text, .. } if tag.starts_with(MCP_NOTE_PREFIX) => conversation.held_notices.push(text.clone()),
                HistoryEntry::Note { .. } => {}
            }
        }
        conversation.flush_notices();
        conversation.close_turn(None);
        conversation
    }

    fn replay(&mut self, message: &Message, usage: Option<Usage>, at: Option<u64>, command: Option<CommandUse>) {
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
                    self.begin(&text, false, at, false, command);
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
                    self.begin(&text, true, at, false, None);
                }
            }
            Role::Assistant => {
                self.flush_notices();
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

    /// Folds in a feature's saved state.
    pub fn apply_state(&mut self, key: &str, value: &Value) {
        match key {
            PLAN_STATE => {
                let Some(state) = PlanState::from_value(value) else {
                    return;
                };
                match &state.plan {
                    Some(text) if self.plan.plan.as_ref() != Some(text) || self.awaiting.is_none() => {
                        let ultra = state.mode == AgentMode::UltraPlan;
                        let index = self.entries.iter().rposition(|entry| matches!(entry, Entry::Turn(_)));
                        if let Some(index) = index
                            && let Entry::Turn(turn) = &mut self.entries[index]
                        {
                            turn.proposal = Some(Proposal { text: text.clone(), ultra });
                        }
                        self.awaiting = index;
                        self.plan_view = PlanView::default();
                    }
                    Some(_) => {}
                    None => {
                        self.awaiting = None;
                        self.plan_view = PlanView::default();
                    }
                }
                self.plan = state;
            }
            TODO_STATE => self.todos = TodoList::from_value(value).unwrap_or_default(),
            _ => {}
        }
    }

    /// The plan awaiting review, if any.
    pub fn awaiting_plan(&self) -> Option<&Proposal> {
        match self.entries.get(self.awaiting?) {
            Some(Entry::Turn(turn)) => turn.proposal.as_ref(),
            _ => None,
        }
    }

    /// The user discarded the plan: back to Build, the proposal kept but inert.
    pub fn discard_plan(&mut self) {
        self.plan = PlanState { mode: AgentMode::Build, plan: None };
        self.awaiting = None;
        self.plan_view = PlanView::default();
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

    /// Shows a background task's news: in the work of a turn that is working, or on its own
    /// with a fresh turn after it when the model had finished (the chat woke for it).
    fn news(&mut self, news: News, at: Option<u64>, live: bool) {
        let after_prompt = matches!(self.entries.iter().rev().nth(1), Some(Entry::Prompt { .. }));
        let working = match self.current() {
            None => false,
            Some(turn) => {
                let empty = turn.steps.is_empty() && turn.reply.trim().is_empty();
                if empty { after_prompt } else { turn.reply.trim().is_empty() || turn.running_tools().next().is_some() }
            }
        };
        if working {
            if let Some(turn) = self.current() {
                turn.thinking_open = false;
                turn.steps.push(Step::Task(news));
            }
            return;
        }
        // An empty turn waiting for this news (a wake) moves after it.
        let waiting = match self.entries.last() {
            Some(Entry::Turn(turn)) if turn.steps.is_empty() && turn.reply.trim().is_empty() => Some((turn.started, turn.live)),
            _ => None,
        };
        let (started, live) = match waiting {
            Some(waiting) => {
                self.entries.pop();
                waiting
            }
            None => {
                self.close_turn(None);
                (at, live)
            }
        };
        self.entries.push(Entry::News(news));
        self.entries.push(Entry::Turn(Turn::new(started.or(at), live)));
    }

    /// The chat woke to tell the model what its background tasks reported.
    pub fn wake(&mut self, now: u64) {
        self.close_turn(None);
        self.entries.push(Entry::Turn(Turn::new(Some(now), true)));
    }

    fn begin(&mut self, text: &str, steered: bool, at: Option<u64>, live: bool, command: Option<CommandUse>) {
        // A turn left waiting after news, which nothing answered, is not shown.
        if let Some(Entry::Turn(turn)) = self.entries.last()
            && !turn.live
            && turn.steps.is_empty()
            && turn.reply.trim().is_empty()
            && matches!(self.entries.iter().rev().nth(1), Some(Entry::News(_)))
        {
            self.entries.pop();
        }
        self.close_turn(if steered { at } else { None });
        // A message that started a suggested task carries a generated tail: it never shows, its
        // words are the prompt, and the suggestion reads as started here.
        let (text, suggestion) = match command {
            Some(_) => (text.to_string(), None),
            None => suggestions::split(text),
        };
        if let Some(started) = &suggestion {
            self.started.insert(started.id.clone());
            self.suggestion_view.queued.remove(&started.id);
        }
        self.entries.push(Entry::Prompt { text, steered, command, suggestion });
        let mut turn = Turn::new(at, live);
        turn.notices = std::mem::take(&mut self.held_notices);
        self.entries.push(Entry::Turn(turn));
    }

    /// Hands held MCP notes to the turn now going.
    fn flush_notices(&mut self) {
        if self.held_notices.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.held_notices);
        match self.current() {
            Some(turn) => turn.notices.extend(held),
            None => self.held_notices = held,
        }
    }

    /// The user sent a message and a run is starting.
    pub fn send(&mut self, text: &str, now: u64) {
        self.begin(text, false, Some(now), true, None);
    }

    /// The user sent a slash command: `text` is what the model gets.
    pub fn send_command(&mut self, text: &str, command: CommandUse, now: u64) {
        self.begin(text, false, Some(now), true, Some(command));
    }

    pub fn apply(&mut self, event: &Event, now: u64) {
        if let Event::Steered(text) = event {
            self.begin(text, true, Some(now), true, None);
            return;
        }
        if let Event::Noted { tag, text } = event {
            if tag.starts_with(NOTE_PREFIX) || tag.starts_with(AGENT_NOTE_PREFIX) {
                self.news(News { tag: tag.clone(), text: text.clone() }, Some(now), true);
            } else if tag.starts_with(MCP_NOTE_PREFIX) {
                self.held_notices.push(text.clone());
                self.flush_notices();
            }
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
            Event::State { key, value } => {
                let (key, value) = (key.clone(), value.clone());
                self.apply_state(&key, &value);
            }
            // Never settles the reply: a notice at the end of a run must not fold it away.
            Event::Notice(text) => turn.notices.push(text.clone()),
            Event::Steered(_) | Event::Question { .. } | Event::Noted { .. } => {}
        }
    }

    /// The suggested task a call of this chat filed, by its tool-call id.
    pub fn suggestion(&self, id: &str) -> Option<suggestions::Card> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Turn(turn) => turn.steps.iter().find_map(|step| match step {
                Step::Tool(tool) if tool.id == id => suggestions::card_from_step(tool),
                _ => None,
            }),
            _ => None,
        })
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

    #[test]
    fn a_notice_at_the_end_of_a_run_keeps_the_reply() {
        let mut conversation = Conversation::default();
        conversation.send("Plan it", 1);
        conversation.apply(&Event::Text("Here is my plan, badly tagged.".into()), 2);
        conversation.apply(&Event::Notice("The plan could not be read.".into()), 3);
        conversation.finish(&Ok(Outcome::Done), 4);
        let Some(Entry::Turn(turn)) = conversation.entries.last() else { panic!("no turn") };
        assert_eq!(turn.reply, "Here is my plan, badly tagged.");
        assert!(turn.steps.is_empty());
        assert_eq!(turn.notices, vec!["The plan could not be read."]);
    }

    fn plan_state(mode: &str, plan: Option<&str>) -> Event {
        let mut value = json!({ "mode": mode });
        if let Some(plan) = plan {
            value["plan"] = json!(plan);
        }
        Event::State { key: PLAN_STATE.into(), value }
    }

    #[test]
    fn a_proposed_plan_awaits_review_until_a_new_message() {
        let mut conversation = Conversation::default();
        conversation.send("Plan the cache", 1);
        conversation.apply(&plan_state("ultraplan", None), 2);
        conversation.apply(&plan_state("ultraplan", Some("1. Do it")), 3);
        conversation.finish(&Ok(Outcome::Done), 4);
        assert_eq!(conversation.awaiting, Some(1));
        assert_eq!(conversation.awaiting_plan().map(|plan| (plan.text.as_str(), plan.ultra)), Some(("1. Do it", true)));
        assert_eq!(conversation.plan.mode, AgentMode::UltraPlan);

        // The same state saved again changes nothing; a new message consumes it.
        conversation.apply_state(PLAN_STATE, &json!({ "mode": "ultraplan", "plan": "1. Do it" }));
        assert_eq!(conversation.awaiting, Some(1));
        conversation.send("Make it smaller", 5);
        conversation.apply(&plan_state("ultraplan", None), 6);
        assert_eq!(conversation.awaiting, None);
        // The earlier proposal stays on its turn, inert.
        assert!(matches!(&conversation.entries[1], Entry::Turn(turn) if turn.proposal.is_some()));
    }

    #[test]
    fn discarding_returns_to_build_and_saved_state_replays() {
        let history = vec![
            HistoryEntry::Message { message: Message::user("Plan it"), usage: None, at: Some(1), command: None },
            HistoryEntry::Message { message: assistant(vec![Block::Text { text: "Here it is.".into() }]), usage: None, at: Some(2), command: None },
            HistoryEntry::State { key: PLAN_STATE.into(), value: json!({ "mode": "plan", "plan": "Steps" }) },
            HistoryEntry::State { key: TODO_STATE.into(), value: json!({ "tasks": [{ "id": 1, "subject": "Write", "status": "pending" }], "nextId": 2 }) },
        ];
        let mut conversation = Conversation::from_history(&history);
        assert_eq!(conversation.awaiting, Some(1));
        assert_eq!(conversation.todos.tasks.len(), 1);
        conversation.discard_plan();
        assert_eq!(conversation.awaiting, None);
        assert_eq!(conversation.plan.mode, AgentMode::Build);
    }

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
        HistoryEntry::Message { message, usage, at: Some(ms), command: None }
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
        assert!(matches!(&conversation.entries[0], Entry::Prompt { text, steered: false, .. } if text == "fix it"));
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

    #[test]
    fn a_command_replays_as_typed() {
        let command = CommandUse { name: "pdf".into(), args: "form.pdf".into(), kind: wack_core::skills::Kind::Skill };
        let history = vec![HistoryEntry::Message { message: Message::user("<skill name=\"pdf\">…</skill>\n\nform.pdf"), usage: None, at: Some(1), command: Some(command.clone()) }];
        let conversation = Conversation::from_history(&history);
        assert!(matches!(&conversation.entries[0], Entry::Prompt { text, command: Some(used), .. } if text.starts_with("<skill") && *used == command));
    }

    fn noted(tag: &str, text: &str) -> HistoryEntry {
        HistoryEntry::Note { tag: tag.into(), text: text.into(), at: Some(5) }
    }

    #[test]
    fn task_news_mid_run_joins_the_work() {
        let mut conversation = Conversation::default();
        conversation.send("Build it", 1);
        conversation.apply(&Event::ToolCall { id: "a".into(), name: "bash".into(), input: json!({ "command": "make" }) }, 2);
        conversation.apply(&Event::ToolResult { id: "a".into(), name: "bash".into(), content: "ok".into(), is_error: false }, 3);
        conversation.apply(&Event::Noted { tag: "task:ab12:end".into(), text: "Background task ab12 (`npm test`) finished: exit code 0 after 4s.".into() }, 4);
        conversation.apply(&Event::Noted { tag: "contract:plan".into(), text: "Plan rules".into() }, 4);
        let Some(Entry::Turn(turn)) = conversation.entries.last() else { panic!("no turn") };
        assert!(matches!(&turn.steps[1], Step::Task(news) if news.task() == "ab12" && news.what() == "end"));
        assert_eq!(turn.steps.len(), 2, "other notes stay hidden");
    }

    #[test]
    fn news_after_a_finished_reply_stands_alone_and_its_answer_follows_live_and_replayed() {
        let news = "Background task ab12 (`cargo build`) failed: exit code 101 after 1m32s.";
        let history = vec![
            HistoryEntry::Message { message: Message::user("Build it in the background"), usage: None, at: Some(1), command: None },
            HistoryEntry::Message { message: assistant(vec![Block::Text { text: "Started it.".into() }]), usage: None, at: Some(2), command: None },
            noted("task:ab12:end", news),
            HistoryEntry::Message { message: assistant(vec![Block::Text { text: "The build failed; fixing it.".into() }]), usage: None, at: Some(6), command: None },
        ];
        let replayed = Conversation::from_history(&history);

        let mut live = Conversation::default();
        live.send("Build it in the background", 1);
        live.apply(&Event::Text("Started it.".into()), 2);
        live.finish(&Ok(Outcome::Done), 3);
        live.wake(4);
        live.apply(&Event::Noted { tag: "task:ab12:end".into(), text: news.into() }, 5);
        live.apply(&Event::Text("The build failed; fixing it.".into()), 6);
        live.finish(&Ok(Outcome::Done), 7);

        for conversation in [&replayed, &live] {
            assert_eq!(conversation.entries.len(), 4, "{:?}", conversation.entries);
            assert!(matches!(&conversation.entries[2], Entry::News(item) if item.text == news));
            assert!(matches!(&conversation.entries[3], Entry::Turn(turn) if turn.reply == "The build failed; fixing it."));
        }
        let Entry::Turn(first) = &live.entries[1] else { panic!() };
        assert_eq!(first.reply, "Started it.", "the earlier reply stays a reply");
    }

    #[test]
    fn a_message_that_started_a_suggestion_keeps_its_words_and_records_the_start() {
        let card = suggestions::Card {
            id: "sug-1".into(),
            kind: suggestions::Kind::Bug,
            title: "Fix the flash".into(),
            description: "The queue row blinks.".into(),
        };
        let text = suggestions::compose(&card, None);

        let mut live = Conversation::default();
        live.send(&text, 10);
        let replayed = Conversation::from_history(&[at(Message::user(&text), 10, None)]);
        for conversation in [&live, &replayed] {
            assert!(matches!(
                conversation.entries.first(),
                Some(Entry::Prompt { text: prompt, suggestion: Some(started), .. })
                    if prompt.as_str() == "Fix the flash\n\nThe queue row blinks." && started.id == "sug-1"
            ));
            assert!(conversation.started.contains("sug-1"), "a replay finds the start too");
        }

        // A slash command never splits, so a suggested title leading with '/' stays a command's text.
        let command = CommandUse { name: "pdf".into(), args: String::new(), kind: wack_core::skills::Kind::Skill };
        let mut conversation = Conversation::default();
        conversation.send_command(&text, command, 10);
        assert!(matches!(
            conversation.entries.first(),
            Some(Entry::Prompt { text: prompt, suggestion: None, .. }) if prompt == &text
        ));
        assert!(conversation.started.is_empty());
    }

    #[test]
    fn news_nobody_answered_leaves_no_empty_turn_before_the_next_message() {
        let history = vec![
            HistoryEntry::Message { message: Message::user("Serve it"), usage: None, at: Some(1), command: None },
            HistoryEntry::Message { message: assistant(vec![Block::Text { text: "It is up.".into() }]), usage: None, at: Some(2), command: None },
            noted("task:ab12:lost", "Background task ab12 (`npm run dev`) is no longer running: it was stopped when WackCode quit."),
            HistoryEntry::Message { message: Message::user("Start it again"), usage: None, at: Some(9), command: None },
        ];
        let conversation = Conversation::from_history(&history);
        assert!(matches!(&conversation.entries[2], Entry::News(_)));
        assert!(matches!(&conversation.entries[3], Entry::Prompt { text, .. } if text == "Start it again"));
    }

    #[test]
    fn an_mcp_server_failure_is_a_notice_in_the_turn_it_happened_in() {
        let failed = "MCP server \"GitHub\" isn't available: Command not found: npx. Check it in Settings › MCP servers.";
        // Saved before the user's message: it belongs to the run that message started.
        let history = vec![
            at(Message::user("first"), 1, None),
            at(assistant(vec![Block::Text { text: "Done.".into() }]), 2, None),
            noted("mcp:mcp-1:0000000000000003", failed),
            at(Message::user("second"), 3, None),
            at(assistant(vec![Block::Text { text: "Again.".into() }]), 4, None),
        ];
        let replayed = Conversation::from_history(&history);
        let turns = turns(&replayed);
        assert!(turns[0].notices.is_empty());
        assert_eq!(turns[1].notices, [failed]);

        let mut live = Conversation::default();
        live.send("second", 3);
        live.apply(&Event::Noted { tag: "mcp:mcp-1:0000000000000003".into(), text: failed.into() }, 3);
        live.apply(&Event::Text("Again.".into()), 4);
        assert_eq!(live.last_turn().unwrap().notices, [failed]);
    }
}
