//! One chat's live state: its conversation, the transcript items drawn from it, its composer, and
//! the run in progress. Owned by [`crate::app::AppState`] for as long as the window is open, so
//! switching chats never stops a run or loses a draft.
//!
//! Invariants:
//! - `items` is always what `work::items` would build from `conversation` (for a Chat chat,
//!   `chat_transcript::entry_items`, entry by entry), and `spans[i]` is how
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
//! - The chat's shell tasks (`tasks`) live here, not in a run, so a server the agent started
//!   keeps running between turns; dropping the chat stops them. When one has news for the model
//!   the chat tells the app ([`ChatEvent::TasksNews`]), which decides whether to wake it.
//! - The chat's sub-agents (`agents`) live here too, so one working in the background outlives
//!   the run that started it. Stop (Esc) stops every one of them: an agent left running only
//!   spends tokens. A report waiting for the model is [`ChatEvent::AgentsNews`].
//! - A question the agent asks (`asking`) is answered exactly once: answering, cancelling,
//!   stopping or the run ending all clear it. Its drafts live here, so switching chats keeps them.
//! - The message being edited (`editing`) and the card asking what a rewind does (`rewind_card`)
//!   live here too. While a rewind is under way (`rewinding`) nothing may run: the engine is
//!   writing the chat's file. After it the conversation is read again from the file (`reload`).

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
    tasks::{Signal, Tasks},
    agents::{Agents, Signal as AgentSignal},
    ask::{Answer, Question},
};

use crate::{
    conversation::{Conversation, Entry, Step},
    panes::{ChatPanes, Layout, PaneKind, PaneModel},
    question::{self, Draft},
    suggestions::{self, View as SuggestionView},
    text_field::{TextField, TextFieldEvent},
    transcript::Item,
    typography::Type,
    work::{self, ReplyCache},
};

pub const FOLLOW_UP_PLACEHOLDER: &str = "Reply, or ask for the next change";

/// How many files differ from the last commit, and the lines added and removed across them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChangeCount {
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

impl ChangeCount {
    pub fn of(files: &[wack_core::git::ChangedFile]) -> Self {
        let (added, removed) = files.iter().filter_map(|file| file.lines).fold((0, 0), |sum, (a, r)| (sum.0 + a, sum.1 + r));
        Self { files: files.len(), added, removed }
    }
}

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
    /// A background task has news for the model.
    TasksNews,
    /// A sub-agent in the background reported.
    AgentsNews,
    /// The browser tabs' addresses or titles changed: worth saving.
    BrowserTabsChanged(Vec<wack_browser::SavedTab>),
    /// The agent started a canvas: the Canvas pane opens for it.
    AgentStartedCanvas,
    /// The agent or a sub-agent opened a tab (Settings › Browser may show the pane).
    AgentOpenedTab,
    /// The agent started a sub-agent: a Team chat's Agents pane may open on its overview.
    AgentStarted,
}

pub struct ChatSession {
    pub id: String,
    cwd: PathBuf,
    /// A Chat area chat: its transcript reads as a messenger's (`chat_transcript`), and `cwd` is
    /// its scratchpad.
    chat_area: bool,
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
    /// Pictures attached to the message being written.
    pub attachments: Vec<crate::attachments::Attachment>,
    /// Text files attached to the message (picked, dropped, pasted, terminal).
    pub files: Vec<crate::attachments::FileAttachment>,
    /// Pictures of queued messages, by the message's text: sent with it, or back in the tray
    /// when it comes back to the composer. A message with pictures always queues, never steers.
    pub queued_pictures: Vec<(String, Vec<crate::attachments::Attachment>)>,
    /// Handed to the run, not yet delivered to the model.
    pub steering: Vec<String>,
    run: Option<Running>,
    /// Numbers runs, so a finished run's task can never act on the run after it.
    runs_started: u64,
    /// How full the model's context is, in tokens, when known.
    pub context_tokens: Option<u64>,
    /// The same, part by part, as the last run left it (`wack_core::context`); empty until a
    /// run in this window has ended.
    pub context_parts: Vec<wack_core::context::Part>,
    /// The project's changed files against its last commit, and the lines added and removed:
    /// what the composer's tab counts. `None` until Git answers, or outside a repository.
    pub changes: Option<ChangeCount>,
    counting: Option<Task<()>>,
    count_again: bool,
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
    /// The todo list is open above the composer; folded, only its count shows.
    pub todos_open: bool,
    /// Commands the agent started, which outlive its runs.
    pub tasks: Tasks,
    /// The list of them is open above its status pill.
    pub tasks_open: bool,
    /// The MCP servers the run is waiting for, for the "Starting MCP servers…" pill.
    pub mcp_starting: Vec<String>,
    /// Runs started for task news since the user last sent a message, and whether waking is
    /// paused because the user stopped the last run.
    pub wakes: u32,
    pub wake_paused: bool,
    /// Sub-agents the agent started, which outlive its runs.
    pub agents: Agents,
    /// How many the registry listed when it last signalled, so a new one is noticed.
    agents_seen: usize,
    /// The chat's browser tabs, which outlive its runs and the Browser pane.
    pub tabs: wack_browser::ChatTabs,
    /// The chat's canvases, which outlive its runs and the Canvas pane.
    pub canvases: wack_canvas::Canvases,
    /// What canvas cards show of each artboard (its name, its newest small picture), found once
    /// and kept until the canvases next signal, so rebuilding a live turn never reads the disk.
    canvas_faces: std::collections::HashMap<(String, String), crate::canvas_card::Face>,
    /// Comments drafted on the canvases and not sent yet, by canvas: the composer's chip. Found
    /// when the canvases signal.
    pub comment_drafts: Vec<(String, usize)>,
    _canvases_watch: Task<()>,
    _tasks_watch: Task<()>,
    _agents_watch: Task<()>,
    _browser_watch: Task<()>,
    /// The chat's computer use, which outlives its runs: a Code chat's only
    /// (`AppState::ensure_session`).
    pub computer: Option<wack_computer::computer::ChatComputer>,
    /// The user's message being edited in place.
    pub editing: Option<Editing>,
    /// The card under a message asking what a rewind, an edit or a retry does.
    pub rewind_card: Option<crate::rewind::Card>,
    /// What an edit or retry will send once the chat is rewound, resolved when it was asked for.
    pub resend: Option<Job>,
    /// A rewind is under way: the engine is writing the chat's file, so nothing may run.
    pub rewinding: bool,
    /// The words a rewind put back in the composer, which undoing it takes away again.
    pub rewound_words: Option<String>,
    /// The access card's app icon, decoded once per card: (card id, picture).
    computer_icon: Option<(u64, Option<std::sync::Arc<gpui::RenderImage>>)>,
    /// Why the card's last Always allow or Never allow couldn't be saved.
    pub computer_error: Option<(u64, SharedString)>,
    _computer_watch: Option<Task<()>>,
}

/// The user's message being edited in place: its number (from 0) and the field holding its
/// words.
pub struct Editing {
    pub number: usize,
    pub field: Entity<TextField>,
    pub _subscriptions: Vec<Subscription>,
}

/// A copy of one of the user's messages, for sending it again.
#[derive(Clone, Debug)]
pub struct SentPrompt {
    /// The words as the user wrote them: a slash command as typed.
    pub words: String,
    pub pictures: Vec<crate::pictures::Picture>,
    pub files: Vec<crate::attachments::FileAttachment>,
    pub refs: Vec<wack_core::chat_refs::ChatRef>,
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
    // Each is a separate thing the app owns and hands over once; a struct would only rename them.
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        id: &str,
        cwd: PathBuf,
        session_path: &std::path::Path,
        layout: Layout,
        runner: Runner,
        commands: (Option<String>, Option<String>),
        tasks_folder: PathBuf,
        tabs: wack_browser::ChatTabs,
        canvases: wack_canvas::Canvases,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut context_tokens = None;
        let mut saved_agents = Vec::new();
        let conversation = if session_path.exists() {
            match wack_core::history(session_path) {
                Ok(history) => {
                    context_tokens = last_context(&history);
                    saved_agents = last_agents(&history);
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
        let composer = cx.new(|cx| TextField::new(Type::Body, FOLLOW_UP_PLACEHOLDER, cx).multiline(crate::app::COMPOSER_ROWS).takes_pictures());
        let tasks = Tasks::new(tasks_folder);
        let tasks_watch = watch_tasks(&tasks, cx);
        panes.set_tasks(tasks.clone());
        // The agents the session last listed, so their rows and pane read right before any run;
        // one that was working when the app quit reads as interrupted.
        let agents = Agents::new();
        agents.restore(saved_agents);
        let agents_watch = watch_agents(&agents, cx);
        panes.set_agents(agents.clone());
        let browser_watch = watch_browser(&tabs, cx);
        panes.set_browser(tabs.clone());
        let canvases_watch = watch_canvases(&canvases, cx);
        panes.set_canvases(canvases.clone());
        let mut chat = Self {
            id: id.to_string(),
            cwd,
            chat_area: false,
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
            attachments: Vec::new(),
            files: Vec::new(),
            queued_pictures: Vec::new(),
            steering: Vec::new(),
            run: None,
            runs_started: 0,
            context_tokens,
            context_parts: Vec::new(),
            changes: None,
            counting: None,
            count_again: false,
            unread: false,
            asking: None,
            focus_request: None,
            todo_faded: HashSet::new(),
            todo_show_done: false,
            todos_open: false,
            tasks,
            tasks_open: false,
            mcp_starting: Vec::new(),
            wakes: 0,
            wake_paused: false,
            agents_seen: agents.list().len(),
            agents,
            tabs,
            comment_drafts: canvases.drafts(),
            canvases,
            canvas_faces: Default::default(),
            _canvases_watch: canvases_watch,
            _tasks_watch: tasks_watch,
            _agents_watch: agents_watch,
            _browser_watch: browser_watch,
            computer: None,
            computer_icon: None,
            computer_error: None,
            _computer_watch: None,
            editing: None,
            rewind_card: None,
            resend: None,
            rewinding: false,
            rewound_words: None,
        };
        chat.conversation.agents = chat.agent_rows();
        chat.refresh_from(0);
        chat.panes.sync(cx, |_, cx| cx.notify());
        chat.count_changes(cx);
        chat
    }

    /// Asks Git for the changed files again, off the UI thread. A request made while one runs
    /// runs once more after it, so the count never stays behind the files.
    pub fn count_changes(&mut self, cx: &mut Context<Self>) {
        if self.chat_area {
            return;
        }
        if self.counting.is_some() {
            self.count_again = true;
            return;
        }
        let folder = self.cwd.clone();
        let lookup = cx.background_spawn(async move { wack_core::git::changes(&folder) });
        self.counting = Some(cx.spawn(async move |chat, cx| {
            let result = lookup.await;
            let _ = chat.update(cx, |chat, cx| {
                chat.counting = None;
                let count = match result {
                    Ok(wack_core::git::Changes::Repository { files, .. }) => Some(ChangeCount::of(&files)),
                    _ => None,
                };
                if chat.changes != count {
                    chat.changes = count;
                    cx.notify();
                }
                if std::mem::take(&mut chat.count_again) {
                    chat.count_changes(cx);
                }
            });
        }));
    }

    /// The chat's agents by the call that started each one's latest round: what their rows read.
    /// A round the user started from the pane has no call, so its agent's rows read as saved.
    fn agent_rows(&self) -> std::collections::HashMap<String, wack_core::agents::AgentInfo> {
        self.agents
            .list()
            .into_iter()
            .filter_map(|info| {
                let call = if info.round > 1 { info.round_call_id.clone() } else { info.call_id.clone() };
                (!call.is_empty()).then_some((call, info))
            })
            .collect()
    }

    /// Brings the agents' rows up to date: those that changed, and those still working, whose
    /// times count on.
    fn sync_agents(&mut self) {
        let rows = self.agent_rows();
        let mut stale: Vec<String> = rows
            .iter()
            .filter(|(call, info)| info.status.active() || self.conversation.agents.get(*call) != Some(info))
            .map(|(call, _)| call.clone())
            .collect();
        stale.extend(self.conversation.agents.keys().filter(|call| !rows.contains_key(*call)).cloned());
        self.conversation.agents = rows;
        for call in stale {
            if let Some(index) = self.entry_holding(&call) {
                self.refresh_entry(index);
            }
        }
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

    /// Starts a run: the user's message, or a compaction. Never while a rewind writes the
    /// chat's file (the app refuses first, saying why).
    pub fn start(&mut self, setup: RunSetup, job: Job, cx: &mut Context<Self>) {
        if self.run.is_some() || self.rewinding {
            return;
        }
        // A run writes to the chat, so the rewind before it can no longer be undone, and the
        // message being edited or asked about stays as it was.
        self.conversation.forget_rewind();
        self.rewound_words = None;
        self.editing = None;
        self.rewind_card = None;
        self.resend = None;
        let from = self.conversation.entries.len();
        let compacting = matches!(job, Job::Compact(_));
        let started_before = self.conversation.started.len();
        match &job {
            Job::Prompt(text) => self.conversation.send(text, now_millis()),
            Job::Command { text, command } => self.conversation.send_command(text, command.clone(), now_millis()),
            Job::Attached { text, command, images } => {
                let pictures = images.iter().map(crate::pictures::Picture::of_image).collect();
                self.conversation.send_pictures(text, command.clone(), pictures, now_millis());
            }
            Job::Compact(_) => self.conversation.begin_compaction(),
            Job::Wake => self.conversation.wake(now_millis()),
        }
        if self.conversation.started.len() != started_before {
            // The message just sent started a suggested task: its card on the earlier turn
            // reads started now (and is out of the queue, which `begin` already dropped).
            self.refresh_suggestions();
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
        let mut setup = setup;
        setup.tasks = Some(self.tasks.clone());
        setup.browser = Some(self.tabs.clone());
        setup.canvases = Some(self.canvases.clone());
        // The Chat area never uses the computer, whatever it is handed.
        if !self.chat_area {
            setup.computer = self.computer.clone();
        }
        if let Some(agents) = setup.agents.as_mut() {
            agents.agents = self.agents.clone();
        }
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

    /// Gives a Code chat its computer use, and redraws when its card comes or goes.
    pub fn set_computer(&mut self, computer: wack_computer::computer::ChatComputer, cx: &mut Context<Self>) {
        let (sender, mut receiver) = mpsc::unbounded::<wack_computer::computer::Signal>();
        computer.set_observer(move |signal| {
            let _ = sender.unbounded_send(signal);
        });
        self._computer_watch = Some(cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                while receiver.try_recv().is_ok() {}
                if this.update(cx, |chat, cx| chat.computer_changed(cx)).is_err() {
                    return;
                }
            }
        }));
        self.computer = Some(computer);
        self.computer_changed(cx);
    }

    fn computer_changed(&mut self, cx: &mut Context<Self>) {
        let card = self.computer.as_ref().and_then(|computer| computer.card());
        match &card {
            Some(card) if self.computer_icon.as_ref().is_none_or(|(id, _)| *id != card.id) => {
                let icon = match &card.ask {
                    wack_computer::computer::Ask::App { icon, .. } => icon.clone(),
                    wack_computer::computer::Ask::Clipboard { .. } | wack_computer::computer::Ask::Screen { .. } => None,
                };
                let picture = icon
                    .as_deref()
                    .and_then(|bytes| image::load_from_memory(bytes).ok())
                    .map(|picture| std::sync::Arc::new(crate::attachments::render_image(&picture, 64)));
                self.computer_icon = Some((card.id, picture));
            }
            Some(_) => {}
            None => {
                self.computer_icon = None;
                self.computer_error = None;
            }
        }
        cx.notify();
    }

    /// The computer-use card as the composer draws it.
    pub fn computer_card(&self) -> Option<crate::computer_card::CardData> {
        let card = self.computer.as_ref()?.card()?;
        let icon = self.computer_icon.as_ref().filter(|(id, _)| *id == card.id).and_then(|(_, icon)| icon.clone());
        let error = self.computer_error.as_ref().filter(|(id, _)| *id == card.id).map(|(_, error)| error.clone());
        Some(match &card.ask {
            wack_computer::computer::Ask::App { app, tier, launching, .. } => crate::computer_card::CardData {
                id: card.id,
                app: app.name.clone().into(),
                path: app.path.as_ref().map(|path| path.display().to_string().into()),
                look_only: *tier == wack_computer::policy::Tier::LookOnly,
                launching: *launching,
                icon,
                error,
                clipboard: None,
                screen: None,
            },
            wack_computer::computer::Ask::Clipboard { write } => crate::computer_card::CardData {
                id: card.id,
                app: "the clipboard".into(),
                path: None,
                look_only: false,
                launching: false,
                icon: None,
                error,
                clipboard: Some(*write),
                screen: None,
            },
            wack_computer::computer::Ask::Screen { reason, apps } => crate::computer_card::CardData {
                id: card.id,
                app: "your screen".into(),
                path: None,
                look_only: false,
                launching: false,
                icon: None,
                error,
                clipboard: None,
                screen: Some(crate::computer_card::ScreenAsk { reason: reason.clone().into(), apps: crate::computer_card::list(apps).into() }),
            },
        })
    }

    /// Stops the run and every sub-agent, in the foreground or the background.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.agents.stop_all();
        if let Some(run) = &self.run {
            run.control.stop();
            self.close_question(cx);
        }
        cx.notify();
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
                TextFieldEvent::Moved | TextFieldEvent::Menu(_) | TextFieldEvent::PastedPictures => {}
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

    /// The pictures queued with `text`, taken out of the queue's keeping.
    pub fn take_queued_pictures(&mut self, text: &str) -> Vec<crate::attachments::Attachment> {
        match self.queued_pictures.iter().position(|(queued, _)| queued == text) {
            Some(at) => self.queued_pictures.remove(at).1,
            None => Vec::new(),
        }
    }

    /// The file the chat is saved in.
    pub fn session_path(&self) -> &std::path::Path {
        &self.session_path
    }

    /// What the user's messages offer as the transcript draws them.
    pub fn prompt_view(&self) -> crate::rewind::Prompts {
        crate::rewind::Prompts {
            chat: true,
            idle: self.run.is_none() && !self.rewinding,
            count: self.conversation.prompt_count(),
            editing: self.editing.as_ref().map(|editing| (editing.number, editing.field.clone())),
            card: self.rewind_card.clone(),
            hovered: None,
        }
    }

    /// A copy of the user's `number`-th message (from 0), to send again.
    pub fn sent_prompt(&self, number: usize) -> Option<SentPrompt> {
        let Some(Entry::Prompt { text, command, pictures, files, refs, .. }) = self.conversation.prompt(number) else {
            return None;
        };
        Some(SentPrompt {
            words: command.as_ref().map_or_else(|| text.clone(), |command| command.line()),
            pictures: pictures.clone(),
            files: files.clone(),
            refs: refs.clone(),
        })
    }

    /// Redraws the user's `number`-th message, whose height changed (opened for editing, or a
    /// card under it).
    pub fn remeasure_prompt(&self, number: usize) {
        if let Some(index) = self.items.iter().position(|item| matches!(item, Item::Prompt { number: shown, .. } if *shown == number)) {
            self.list.remeasure_items(index..index + 1);
        }
    }

    /// Reads the conversation again from the chat's file, after a rewind changed it. Every item
    /// is rebuilt; what the user opened stays open where it still exists.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let history = match wack_core::history(&self.session_path) {
            Ok(history) => history,
            Err(message) => {
                self.show_failure(message, cx);
                return;
            }
        };
        self.context_tokens = last_context(&history);
        self.context_parts.clear();
        let mut conversation = Conversation::from_history(&history);
        conversation.suggestion_view = std::mem::take(&mut self.conversation.suggestion_view);
        self.conversation = conversation;
        self.conversation.agents = self.agent_rows();
        self.cache = ReplyCache::default();
        self.refresh_from(0);
        self.count_changes(cx);
        cx.notify();
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
    /// The project folder the chat works in.
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// The suggested task a call of this chat filed, by its tool-call id.
    pub fn suggestion(&self, id: &str) -> Option<suggestions::Card> {
        self.conversation.suggestion(id)
    }

    /// Brings the cards' outside state up to date — dismissals and opened chats from the
    /// store — filling in this chat's queue.
    pub fn sync_suggestion_view(&mut self, mut view: SuggestionView, cx: &mut Context<Self>) {
        view.queued = self.queued_suggestions();
        if view != self.conversation.suggestion_view {
            self.conversation.suggestion_view = view;
            self.refresh_suggestions();
            cx.notify();
        }
    }

    /// The send queue moved: cards it holds a suggestion for read as queued.
    pub fn sync_suggestion_queue(&mut self, cx: &mut Context<Self>) {
        let queued = self.queued_suggestions();
        if queued != self.conversation.suggestion_view.queued {
            self.conversation.suggestion_view.queued = queued;
            self.refresh_suggestions();
            cx.notify();
        }
    }

    /// Suggested tasks waiting in the send queue, by id.
    fn queued_suggestions(&self) -> HashSet<String> {
        self.queue
            .iter()
            .filter_map(|text| suggestions::split(text).1.map(|started| started.id))
            .collect()
    }

    /// Rebuilds the entries whose turns hold suggestion cards.
    fn refresh_suggestions(&mut self) {
        let entries: Vec<usize> = self
            .conversation
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                matches!(entry, Entry::Turn(turn) if turn.steps.iter().any(|step| matches!(step, Step::Tool(tool) if tool.name == wack_core::suggest::SUGGEST_TASK))).then_some(index)
            })
            .collect();
        for index in entries {
            self.refresh_entry(index);
        }
    }

    /// The text of the latest finished reply, for `/copy`.
    pub fn last_reply(&self) -> Option<String> {
        self.conversation.entries.iter().rev().find_map(|entry| match entry {
            Entry::Turn(turn) if !turn.reply.trim().is_empty() => Some(turn.reply.clone()),
            _ => None,
        })
    }

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
                        Event::ToolResult { .. } => {
                            self.panes.refresh_changes(cx);
                            self.count_changes(cx);
                        }
                        Event::Question { id, questions, wrap_up } => {
                            self.ask(id.clone(), questions.clone(), *wrap_up, cx);
                        }
                        _ => {}
                    }
                    self.conversation.apply(&event, now);
                }
                RunEvent::McpStarting(servers) => self.mcp_starting = servers,
                RunEvent::Context(parts) => self.context_parts = parts,
                RunEvent::Finished { result, context_tokens } => {
                    self.mcp_starting.clear();
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
                    self.count_changes(cx);
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
        // What the model is drawing on the canvas, for the board (cleared as the call ends).
        let writing = if self.running() { self.conversation.canvas_writing() } else { None };
        self.panes.canvas_writing(writing, cx);
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

    /// While a run is going: how long it has run ("Working for 8s", or the retry it is waiting
    /// on), or the MCP servers it is waiting for. Never what it is doing: the transcript shows
    /// that, and the owner found it noise beside the time.
    pub fn activity(&self) -> Option<SharedString> {
        // Waiting for an MCP server that has never listed its tools (or for one a tool needs).
        if !self.mcp_starting.is_empty() {
            return Some(format!("Starting {}…", self.mcp_starting.join(", ")).into());
        }
        let Entry::Turn(turn) = self.conversation.entries.last()? else {
            return None;
        };
        turn.live.then(|| crate::work::summary(turn, now_millis()).into())
    }

    /// Moves the command or the sub-agents the agent is waiting on to the background (⌘B).
    /// False when it waits on none.
    pub fn background_command(&mut self, cx: &mut Context<Self>) -> bool {
        let moved = self.tasks.background_foreground();
        let moved = self.agents.background_foreground() || moved;
        cx.notify();
        moved
    }

    /// Stops a sub-agent for the user; the agent is told, quietly.
    pub fn stop_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(child) = self.agents.get(id) {
            child.stop();
        }
        cx.notify();
    }

    /// Stops a background task for the user; the agent is told.
    pub fn stop_task(&mut self, id: String, cx: &mut Context<Self>) {
        let tasks = self.tasks.clone();
        Tokio::spawn(cx, async move { tasks.stop(&id, wack_core::tasks::StoppedBy::User).await }).detach();
        cx.notify();
    }

    pub fn toggle_tasks(&mut self, cx: &mut Context<Self>) {
        self.tasks_open = !self.tasks_open;
        cx.notify();
    }

    pub fn todos(&self) -> &wack_core::todo::TodoList {
        &self.conversation.todos
    }

    /// The share of the chat's prompt tokens served from the provider's cache (`Conversation::cache_rate`).
    pub fn cache_rate(&self) -> Option<f32> {
        self.conversation.cache_rate()
    }

    /// Every token the chat's own model used since the user's latest message: its turns,
    /// wakes included, not its agents'.
    pub fn lead_tokens(&self) -> u64 {
        let entries = &self.conversation.entries;
        let start = entries.iter().rposition(|entry| matches!(entry, Entry::Prompt { steered: false, .. })).unwrap_or(0);
        entries[start..]
            .iter()
            .filter_map(|entry| match entry {
                Entry::Turn(turn) => Some(crate::work::spent(&turn.usage)),
                _ => None,
            })
            .sum()
    }

    /// Keeps "Working · 8s" counting.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.run.is_some() && !self.compacting() {
            self.refresh_from(self.conversation.entries.len().saturating_sub(1));
            cx.notify();
        }
    }

    /// Makes this a Chat area chat, drawn as a messenger's transcript. Its list tells the chat
    /// when it stops or starts being scrolled to the end, for the Jump to latest pill.
    pub fn set_chat_area(&mut self, cx: &mut Context<Self>) {
        if self.chat_area {
            return;
        }
        self.chat_area = true;
        self.changes = None;
        self.refresh_from(0);
        // No scroll handler for the jump pill: GPUI calls one with the list's state borrowed (so it
        // can't ask `is_scrolled_to_end`), and already redraws the view that draws the list, which
        // is the one that decides the pill.
        cx.notify();
    }

    pub fn is_chat_area(&self) -> bool {
        self.chat_area
    }

    /// Entry `index`'s items, appended to `items`.
    fn build_entry(&mut self, index: usize, now: u64, items: &mut Vec<Item>) {
        if self.chat_area {
            crate::chat_transcript::entry_items(&self.conversation, index, &self.cwd, &self.open, &mut self.cache, items);
        } else {
            work::entry_items(&self.conversation, index, &self.cwd, &self.open, now, &mut self.cache, items);
        }
    }

    /// The entry whose items show the work log or row with this key.
    fn entry_holding(&self, key: &str) -> Option<usize> {
        let mut start = 0;
        for (index, span) in self.spans.iter().enumerate() {
            let holds = self.items[start..start + span].iter().any(|item| match item {
                Item::Work(work) => work.key.as_ref() == key || work.rows.iter().any(|row| row.key.as_ref() == key),
                Item::Commit(commit) => commit.key.as_ref() == key,
                Item::News(row) => row.key.as_ref() == key,
                Item::Prompt { command, files, .. } => {
                    command.as_ref().is_some_and(|command| command.key.as_ref() == key)
                        || files.iter().any(|file| file.key.as_ref() == key)
                }
                Item::Changes(card) => card.holds(key),
                Item::Suggestions(tray) => tray.cards.iter().any(|card| card.holds(key)),
                Item::Compacted(card) | Item::ChatTidied(card) => card.key.as_ref() == key,
                Item::ChatThought { key: thought, .. } => thought.as_ref() == key,
                Item::ChatChips(chips) => chips.iter().any(|chip| matches!(chip, crate::chat_transcript::Chip::Fold { key: fold, .. } if fold.as_ref() == key)),
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
        let mut items = std::mem::take(&mut self.items);
        for index in from..self.conversation.entries.len() {
            let before = items.len();
            self.build_entry(index, now, &mut items);
            self.spans.push(items.len() - before);
        }
        self.items = items;
        self.fill_canvas_cards(start);
        self.tell_list(start, old_end - start, self.items.len() - start);
    }

    /// Gives the canvas cards from item `from` on their artboards' names and pictures.
    fn fill_canvas_cards(&mut self, from: usize) {
        let mut items = std::mem::take(&mut self.items);
        let from = from.min(items.len());
        self.fill_faces(&mut items[from..]);
        self.items = items;
    }

    fn fill_faces(&mut self, items: &mut [Item]) {
        for item in items {
            let Item::Canvas(card) = item else { continue };
            for board in &mut card.boards {
                let key = (card.canvas.clone(), board.id.clone());
                if !self.canvas_faces.contains_key(&key) {
                    let face = crate::canvas_card::face(&self.canvases, &card.canvas, &board.id);
                    self.canvas_faces.insert(key.clone(), face);
                }
                board.wear(&self.canvas_faces[&key]);
            }
        }
    }

    /// Rebuilds one entry's items in place.
    fn refresh_entry(&mut self, index: usize) {
        let Some(&old_count) = self.spans.get(index) else {
            return;
        };
        let start: usize = self.spans[..index].iter().sum();
        let mut fresh = Vec::new();
        self.build_entry(index, now_millis(), &mut fresh);
        self.fill_faces(&mut fresh);
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
/// Redraws the chat as its tasks change, and tells the app when one has news. While any runs, it
/// also redraws each second, so their times keep counting.
fn watch_tasks(tasks: &Tasks, cx: &mut Context<ChatSession>) -> Task<()> {
    let (sender, mut receiver) = mpsc::unbounded::<Signal>();
    tasks.observe(move |signal| {
        let _ = sender.unbounded_send(signal);
    });
    cx.spawn(async move |this, cx| {
        loop {
            let first = futures::select_biased! {
                signal = receiver.next() => match signal {
                    Some(signal) => Some(signal),
                    None => return,
                },
                () = futures::FutureExt::fuse(cx.background_executor().timer(Duration::from_secs(1))) => None,
            };
            let mut news = first == Some(Signal::News);
            while let Ok(signal) = receiver.try_recv() {
                news |= signal == Signal::News;
            }
            let kept = this.update(cx, |chat, cx| {
                if first.is_some() || !chat.tasks.running().is_empty() {
                    cx.notify();
                }
                if news {
                    cx.emit(ChatEvent::TasksNews);
                }
            });
            if kept.is_err() {
                return;
            }
            // Output can arrive in a flood: redraw at most ten times a second.
            cx.background_executor().timer(Duration::from_millis(100)).await;
        }
    })
}

/// Redraws the chat as its sub-agents work, and tells the app when one in the background has
/// reported. While any works, it also redraws each second, so their times keep counting.
/// Redraws the chat as its canvases change and their pictures are drawn, and tells the app when
/// the agent starts one. Signals come in bursts (a call draws several artboards), so they are
/// taken in batches, at most ten redraws a second.
fn watch_canvases(canvases: &wack_canvas::Canvases, cx: &mut Context<ChatSession>) -> Task<()> {
    let (sender, mut receiver) = mpsc::unbounded::<wack_canvas::Signal>();
    canvases.set_observer(move |signal| {
        let _ = sender.unbounded_send(signal);
    });
    cx.spawn(async move |this, cx| {
        while let Some(first) = receiver.next().await {
            let mut started = false;
            let mut signal = Some(first);
            while let Some(next) = signal {
                if matches!(next, wack_canvas::Signal::Changed { created: true, .. }) {
                    started = true;
                }
                signal = receiver.try_recv().ok();
            }
            let kept = this.update(cx, |chat, cx| {
                if started {
                    cx.emit(ChatEvent::AgentStartedCanvas);
                }
                chat.panes.refresh_canvas(cx);
                chat.comment_drafts = chat.canvases.drafts();
                chat.canvas_faces.clear();
                chat.fill_canvas_cards(0);
                cx.notify();
            });
            if kept.is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_millis(100)).await;
        }
    })
}

/// Redraws the Browser pane as the chat's tabs change. A page sends many events (every request,
/// every console line), so they are taken in batches, at most ten redraws a second.
fn watch_browser(tabs: &wack_browser::ChatTabs, cx: &mut Context<ChatSession>) -> Task<()> {
    let (sender, mut receiver) = mpsc::unbounded::<wack_browser::Signal>();
    // What the chat loaded with, so the first change after it is saved.
    let restored = tabs.saved();
    tabs.set_observer(move |signal| {
        let _ = sender.unbounded_send(signal);
    });
    cx.spawn(async move |this, cx| {
        let mut saved = restored;
        while let Some(first) = receiver.next().await {
            // Read every signal in the batch: an agent's new tab is one worth acting on.
            let mut opened = vec![];
            let mut signal = Some(first);
            while let Some(next) = signal {
                if let wack_browser::Signal::Opened(id) = next {
                    opened.push(id);
                }
                signal = receiver.try_recv().ok();
            }
            let kept = this.update(cx, |chat, cx| {
                // Restored tabs were listed before this watch began, so these are new.
                if opened.iter().any(|id| chat.tabs.get(*id).is_some_and(|tab| tab.owner != wack_browser::Owner::User)) {
                    cx.emit(ChatEvent::AgentOpenedTab);
                }
                chat.panes.refresh_browser(cx);
                let now = chat.tabs.saved();
                if saved != now {
                    cx.emit(ChatEvent::BrowserTabsChanged(now.clone()));
                    saved = now;
                }
            });
            if kept.is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_millis(100)).await;
        }
    })
}

fn watch_agents(agents: &Agents, cx: &mut Context<ChatSession>) -> Task<()> {
    let (sender, mut receiver) = mpsc::unbounded::<AgentSignal>();
    agents.set_observer(move |signal| {
        let _ = sender.unbounded_send(signal);
    });
    cx.spawn(async move |this, cx| {
        loop {
            let first = futures::select_biased! {
                signal = receiver.next() => match signal {
                    Some(signal) => Some(signal),
                    None => return,
                },
                () = futures::FutureExt::fuse(cx.background_executor().timer(Duration::from_secs(1))) => None,
            };
            let mut news = first == Some(AgentSignal::News);
            while let Ok(signal) = receiver.try_recv() {
                news |= signal == AgentSignal::News;
            }
            let kept = this.update(cx, |chat, cx| {
                if first.is_some() || !chat.agents.active().is_empty() {
                    chat.sync_agents();
                    chat.panes.refresh_agents(cx);
                    cx.notify();
                }
                let count = chat.agents.list().len();
                if count > chat.agents_seen {
                    chat.agents_seen = count;
                    cx.emit(ChatEvent::AgentStarted);
                }
                if news {
                    cx.emit(ChatEvent::AgentsNews);
                }
            });
            if kept.is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_millis(100)).await;
        }
    })
}

/// The agents a saved chat last listed.
fn last_agents(history: &[HistoryEntry]) -> Vec<wack_core::agents::AgentInfo> {
    history
        .iter()
        .rev()
        .find_map(|entry| match entry {
            HistoryEntry::State { key, value } if key == wack_core::agents::AGENTS_STATE => serde_json::from_value(value.clone()).ok(),
            _ => None,
        })
        .unwrap_or_default()
}

fn last_context(history: &[HistoryEntry]) -> Option<u64> {
    for entry in history.iter().rev() {
        match entry {
            HistoryEntry::Compacted { .. } => return None,
            HistoryEntry::Message { usage: Some(usage), .. } => return Some(usage.context_tokens() + usage.output),
            HistoryEntry::Message { .. } | HistoryEntry::Note { .. } | HistoryEntry::State { .. } | HistoryEntry::Rewound { .. } => {}
        }
    }
    None
}
