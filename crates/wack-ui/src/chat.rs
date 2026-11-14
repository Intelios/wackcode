//! One chat's live state: its conversation, the transcript items drawn from it, its composer, and
//! the run in progress. Owned by [`crate::app::AppState`] for as long as the window is open, so
//! switching chats never stops a run or loses a draft.
//!
//! Invariants:
//! - `items` is always what `work::items` would build from `conversation`, and `spans[i]` is how
//!   many of them entry `i` made. Only entries that changed are rebuilt, and `list` is told
//!   exactly which items changed, so a long chat costs nothing extra while a reply streams.
//!   Events only ever change the conversation's last entry or add entries after it.
//! - One run at a time. The run itself is on Tokio; its events come back through a channel and are
//!   applied a batch at a time. Dropping this model drops the run's task, which stops the run.
//! - Messages written while the agent works either wait in `queue` (↩, sent after the run) or go
//!   to the model straight away as steering (⌘↩); `steering` holds those not yet delivered.
//!   Neither is ever silently dropped: when a run ends without sending them, the app puts them
//!   back in the composer.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{AppContext, Context, Entity, EventEmitter, ListAlignment, ListState, SharedString, Task, px};
use gpui_tokio::Tokio;
use wack_core::{Control, Event, HistoryEntry, Job, Outcome, RunEvent, RunSetup};

use crate::{
    conversation::{Conversation, Entry},
    text_field::TextField,
    transcript::Item,
    typography::Type,
    work::{self, ReplyCache},
};

pub const FOLLOW_UP_PLACEHOLDER: &str = "Reply, or ask for the next change";

/// What a chat tells the app.
#[derive(Clone, Debug)]
pub enum ChatEvent {
    /// A run ended, with how.
    Finished(Result<Outcome, String>),
}

pub struct ChatSession {
    pub id: String,
    cwd: PathBuf,
    conversation: Conversation,
    /// Keys of the work logs and rows the user opened.
    open: HashSet<String>,
    cache: ReplyCache,
    items: Vec<Item>,
    spans: Vec<usize>,
    pub list: ListState,
    pub composer: Entity<TextField>,
    /// Sent one at a time after the run ends.
    pub queue: Vec<String>,
    /// Handed to the run, not yet delivered to the model.
    pub steering: Vec<String>,
    run: Option<Running>,
    /// Numbers runs, so a finished run's task can never act on the run after it.
    runs_started: u64,
    /// How full the model's context is, in tokens, when known.
    pub context_tokens: Option<u64>,
    /// Finished while the user was elsewhere.
    pub unread: bool,
}

struct Running {
    number: u64,
    control: Control,
    compacting: bool,
    events: Task<()>,
    _tick: Task<()>,
}

impl EventEmitter<ChatEvent> for ChatSession {}

pub fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_millis() as u64).unwrap_or(0)
}

impl ChatSession {
    /// A chat as saved in `session_path`, or an empty one when it has never run.
    pub fn load(id: &str, cwd: PathBuf, session_path: &std::path::Path, cx: &mut Context<Self>) -> Self {
        let mut context_tokens = None;
        let conversation = if session_path.exists() {
            match wack_core::history(session_path) {
                Ok(history) => {
                    context_tokens = last_context(&history);
                    Conversation::from_history(&history)
                }
                Err(message) => {
                    let mut conversation = Conversation::default();
                    conversation.entries.push(Entry::Notice { text: message, failed: true });
                    conversation
                }
            }
        } else {
            Conversation::default()
        };
        let composer = cx.new(|cx| TextField::new(Type::Body, FOLLOW_UP_PLACEHOLDER, cx).multiline(12));
        let mut chat = Self {
            id: id.to_string(),
            cwd,
            conversation,
            open: HashSet::new(),
            cache: ReplyCache::default(),
            items: Vec::new(),
            spans: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(800.)),
            composer,
            queue: Vec::new(),
            steering: Vec::new(),
            run: None,
            runs_started: 0,
            context_tokens,
            unread: false,
        };
        chat.refresh_from(0);
        chat
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.conversation.entries.is_empty()
    }

    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    pub fn compacting(&self) -> bool {
        self.run.as_ref().is_some_and(|run| run.compacting)
    }

    /// Starts a run: the user's message, or a compaction.
    pub fn start(&mut self, setup: RunSetup, job: Job, cx: &mut Context<Self>) {
        if self.run.is_some() {
            return;
        }
        let from = self.conversation.entries.len();
        let compacting = matches!(job, Job::Compact);
        match &job {
            Job::Prompt(text) => self.conversation.send(text, now_millis()),
            Job::Compact => self.conversation.begin_compaction(),
        }

        self.runs_started += 1;
        let number = self.runs_started;
        let control = Control::new();
        let (sender, mut receiver) = mpsc::unbounded::<RunEvent>();
        let sink: Arc<dyn Fn(RunEvent) + Send + Sync> = Arc::new(move |event| {
            let _ = sender.unbounded_send(event);
        });
        let run = Tokio::spawn(cx, wack_core::run(setup, job, control.clone(), sink));
        let events = cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                let mut batch = vec![first];
                while let Ok(event) = receiver.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |chat, cx| chat.apply(number, batch, cx)).is_err() {
                    return;
                }
            }
            // The channel closes when the run is over. A run always reports how it ended, so
            // reaching here with this run still live means its task died.
            let outcome = run.await;
            let _ = this.update(cx, |chat, cx| {
                if chat.run.as_ref().is_some_and(|run| run.number == number) {
                    let message = match outcome {
                        Err(error) if error.is_panic() => "The agent crashed. The chat is saved up to its last reply.",
                        _ => "The run ended without saying how. The chat is saved up to its last reply.",
                    };
                    chat.apply(number, vec![RunEvent::Finished { result: Err(message.into()), context_tokens: None }], cx);
                }
            });
        });
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |chat, cx| chat.tick(cx)).is_err() {
                    return;
                }
            }
        });
        self.run = Some(Running { number, control, compacting, events, _tick: tick });
        self.refresh_from(from.saturating_sub(1));
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.run {
            run.control.stop();
            cx.notify();
        }
    }

    /// Hands `text` to the running agent, which reads it at its next step.
    pub fn steer(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(run) = &self.run
            && !run.compacting
        {
            run.control.steer(text.clone());
            self.steering.push(text);
            cx.notify();
        }
    }

    /// Shows a problem in the transcript, such as a run that could not start.
    pub fn show_failure(&mut self, message: String, cx: &mut Context<Self>) {
        let from = self.conversation.entries.len();
        self.conversation.entries.push(Entry::Notice { text: message, failed: true });
        self.refresh_from(from);
        cx.notify();
    }

    /// Opens or closes a work log or one of its rows.
    pub fn toggle(&mut self, key: SharedString, cx: &mut Context<Self>) {
        let key = key.to_string();
        if !self.open.remove(&key) {
            self.open.insert(key.clone());
        }
        if let Some(index) = self.entry_holding(&key) {
            self.refresh_entry(index);
        }
        cx.notify();
    }

    /// Applies events from run `number`; anything from another run is ignored.
    fn apply(&mut self, number: u64, batch: Vec<RunEvent>, cx: &mut Context<Self>) {
        if self.run.as_ref().is_none_or(|run| run.number != number) {
            return;
        }
        let now = now_millis();
        let from = self.conversation.entries.len().saturating_sub(1);
        let mut finished = None;
        for event in batch {
            match event {
                RunEvent::Agent(event) => {
                    match &event {
                        Event::Reply { usage: Some(usage) } => {
                            self.context_tokens = Some(usage.context_tokens() + usage.output);
                        }
                        // Everything waiting is delivered together.
                        Event::Steered(_) => self.steering.clear(),
                        _ => {}
                    }
                    self.conversation.apply(&event, now);
                }
                RunEvent::Finished { result, context_tokens } => {
                    if self.compacting() {
                        self.conversation.finish_compaction(&result);
                    } else {
                        self.conversation.finish(&result, now);
                    }
                    if context_tokens.is_some() {
                        self.context_tokens = context_tokens;
                    }
                    // Its event task is the caller: let it finish on its own rather than drop it
                    // mid-poll.
                    if let Some(run) = self.run.take() {
                        run.events.detach();
                    }
                    finished = Some(result);
                }
            }
        }
        self.refresh_from(from);
        if let Some(result) = finished {
            cx.emit(ChatEvent::Finished(result));
        }
        cx.notify();
    }

    /// Keeps "Working · 8s" counting.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.run.is_some() && !self.compacting() {
            self.refresh_from(self.conversation.entries.len().saturating_sub(1));
            cx.notify();
        }
    }

    /// The entry whose items show the work log or row with this key.
    fn entry_holding(&self, key: &str) -> Option<usize> {
        let mut start = 0;
        for (index, span) in self.spans.iter().enumerate() {
            let holds = self.items[start..start + span].iter().any(|item| match item {
                Item::Work(work) => work.key.as_ref() == key || work.rows.iter().any(|row| row.key.as_ref() == key),
                _ => false,
            });
            if holds {
                return Some(index);
            }
            start += span;
        }
        None
    }

    /// Rebuilds the items of entry `from` and every entry after it.
    fn refresh_from(&mut self, from: usize) {
        let now = now_millis();
        let from = from.min(self.spans.len());
        let start: usize = self.spans[..from].iter().sum();
        let old_end = self.items.len();
        self.items.truncate(start);
        self.spans.truncate(from);
        for index in from..self.conversation.entries.len() {
            let before = self.items.len();
            work::entry_items(&self.conversation, index, &self.cwd, &self.open, now, &mut self.cache, &mut self.items);
            self.spans.push(self.items.len() - before);
        }
        self.tell_list(start, old_end - start, self.items.len() - start);
    }

    /// Rebuilds one entry's items in place.
    fn refresh_entry(&mut self, index: usize) {
        let Some(&old_count) = self.spans.get(index) else {
            return;
        };
        let start: usize = self.spans[..index].iter().sum();
        let mut fresh = Vec::new();
        work::entry_items(&self.conversation, index, &self.cwd, &self.open, now_millis(), &mut self.cache, &mut fresh);
        let new_count = fresh.len();
        self.items.splice(start..start + old_count, fresh);
        self.spans[index] = new_count;
        self.tell_list(start, old_count, new_count);
    }

    /// Items `start..start + old_count` were replaced by `new_count` new ones.
    fn tell_list(&self, start: usize, old_count: usize, new_count: usize) {
        if old_count == new_count {
            // Same items, new content: keeps the reader's place if they scrolled up.
            self.list.remeasure_items(start..start + new_count);
        } else {
            self.list.splice(start..start + old_count, new_count);
        }
    }
}

/// How full the context was after the last reply in a saved chat. Unknown after a compaction,
/// until the next reply.
fn last_context(history: &[HistoryEntry]) -> Option<u64> {
    for entry in history.iter().rev() {
        match entry {
            HistoryEntry::Compacted => return None,
            HistoryEntry::Message { usage: Some(usage), .. } => return Some(usage.context_tokens() + usage.output),
            HistoryEntry::Message { .. } => {}
        }
    }
    None
}
