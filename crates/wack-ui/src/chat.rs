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
//! - The chat's panes (`panes`) live here too, so they outlast switching chats; every change to
//!   their layout goes through `change_layout`, which tells the app to save it.
//! - Messages written while the agent works either wait in `queue` (↩, sent after the run) or go
//!   to the model straight away as steering (⌘↩); `steering` holds those not yet delivered.
//!   Neither is ever silently dropped: when a run ends without sending them, the app puts them
//!   back in the composer.
//! - A question the agent asks (`asking`) is answered exactly once: answering, cancelling,
//!   stopping or the run ending all clear it. Its drafts live here, so switching chats keeps them.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, ListAlignment, ListState, SharedString, Subscription, Task, px};
use gpui_tokio::Tokio;
use wack_core::{
    Control, Event, HistoryEntry, Job, Outcome, RunEvent, RunSetup, Runner,
    ask::{Answer, Question},
};

use crate::{
    conversation::{Conversation, Entry},
    panes::{ChatPanes, Layout, PaneKind, PaneModel},
    question::{self, Draft},
    text_field::{TextField, TextFieldEvent},
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
    /// The panes moved, opened or closed: worth saving.
    LayoutChanged,
    /// The mode in force changed without the user: the agent entered Plan, or a plan was
    /// discarded.
    ModeChanged(wack_core::AgentMode),
}

pub struct ChatSession {
    pub id: String,
    cwd: PathBuf,
    session_path: PathBuf,
    conversation: Conversation,
    /// Keys of the work logs and rows the user opened.
    open: HashSet<String>,
    cache: ReplyCache,
    items: Vec<Item>,
    spans: Vec<usize>,
    pub list: ListState,
    pub composer: Entity<TextField>,
    pub panes: ChatPanes,
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
    /// The question the agent is waiting on.
    pub asking: Option<Asking>,
    /// Where focus should go at the next frame: the question card as it arrives, the composer
    /// after it closes.
    pub focus_request: Option<FocusHandle>,
    /// Tasks already done when the current run started: they fold behind "+N done".
    pub todo_faded: HashSet<u64>,
    /// The folded tasks are shown anyway.
    pub todo_show_done: bool,
    /// The todo list and the waiting messages are open above their status pills; folded, only
    /// the pills show.
    pub todos_open: bool,
    pub waiting_open: bool,
}

/// A question the agent is waiting on, and the answers so far.
pub struct Asking {
    id: String,
    pub questions: Vec<Question>,
    /// Offer "Write the plan now" (Ultra Plan).
    pub wrap_up: bool,
    pub tab: usize,
    /// The chosen options, per question.
    selected: Vec<Vec<String>>,
    /// The "Other" fields, per question.
    pub others: Vec<Entity<TextField>>,
    /// The options take focus, so 1–4 reach them.
    pub focus: FocusHandle,
    /// Picks so far: each nods the duck.
    pub nods: usize,
    _subscriptions: Vec<Subscription>,
}

impl Asking {
    pub fn drafts(&self, cx: &gpui::App) -> Vec<Draft> {
        self.selected
            .iter()
            .zip(&self.others)
            .map(|(selected, other)| Draft { selected: selected.clone(), custom: other.read(cx).text().to_string() })
            .collect()
    }
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
    pub fn load(
        id: &str,
        cwd: PathBuf,
        session_path: &std::path::Path,
        layout: Layout,
        runner: Runner,
        commands: (Option<String>, Option<String>),
        cx: &mut Context<Self>,
    ) -> Self {
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
        let mut panes = ChatPanes::new(layout, cwd.clone());
        panes.set_runner(runner);
        panes.set_commands(commands.0, commands.1, cx);
        let composer = cx.new(|cx| TextField::new(Type::Body, FOLLOW_UP_PLACEHOLDER, cx).multiline(12));
        let mut chat = Self {
            id: id.to_string(),
            cwd,
            session_path: session_path.to_path_buf(),
            conversation,
            open: HashSet::new(),
            cache: ReplyCache::default(),
            items: Vec::new(),
            spans: Vec::new(),
            list: ListState::new(0, ListAlignment::Bottom, px(800.)),
            composer,
            panes,
            queue: Vec::new(),
            steering: Vec::new(),
            run: None,
            runs_started: 0,
            context_tokens,
            unread: false,
            asking: None,
            focus_request: None,
            todo_faded: HashSet::new(),
            todo_show_done: false,
            todos_open: false,
            waiting_open: false,
        };
        chat.refresh_from(0);
        chat.panes.sync(cx, |_, cx| cx.notify());
        chat
    }

    /// Changes the panes' layout. When anything changed, makes models for panes now on screen,
    /// refreshes a Changes pane that just came back into view, and tells the app to save.
    pub fn change_layout<R>(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Layout) -> R) -> R {
        let before = self.panes.layout.clone();
        let result = change(&mut self.panes.layout);
        if self.panes.layout != before {
            let was_visible: Vec<_> = before.visible().iter().map(|pane| pane.id).collect();
            let made = self.panes.sync(cx, |_, cx| cx.notify());
            for pane in self.panes.layout.visible() {
                if !was_visible.contains(&pane.id)
                    && !made.contains(&pane.id)
                    && pane.kind == PaneKind::Changes
                    && let Some(PaneModel::Changes(changes)) = self.panes.model(pane.id)
                {
                    changes.update(cx, |changes, cx| changes.refresh(cx));
                }
            }
            cx.emit(ChatEvent::LayoutChanged);
            cx.notify();
        }
        result
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
        let compacting = matches!(job, Job::Compact(_));
        match &job {
            Job::Prompt(text) => self.conversation.send(text, now_millis()),
            Job::Command { command, .. } => self.conversation.send(&command.line(), now_millis()),
            Job::Compact(_) => self.conversation.begin_compaction(),
        }

        // What was done before this run folds away.
        self.todo_faded = crate::todo_panel::completed_ids(&self.conversation.todos);
        self.todo_show_done = false;
        self.conversation.plan_view = Default::default();

        self.runs_started += 1;
        let number = self.runs_started;
        // Someone is at the window to answer questions.
        let control = Control::with_user();
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
            self.close_question(cx);
            cx.notify();
        }
    }

    /// The agent asked: shows the card, and gives it focus.
    fn ask(&mut self, id: String, questions: Vec<Question>, wrap_up: bool, cx: &mut Context<Self>) {
        let mut others = Vec::new();
        let mut subscriptions = Vec::new();
        for index in 0..questions.len() {
            let field = cx.new(|cx| TextField::new(Type::Ui, "Other — type a custom answer", cx).multiline(4));
            subscriptions.push(cx.subscribe(&field, move |chat, field, event: &TextFieldEvent, cx| match event {
                // Typing an answer of one's own replaces the choices.
                TextFieldEvent::Changed => {
                    if let Some(asking) = &mut chat.asking
                        && !field.read(cx).text().trim().is_empty()
                        && let Some(selected) = asking.selected.get_mut(index)
                        && !selected.is_empty()
                    {
                        selected.clear();
                    }
                    cx.notify();
                }
                TextFieldEvent::Enter => chat.next_question(cx),
                TextFieldEvent::SendNow => chat.submit_answers(cx),
            }));
            others.push(field);
        }
        let focus = cx.focus_handle();
        self.focus_request = Some(focus.clone());
        self.asking = Some(Asking {
            id,
            selected: vec![Vec::new(); questions.len()],
            questions,
            wrap_up,
            tab: 0,
            others,
            focus,
            nods: 0,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Picks option `index` of the question on show.
    pub fn pick_option(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(asking) = &mut self.asking else {
            return;
        };
        let tab = asking.tab;
        let Some(label) = asking.questions[tab].options.get(index).map(|option| option.label.clone()) else {
            return;
        };
        let mut draft = Draft { selected: asking.selected[tab].clone(), custom: asking.others[tab].read(cx).text().to_string() };
        question::pick(&asking.questions[tab], &mut draft, &label);
        asking.selected[tab] = draft.selected;
        asking.nods += 1;
        let other = asking.others[tab].clone();
        if !other.read(cx).text().is_empty() {
            other.update(cx, |other, cx| other.set_text("", cx));
        }
        cx.notify();
    }

    pub fn show_question(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(asking) = &mut self.asking
            && index < asking.questions.len()
        {
            asking.tab = index;
            self.focus_request = Some(asking.focus.clone());
            cx.notify();
        }
    }

    /// ↩ on the card: the next question once this one is answered, or Submit on the last.
    pub fn next_question(&mut self, cx: &mut Context<Self>) {
        let Some(asking) = &self.asking else {
            return;
        };
        let drafts = asking.drafts(cx);
        let tab = asking.tab;
        if tab + 1 == asking.questions.len() {
            self.submit_answers(cx);
        } else if drafts[tab].answered() {
            self.show_question(tab + 1, cx);
        }
    }

    pub fn previous_question(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.asking.as_ref().map(|asking| asking.tab)
            && tab > 0
        {
            self.show_question(tab - 1, cx);
        }
    }

    /// Sends the answers, once every question has one.
    pub fn submit_answers(&mut self, cx: &mut Context<Self>) {
        let Some(asking) = &self.asking else {
            return;
        };
        if let Some(answers) = question::answers(&asking.questions, &asking.drafts(cx)) {
            self.answer(Answer::Answered(answers), cx);
        }
    }

    pub fn cancel_question(&mut self, cx: &mut Context<Self>) {
        self.answer(Answer::Cancelled, cx);
    }

    /// Ultra Plan's "Write the plan now".
    pub fn wrap_up(&mut self, cx: &mut Context<Self>) {
        if self.asking.as_ref().is_some_and(|asking| asking.wrap_up) {
            self.answer(Answer::WrapUp, cx);
        }
    }

    fn answer(&mut self, answer: Answer, cx: &mut Context<Self>) {
        let Some(asking) = &self.asking else {
            return;
        };
        if let Some(run) = &self.run {
            run.control.answer(&asking.id, answer);
        }
        self.close_question(cx);
    }

    /// Takes the card away and hands focus back to the composer.
    fn close_question(&mut self, cx: &mut Context<Self>) {
        if self.asking.take().is_some() {
            self.focus_request = Some(self.composer.read(cx).focus_handle(cx));
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

    /// Tells the user something about the chat that is not a failure of its run, such as an
    /// automatic title that could not be had. It joins the latest turn's notices, so a turn still
    /// streaming stays the last entry; before any turn it stands on its own.
    pub fn note(&mut self, message: String, cx: &mut Context<Self>) {
        let last_turn = self.conversation.entries.iter().rposition(|entry| matches!(entry, Entry::Turn(_)));
        match last_turn {
            Some(index) => {
                if let Some(Entry::Turn(turn)) = self.conversation.entries.get_mut(index) {
                    turn.notices.push(message);
                }
                self.refresh_entry(index);
            }
            None => {
                let from = self.conversation.entries.len();
                self.conversation.entries.push(Entry::Notice { text: message, failed: false });
                self.refresh_from(from);
            }
        }
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
        let mut from = self.conversation.entries.len().saturating_sub(1);
        let (awaiting_before, mode_before) = (self.conversation.awaiting, self.conversation.plan.mode);
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
                        // A tool may have changed files.
                        Event::ToolResult { .. } => self.panes.refresh_changes(cx),
                        Event::Question { id, questions, wrap_up } => {
                            self.ask(id.clone(), questions.clone(), *wrap_up, cx);
                        }
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
                    self.close_question(cx);
                    self.panes.refresh_changes(cx);
                    finished = Some(result);
                }
            }
        }
        // A plan that stopped awaiting a decision loses its actions, wherever it is.
        if self.conversation.awaiting != awaiting_before
            && let Some(before) = awaiting_before
        {
            from = from.min(before);
        }
        self.refresh_from(from);
        if self.conversation.plan.mode != mode_before {
            cx.emit(ChatEvent::ModeChanged(self.conversation.plan.mode));
        }
        if let Some(result) = finished {
            cx.emit(ChatEvent::Finished(result));
        }
        cx.notify();
    }

    /// The text of the plan awaiting review.
    pub fn awaiting_plan(&self) -> Option<String> {
        self.conversation.awaiting_plan().map(|proposal| proposal.text.clone())
    }

    /// Puts focus back in the composer at the next frame: after the plan card is acted on.
    pub fn focus_composer(&mut self, cx: &mut Context<Self>) {
        self.focus_request = Some(self.composer.read(cx).focus_handle(cx));
    }

    /// Rebuilds the card of the plan awaiting review.
    fn refresh_plan_card(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.conversation.awaiting {
            self.refresh_entry(index);
        }
        cx.notify();
    }

    /// Discard was pressed: the card asks first.
    pub fn ask_discard_plan(&mut self, cx: &mut Context<Self>) {
        if self.conversation.awaiting.is_some() && !self.running() {
            self.conversation.plan_view.confirming_discard = true;
            self.refresh_plan_card(cx);
        }
    }

    pub fn keep_plan(&mut self, cx: &mut Context<Self>) {
        self.conversation.plan_view.confirming_discard = false;
        self.refresh_plan_card(cx);
    }

    /// A word under the plan card's buttons, after Copy or Save.
    pub fn plan_notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.conversation.plan_view.notice = Some(text);
        self.refresh_plan_card(cx);
    }

    /// Abandons the plan awaiting review and returns the chat to Build. The conversation is kept.
    pub fn discard_plan(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.running() {
            return Err("Wait for the agent to finish first.".into());
        }
        wack_core::discard_plan(&self.session_path)?;
        let before = self.conversation.awaiting;
        self.conversation.discard_plan();
        if let Some(index) = before {
            self.refresh_entry(index);
        }
        cx.emit(ChatEvent::ModeChanged(wack_core::AgentMode::Build));
        self.focus_composer(cx);
        cx.notify();
        Ok(())
    }

    /// Shows the folded tasks.
    pub fn show_done_todos(&mut self, cx: &mut Context<Self>) {
        self.todo_show_done = true;
        cx.notify();
    }

    pub fn toggle_todos(&mut self, cx: &mut Context<Self>) {
        self.todos_open = !self.todos_open;
        cx.notify();
    }

    pub fn toggle_waiting(&mut self, cx: &mut Context<Self>) {
        self.waiting_open = !self.waiting_open;
        cx.notify();
    }

    /// While a run is going: how long it has run ("Working · 8s", or the retry it is waiting on)
    /// and what it is doing now (`work::activity`), from its live work log.
    pub fn activity(&self) -> Option<(SharedString, Option<SharedString>)> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Work(work) if work.live => Some((work.summary.clone(), crate::work::activity(&work.rows).map(Into::into))),
            _ => None,
        })
    }

    pub fn todos(&self) -> &wack_core::todo::TodoList {
        &self.conversation.todos
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
            HistoryEntry::Message { .. } | HistoryEntry::Note { .. } | HistoryEntry::State { .. } => {}
        }
    }
    None
}
