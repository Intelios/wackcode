//! The turn loop: send the conversation, run the tools the model calls, repeat until it stops.
//!
//! Invariants the loop keeps, which the session and every provider rely on:
//! - Every tool call gets exactly one result, in the message straight after the call, whatever
//!   happens: a refusal, a failure, or the user stopping the run.
//! - The session is written before anything acts on it, so a crash never loses a step that ran.
//! - The system prompt and the start of the tool list never change during a run (prompt caching).
//! - A feature's notes are saved before the user's message at the start of a run, so the user's
//!   request is always the last thing the model reads; later in a run they follow the tool
//!   results.
//! - Whether a tool may run is judged when it is about to run, not when the reply arrives: a call
//!   that changes the mode decides for the calls after it in the same reply.
//!
//! Nothing here asks the user for approval before a tool runs: that is out of scope by design.
//! `Feature::on_tool_call` exists for modes that forbid tools outright (a plan mode, a confined
//! chat), not for prompts. A tool can still put a question of its own to the user and wait for
//! the answer ([`crate::tool::User`]): the question leaves as [`Event::Question`] and the answer
//! comes back through [`Control::answer`].

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use futures::future::join_all;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use wack_llm::{
    Block, Delta, Effort, ErrorKind, Image, Message, Provider, Request, RetryNotice, Role, StopReason, SystemPart, Usage,
    stream_with_retry,
};

use crate::{
    agents::Agents,
    ask::{Answer, Question},
    commands::CommandUse,
    compaction::{self, Summary},
    images::{self, Images},
    prompt,
    registry::{Registry, TOOL_SEARCH},
    session::{Note, Session},
    tasks::Tasks,
    tool::{Loading, Seen, Tool, ToolContext, ToolOutput, ToolSpec, User},
    usage::{self, CallOutcome, Purpose, Recorder},
};

/// Compaction starts when the conversation fills this share of the model's context window.
pub const COMPACT_AT_PERCENT: u64 = 80;
/// The conversation text sent to the summariser when the window's size is unknown.
const DEFAULT_SUMMARY_BUDGET_CHARS: usize = 300_000;
const STOPPED: &str = "Stopped by the user before this tool finished.";

/// What kind of agent this is: the coding agent, or the Chat area's assistant. Sub-agent roles
/// are code profiles with their own core text.
#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    /// The core of the system prompt.
    pub prompt: String,
    /// The Chat area's agent: its guide is the chat one (no folder, Git or instruction files),
    /// and its file tools are confined to its working directory, the chat's scratchpad.
    pub chat: bool,
}

impl Profile {
    pub fn code() -> Self {
        Self::code_with(None)
    }

    pub fn code_with(core: Option<&str>) -> Self {
        Self { name: "code".to_string(), prompt: core.unwrap_or(prompt::CODE_PROMPT).to_string(), chat: false }
    }

    pub fn chat() -> Self {
        Self::chat_with(None)
    }

    pub fn chat_with(core: Option<&str>) -> Self {
        Self { name: "chat".to_string(), prompt: core.unwrap_or(prompt::CHAT_PROMPT).to_string(), chat: true }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub model: String,
    pub effort: Option<Effort>,
    pub max_output_tokens: Option<u32>,
    /// The model's context size in tokens, when known. Without it compaction still happens, but
    /// only once the provider refuses a request as too long.
    pub context_window: Option<u64>,
    /// The model reads pictures. When it does not, tools' pictures are left out of its requests
    /// and their results say so (`images::prepare`).
    pub accepts_images: bool,
    /// Ultra: the user asked for the most thorough work the model can do. It adds an instruction
    /// to the system prompt; the effort itself is `effort`, which the caller sets to the top.
    pub ultra: bool,
    /// Replaces `prompt::ULTRA_PROMPT` while `ultra` is on (Settings › Prompts).
    pub ultra_prompt: Option<String>,
    /// Replaces `compaction::SUMMARY_FORMAT` when compacting (Settings › Prompts).
    pub compaction_prompt: Option<String>,
    /// Ask the model to end each commit message it writes with a `Co-authored-by` trailer naming
    /// the active model and WackCode Agent (`prompt::co_author_trailer`).
    pub commit_attribution: bool,
}

pub enum Verdict {
    Allow,
    /// The tool does not run, and the model is told why.
    Deny(String),
}

/// A compiled-in feature's hooks into the loop. All default to doing nothing.
pub trait Feature: Send + Sync {
    /// Called when a run starts, before the user's message is saved.
    fn on_run_start(&self) {}

    /// Messages the model is owed before its next request (a mode contract). Called before the
    /// user's message is saved, before every request after that, and after a compaction; the
    /// loop saves what comes back. Return nothing once the session shows the model was told.
    fn notes(&self, _session: &Session) -> Vec<Note> {
        Vec::new()
    }

    /// True when `notes` has something new the model should read before it finishes: a reply
    /// that would end the run goes round again instead.
    fn pending(&self) -> bool {
        false
    }

    /// Called with each request just before it is sent. Changes here are never saved.
    fn before_request(&self, _request: &mut Request) {}

    /// Called just before a tool runs.
    fn on_tool_call(&self, _tool: &ToolSpec, _input: &Value) -> Verdict {
        Verdict::Allow
    }

    /// Called with each reply once it is saved.
    fn after_turn(&self, _reply: &Message) {}

    /// Called when the model finishes on its own, with its last reply's text. A returned
    /// sentence is shown to the user as a notice.
    fn on_run_end(&self, _reply: &str) -> Option<String> {
        None
    }

    /// State to save in the session, as `(key, value)`. The loop asks at the start and end of a
    /// run and after every batch of tools, and writes only what changed.
    fn state_to_save(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    /// Text to add to a compaction's summary in code, each with its own heading.
    fn summary_sections(&self) -> Vec<String> {
        Vec::new()
    }
}

/// How the user reaches into a run from outside: to stop it, to send a message while it works,
/// or to answer a question a tool asked.
#[derive(Clone, Default)]
pub struct Control {
    cancel: CancellationToken,
    steering: Arc<Mutex<VecDeque<String>>>,
    interactive: Arc<AtomicBool>,
    questions: Arc<Questions>,
}

#[derive(Default)]
struct Questions {
    next: AtomicU64,
    waiting: Mutex<HashMap<String, oneshot::Sender<Answer>>>,
}

/// Forgets a question nobody will answer: the run was stopped while it waited.
struct Waiting<'a> {
    questions: &'a Questions,
    id: String,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.questions.waiting.lock().unwrap().remove(&self.id);
    }
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// A control for a run someone is watching, who can answer questions.
    pub fn with_user() -> Self {
        let control = Self::default();
        control.interactive.store(true, Ordering::SeqCst);
        control
    }

    pub fn interactive(&self) -> bool {
        self.interactive.load(Ordering::SeqCst)
    }

    /// Answers the question with this id. False when it is no longer waiting.
    pub fn answer(&self, id: &str, answer: Answer) -> bool {
        match self.questions.waiting.lock().unwrap().remove(id) {
            Some(sender) => sender.send(answer).is_ok(),
            None => false,
        }
    }

    /// Registers a question and returns its id with the wait for its answer.
    pub(crate) fn open_question(&self) -> (String, impl Future<Output = Answer> + '_) {
        let id = format!("q{}", self.questions.next.fetch_add(1, Ordering::SeqCst) + 1);
        let (sender, receiver) = oneshot::channel();
        self.questions.waiting.lock().unwrap().insert(id.clone(), sender);
        let waiting = Waiting { questions: &self.questions, id: id.clone() };
        (id, async move {
            let _waiting = waiting;
            receiver.await.unwrap_or(Answer::Cancelled)
        })
    }

    /// Stops the run. Running tools are cut off and the reply being written is dropped.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Fires when the run is stopped, for work done on a run's behalf before the loop starts
    /// (waiting for an MCP server).
    pub fn stop_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Queues a message for the model. It is delivered with the next tool results, or as a new
    /// turn if the model was about to finish.
    pub fn steer(&self, text: impl Into<String>) {
        self.steering.lock().unwrap().push_back(text.into());
    }

    fn take_steering(&self) -> Option<String> {
        let mut queue = self.steering.lock().unwrap();
        if queue.is_empty() {
            return None;
        }
        Some(queue.drain(..).collect::<Vec<_>>().join("\n\n"))
    }
}

/// What a run reports as it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Text(String),
    Thinking(String),
    /// The model began writing a tool call. Its input follows in pieces.
    ToolStart { id: String, name: String },
    ToolInput { id: String, json: String },
    /// A complete tool call, about to run.
    ToolCall { id: String, name: String, input: Value },
    /// `images` are the viewing copies the model is sent (each with its saved original's id).
    ToolResult { id: String, name: String, content: String, is_error: bool, images: Vec<Image> },
    /// One reply from the model is complete and saved.
    Reply { usage: Option<Usage> },
    Retrying(RetryNotice),
    /// A message the user sent mid-run was handed to the model.
    Steered(String),
    Compacting,
    Compacted { usage: Option<Usage> },
    /// A tool is waiting for the user to answer. Answer with [`Control::answer`]. `wrap_up`
    /// offers Ultra Plan's "Write the plan now".
    Question { id: String, questions: Vec<Question>, wrap_up: bool },
    /// A feature saved its state: the todo list, the plan awaiting review.
    State { key: String, value: Value },
    /// A note the app wrote for the model was saved (a background task's news).
    Noted { tag: String, text: String },
    /// Something the user should know that is not part of the conversation.
    Notice(String),
}

pub type EventSink<'a> = &'a (dyn Fn(Event) + Send + Sync);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Stopped,
    /// The reply hit the output limit before it finished.
    CutOff,
    Refused,
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    registry: Arc<Registry>,
    session: Session,
    settings: Settings,
    features: Vec<Box<dyn Feature>>,
    system: Vec<SystemPart>,
    cwd: PathBuf,
    tasks: Tasks,
    agents: Agents,
    recorder: Option<Arc<dyn Recorder>>,
    /// The sub-agent this is, for tools that keep its work apart (its own browser tabs).
    caller: Option<String>,
    /// Where the chat keeps its pictures: beside its session, or its parent's for a sub-agent.
    images: Images,
    /// The one folder file tools may reach (a Chat chat's scratchpad), from the profile.
    confine: Option<PathBuf>,
}

enum Planned {
    Run { tool: Arc<dyn Tool>, spec: ToolSpec },
    Settled(Result<String, String>),
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        registry: Arc<Registry>,
        session: Session,
        profile: &Profile,
        settings: Settings,
    ) -> Self {
        let cwd = session.cwd().to_path_buf();
        let images = Images::for_session(session.path());
        let specs: Vec<_> = registry.specs().collect();
        let mut system = if profile.chat {
            prompt::chat_system(&profile.prompt, &prompt::today(), &specs)
        } else {
            let co_author = settings.commit_attribution.then(|| prompt::co_author_label(&settings.model));
            prompt::system(&profile.prompt, &cwd, &prompt::today(), &specs, co_author.as_deref())
        };
        if settings.ultra {
            let text = settings.ultra_prompt.as_deref().unwrap_or(prompt::ULTRA_PROMPT).to_string();
            system.push(SystemPart { text });
        }
        let confine = profile.chat.then(|| cwd.clone());
        Self {
            provider,
            registry,
            session,
            settings,
            features: Vec::new(),
            system,
            cwd,
            tasks: Tasks::scratch(),
            agents: Agents::new(),
            recorder: None,
            caller: None,
            images,
            confine,
        }
    }

    /// Keeps pictures in `images` instead of beside this agent's session: a sub-agent shares its
    /// chat's.
    pub fn with_images(mut self, images: Images) -> Self {
        self.images = images;
        self
    }

    /// Adds a part to the system prompt after the project instructions and before Ultra's, for a
    /// feature whose text is read when its run starts (memory's index).
    pub fn with_system_part(mut self, text: String) -> Self {
        let at = self.system.len() - usize::from(self.settings.ultra);
        self.system.insert(at, SystemPart { text });
        self
    }

    /// Runs shell commands as tasks of this registry, which outlives the run. Without one, a run
    /// has a registry of its own that goes with it.
    pub fn with_tasks(mut self, tasks: Tasks) -> Self {
        self.tasks = tasks;
        self
    }

    /// The chat's sub-agents, which tools may wait on. Without them, a run has none.
    pub fn with_agents(mut self, agents: Agents) -> Self {
        self.agents = agents;
        self
    }

    pub fn with_feature(mut self, feature: Box<dyn Feature>) -> Self {
        self.features.push(feature);
        self
    }

    /// Reports every model invocation to this recorder, for the usage ledger.
    /// Marks this agent as the sub-agent `id`: its tool calls carry it (`ToolContext::caller`).
    pub fn with_caller(mut self, id: impl Into<String>) -> Self {
        self.caller = Some(id.into());
        self
    }

    pub fn with_recorder(mut self, recorder: Arc<dyn Recorder>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Tells the recorder about one finished model invocation.
    fn record(&self, purpose: Purpose, started: Instant, outcome: CallOutcome, usage: Option<Usage>) {
        usage::report(&self.recorder, purpose, &self.settings.model, started, outcome, usage);
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Runs one user message to completion: until the model stops calling tools, or the user
    /// stops it. An `Err` is a failure the user has to see; the session stays valid either way.
    pub async fn run(&mut self, prompt: &str, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        self.run_as(prompt, None, control, events).await
    }

    /// [`Agent::run`] for a message the user sent as a slash command: `prompt` is the expanded
    /// text the model gets, and `command` what they typed, saved beside it for the transcript.
    pub async fn run_as(&mut self, prompt: &str, command: Option<CommandUse>, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        self.run_with(prompt, Vec::new(), command, control, events).await
    }

    /// [`Agent::run_as`] with pictures the user attached, after the text. Each is saved for the
    /// chat and sent as its viewing copy (`images::ingest`), as a tool's are; one that can't be
    /// read is left out, which the user was told when they attached it.
    pub async fn run_with(
        &mut self,
        prompt: &str,
        images: Vec<Image>,
        command: Option<CommandUse>,
        control: &Control,
        events: EventSink<'_>,
    ) -> Result<Outcome, String> {
        for feature in &self.features {
            feature.on_run_start();
        }
        self.save_state(events)?;
        self.save_notes(events)?;
        let mut message = Message::user(prompt);
        if prompt.is_empty() {
            message.content.clear();
        }
        if !images.is_empty() {
            let ready = images::ingest(&self.images, ToolOutput::with_images(String::new(), images)).await;
            message.content.extend(ready.images.into_iter().map(|image| Block::Image { media_type: image.media_type, data: image.data, label: image.label, id: image.id }));
        }
        self.session.push_as(message, None, command)?;
        self.turns(control, events).await
    }

    /// Runs with no message from the user, for news a feature has for the model (a background
    /// task finished while the chat was idle). Done at once, sending nothing, when there is none.
    pub async fn wake(&mut self, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        for feature in &self.features {
            feature.on_run_start();
        }
        self.save_state(events)?;
        if self.save_notes(events)? == 0 {
            return Ok(Outcome::Done);
        }
        self.turns(control, events).await
    }

    /// Carries on from the session as it is, adding nothing: for a session that already ends
    /// with what the model should answer (a sub-agent started from a copy of a conversation).
    pub async fn carry_on(&mut self, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        for feature in &self.features {
            feature.on_run_start();
        }
        self.save_state(events)?;
        self.turns(control, events).await
    }

    /// Sends `system` as the system prompt instead of the one built from the profile: for a
    /// sub-agent that must send exactly what its parent sends.
    pub fn with_system(mut self, system: Vec<SystemPart>) -> Self {
        self.system = system;
        self
    }

    /// Asks the model and runs its tools until it stops.
    async fn turns(&mut self, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        let mut compacted_for_overflow = false;
        loop {
            if control.stopped() {
                return Ok(Outcome::Stopped);
            }
            if self.over_threshold() {
                // A failure here is not fatal: the request is still tried, and if it really is
                // too long the overflow path below reports why.
                let _ = self.compact(None, false, control, events).await;
                if control.stopped() {
                    return Ok(Outcome::Stopped);
                }
            }
            // A tool may have changed the mode, a compaction summarised a note away, or a
            // background task has news.
            self.save_notes(events)?;

            let request = self.request();
            let written = Mutex::new(String::new());
            let sink = |delta: Delta| {
                events(match delta {
                    Delta::Text(text) => {
                        written.lock().unwrap().push_str(&text);
                        Event::Text(text)
                    }
                    Delta::Thinking(text) => Event::Thinking(text),
                    Delta::ToolStart { id, name } => Event::ToolStart { id, name },
                    Delta::ToolInput { id, json } => Event::ToolInput { id, json },
                })
            };
            let retrying = |notice: RetryNotice| events(Event::Retrying(notice));
            let started = Instant::now();
            let result = tokio::select! {
                biased;
                () = control.cancel.cancelled() => None,
                result = stream_with_retry(self.provider.as_ref(), &request, &sink, &retrying) => Some(result),
            };
            let response = match result {
                None => {
                    self.record(Purpose::Chat, started, CallOutcome::Cancelled, None);
                    // The user saw this text appear, so it stays in the conversation.
                    let text = written.into_inner().unwrap();
                    if !text.trim().is_empty() {
                        self.session.push(Message { role: Role::Assistant, content: vec![Block::Text { text }] }, None)?;
                    }
                    return Ok(Outcome::Stopped);
                }
                Some(Err(error)) if error.kind == ErrorKind::ContextOverflow && !compacted_for_overflow => {
                    self.record(Purpose::Chat, started, CallOutcome::Failed, None);
                    compacted_for_overflow = true;
                    match self.compact(None, true, control, events).await {
                        Ok(()) => continue,
                        Err(_) if control.stopped() => return Ok(Outcome::Stopped),
                        Err(reason) => {
                            return Err(format!("The conversation is too long for this model, and compacting it failed: {reason}"));
                        }
                    }
                }
                Some(Err(error)) => {
                    self.record(Purpose::Chat, started, CallOutcome::Failed, None);
                    return Err(error.message);
                }
                Some(Ok(response)) => {
                    self.record(Purpose::Chat, started, CallOutcome::Completed, response.usage);
                    response
                }
            };
            compacted_for_overflow = false;

            self.session.push(response.message.clone(), response.usage)?;
            events(Event::Reply { usage: response.usage });
            for feature in &self.features {
                feature.after_turn(&response.message);
            }

            let calls: Vec<(String, String, Value)> = response
                .message
                .tool_uses()
                .map(|(id, name, input)| (id.to_string(), name.to_string(), input.clone()))
                .collect();
            if calls.is_empty() {
                // The provider paused a long turn of its own tools; sending it back resumes it.
                if response.stop == StopReason::Other("pause_turn".to_string()) {
                    continue;
                }
                if let Some(text) = control.take_steering() {
                    self.session.push(Message::user(text.clone()), None)?;
                    events(Event::Steered(text));
                    continue;
                }
                // News that arrived while the model wrote its last reply is read before it stops.
                if self.features.iter().any(|feature| feature.pending()) {
                    continue;
                }
                let reply = response.message.text();
                for feature in &self.features {
                    if let Some(notice) = feature.on_run_end(&reply) {
                        events(Event::Notice(notice));
                    }
                }
                self.save_state(events)?;
                return Ok(match response.stop {
                    StopReason::MaxTokens => Outcome::CutOff,
                    StopReason::Refusal => Outcome::Refused,
                    _ => Outcome::Done,
                });
            }

            let cut_off = response.stop == StopReason::MaxTokens;
            let mut content = self.run_tools(&calls, cut_off, control, events).await?;
            images::cap_turn(&mut content);
            if control.stopped() {
                self.session.push(Message { role: Role::User, content }, None)?;
                self.save_state(events)?;
                return Ok(Outcome::Stopped);
            }
            let ended = self.ends_run(&calls, &content);
            let steered = control.take_steering();
            if let Some(text) = &steered {
                // Results come first: a provider reads the message's start as the calls' answers.
                content.push(Block::Text { text: text.clone() });
                events(Event::Steered(text.clone()));
            }
            self.session.push(Message { role: Role::User, content }, None)?;
            self.save_state(events)?;
            // What the user just sent still deserves an answer, so it keeps the run going.
            if ended && steered.is_none() {
                return Ok(Outcome::Done);
            }
        }
    }

    /// Whether a tool that ends the run succeeded in this batch.
    fn ends_run(&self, calls: &[(String, String, Value)], results: &[Block]) -> bool {
        calls.iter().zip(results).any(|((_, name, _), result)| {
            self.registry.get(name).is_some_and(|(spec, _)| spec.ends_run) && matches!(result, Block::ToolResult { is_error: false, .. })
        })
    }

    /// Saves the notes features owe the model, and says how many there were.
    fn save_notes(&mut self, events: EventSink<'_>) -> Result<usize, String> {
        let notes: Vec<Note> = self.features.iter().flat_map(|feature| feature.notes(&self.session)).collect();
        let count = notes.len();
        for note in notes {
            let (tag, text) = (note.tag.clone(), note.text.clone());
            self.session.push_note(note)?;
            events(Event::Noted { tag, text });
        }
        Ok(count)
    }

    /// Saves each feature's state that changed, and tells whoever is watching.
    fn save_state(&mut self, events: EventSink<'_>) -> Result<(), String> {
        let states: Vec<(String, Value)> = self.features.iter().flat_map(|feature| feature.state_to_save()).collect();
        for (key, value) in states {
            if self.session.set_state(&key, value.clone())? {
                events(Event::State { key, value });
            }
        }
        Ok(())
    }

    fn request(&self) -> Request {
        let mut request = Request {
            model: self.settings.model.clone(),
            system: self.system.clone(),
            messages: self.session.messages().to_vec(),
            tools: self.registry.defs(self.session.loaded_tools(), self.provider.native_tool_search()),
            max_output_tokens: self.settings.max_output_tokens,
            effort: self.settings.effort,
        };
        images::prepare(&mut request.messages, self.settings.accepts_images);
        for feature in &self.features {
            feature.before_request(&mut request);
        }
        request
    }

    /// What the next request would hold, part by part, adding up to [`Session::context_tokens`]
    /// (`crate::context`).
    pub fn context(&self) -> Vec<crate::context::Part> {
        let mut request = Request {
            model: self.settings.model.clone(),
            system: self.system.clone(),
            messages: self.session.messages().to_vec(),
            tools: self.registry.defs(self.session.loaded_tools(), self.provider.native_tool_search()),
            max_output_tokens: None,
            effort: None,
        };
        for feature in &self.features {
            feature.before_request(&mut request);
        }
        crate::context::breakdown(&request, self.session.context_tokens())
    }

    fn over_threshold(&self) -> bool {
        self.settings.context_window.is_some_and(|window| self.session.context_tokens() > window * COMPACT_AT_PERCENT / 100)
    }

    /// Decides what happens to each call, runs the ones that run, and returns one result per
    /// call, in the order the model made them.
    async fn run_tools(
        &mut self,
        calls: &[(String, String, Value)],
        cut_off: bool,
        control: &Control,
        events: EventSink<'_>,
    ) -> Result<Vec<Block>, String> {
        let mut plans = Vec::with_capacity(calls.len());
        for (id, name, input) in calls {
            let plan = if cut_off {
                // The input may be missing its end even when it happens to parse.
                Planned::Settled(Err("The reply reached its length limit before this tool call was complete, so it did not run. Call it again, with a smaller input if it was large.".to_string()))
            } else if !input.is_object() {
                Planned::Settled(Err("The tool's input was not valid JSON, so it did not run. Call it again with complete input.".to_string()))
            } else if name == TOOL_SEARCH {
                Planned::Settled(self.tool_search(input))
            } else if let Some((spec, tool)) = self.registry.get(name) {
                let (spec, tool) = (spec.clone(), tool.clone());
                if spec.loading == Loading::Deferred {
                    // Also on the provider's own search path, so a chat that later moves to a
                    // provider without one still offers the tools it was using.
                    self.session.load_tool(name)?;
                }
                Planned::Run { tool, spec }
            } else {
                Planned::Settled(Err(format!("There is no tool named {name}.")))
            };
            events(Event::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() });
            plans.push(plan);
        }

        let features = &self.features;
        let (cwd, tasks, agents, pictures) = (&self.cwd, &self.tasks, &self.agents, &self.images);
        let seen = Seen {
            system: &self.system,
            messages: self.session.messages(),
            registry: Some(&self.registry),
            loaded: self.session.loaded_tools(),
            settings: Some(&self.settings),
        };
        let run_one = |index: usize| {
            let (id, name, input) = &calls[index];
            let plan = &plans[index];
            let context = ToolContext {
                call_id: id.clone(),
                cwd: cwd.clone(),
                cancel: control.cancel.clone(),
                user: User::new(control.clone(), events),
                tasks: tasks.clone(),
                agents: agents.clone(),
                seen,
                caller: self.caller.clone(),
                images: pictures.clone(),
                confine: self.confine.clone(),
            };
            async move {
                let context = &context;
                let result = match plan {
                    Planned::Settled(result) => result.clone().map(ToolOutput::from),
                    Planned::Run { .. } if control.stopped() => Err(STOPPED.to_string()),
                    // Judged now, not when the reply arrived: an earlier call in this reply may
                    // have changed what is allowed.
                    Planned::Run { tool, spec } => match denied(features, spec, input) {
                        Some(reason) => Err(reason),
                        None => {
                            tokio::select! {
                                biased;
                                () = control.cancel.cancelled() => Err(STOPPED.to_string()),
                                result = tool.run(input.clone(), context) => result,
                            }
                        }
                    },
                };
                let (content, images, is_error) = match result {
                    Ok(output) => {
                        let ToolOutput { text, images } = images::ingest(pictures, output).await;
                        (text, images, false)
                    }
                    Err(content) => (content, Vec::new(), true),
                };
                events(Event::ToolResult { id: id.clone(), name: name.clone(), content: content.clone(), is_error, images: images.clone() });
                Block::ToolResult { tool_use_id: id.clone(), content, is_error, images }
            }
        };

        // Calls run in the model's order. Neighbours that are all safe to run side by side do, and
        // so do calls made to run together (sub-agents); anything else that changes state runs
        // alone, after what came before it has finished.
        let safe = |index: usize| matches!(&plans[index], Planned::Run { spec, .. } if spec.parallel_safe || spec.together);
        let mut results = Vec::with_capacity(calls.len());
        let mut index = 0;
        while index < calls.len() {
            let mut end = index + 1;
            if safe(index) {
                while end < calls.len() && safe(end) {
                    end += 1;
                }
            }
            results.extend(join_all((index..end).map(&run_one)).await);
            index = end;
        }
        Ok(results)
    }

    /// Our own tool search, for providers without one. Found tools join the next request.
    fn tool_search(&mut self, input: &Value) -> Result<String, String> {
        let query = input["query"].as_str().unwrap_or_default();
        let found: Vec<(String, String)> =
            self.registry.search(query).into_iter().map(|spec| (spec.name.clone(), spec.hint.clone())).collect();
        if found.is_empty() {
            return Err(format!("No tools matched \"{query}\". Try other words, or a tool's name from the list in your instructions."));
        }
        let mut lines = vec!["Loaded. You can call these from your next step:".to_string()];
        for (name, hint) in found {
            self.session.load_tool(&name)?;
            lines.push(format!("- {name}: {hint}"));
        }
        Ok(lines.join("\n"))
    }

    /// Replaces the older part of the conversation with a summary. `forced` is for a
    /// conversation the provider has refused as too long, where something must go even if the
    /// estimate says it all fits.
    pub async fn compact(
        &mut self,
        focus: Option<&str>,
        forced: bool,
        control: &Control,
        events: EventSink<'_>,
    ) -> Result<(), String> {
        let messages = self.session.messages();
        // The first message of a compacted session is the previous summary, which is updated
        // from its stored body rather than summarised again.
        let start = usize::from(self.session.summary().is_some());
        // On a small model the usual tail would be most of the window, leaving nothing to gain.
        let keep_tokens = self.settings.context_window.map_or(compaction::KEEP_TOKENS, |window| compaction::KEEP_TOKENS.min(window / 4));
        let mut cut = compaction::cut_index(messages, start, keep_tokens);
        if cut <= start && forced {
            cut = messages.len();
        }
        if cut <= start {
            return Err("There is nothing to compact yet.".to_string());
        }
        events(Event::Compacting);
        let budget = self.settings.context_window.map_or(DEFAULT_SUMMARY_BUDGET_CHARS, |window| window as usize * 2);
        // The app's own notes are neither the conversation nor the user's words.
        let conversation: Vec<Message> =
            (start..cut).filter(|index| !self.session.is_note(*index)).map(|index| messages[index].clone()).collect();
        let request = compaction::request_with(
            &self.settings.model,
            self.session.summary(),
            &conversation,
            focus,
            budget,
            self.settings.compaction_prompt.as_deref(),
        );
        let retrying = |notice: RetryNotice| events(Event::Retrying(notice));
        let started = Instant::now();
        let response = tokio::select! {
            biased;
            () = control.cancel.cancelled() => {
                self.record(Purpose::Compaction, started, CallOutcome::Cancelled, None);
                return Err("Compaction was stopped.".to_string());
            }
            result = stream_with_retry(self.provider.as_ref(), &request, &|_| {}, &retrying) => {
                match result {
                    Ok(response) => {
                        self.record(Purpose::Compaction, started, CallOutcome::Completed, response.usage);
                        response
                    }
                    Err(error) => {
                        self.record(Purpose::Compaction, started, CallOutcome::Failed, None);
                        return Err(error.message);
                    }
                }
            }
        };
        let body = response.message.text();
        if body.trim().is_empty() {
            return Err("The model returned an empty summary.".to_string());
        }
        let mut summary = Summary::build(self.session.summary(), &conversation, body);
        summary.sections = self.features.iter().flat_map(|feature| feature.summary_sections()).collect();
        let keep = messages.len() - cut;
        self.session.compact(summary, keep)?;
        events(Event::Compacted { usage: response.usage });
        Ok(())
    }
}

/// Why this call may not run, if a feature forbids it.
fn denied(features: &[Box<dyn Feature>], tool: &ToolSpec, input: &Value) -> Option<String> {
    features.iter().find_map(|feature| match feature.on_tool_call(tool, input) {
        Verdict::Allow => None,
        Verdict::Deny(reason) => Some(reason),
    })
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::AtomicUsize, time::Duration};

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::{Barrier, Notify};
    use crate::usage::ModelCall;
    use wack_llm::{
        LlmError, Response,
        testing::{ScriptedProvider, Step, call, say},
    };

    use super::*;
    use crate::tool::ToolSpec;

    fn spec(name: &str, loading: Loading, parallel_safe: bool) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("The {name} tool."),
            input_schema: json!({ "type": "object" }),
            loading,
            parallel_safe,
            hint: format!("use {name} for {name} things"),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    #[test]
    fn memory_brings_its_tools_and_an_index_before_ultra() {
        use crate::{kit::Kit, memory::store};

        let folder = tempfile::tempdir().unwrap();
        let memory = folder.path().join("memory");
        store::write(&memory, "user_tabs", store::Kind::User, "Prefers tabs", "Indentation", "Tabs, width 4").unwrap();
        let session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let build = |memory: Option<std::path::PathBuf>, ultra: bool| {
            let kit = Kit::new(&session, None, false, memory);
            let names: Vec<String> = kit.tools().iter().map(|tool| tool.spec().name).collect();
            let settings = Settings {
                model: "m".into(),
                effort: None,
                max_output_tokens: None,
                context_window: None,
                accepts_images: true,
                ultra,
                ultra_prompt: None,
                compaction_prompt: None,
                commit_attribution: false,
            };
            let registry = Arc::new(Registry::new(kit.tools()).unwrap());
            let provider = Arc::new(ScriptedProvider::new(Vec::new()));
            let session = Session::open(session.path()).unwrap();
            (names, kit.attach(Agent::new(provider, registry, session, &Profile::code(), settings)).system)
        };

        let (names, system) = build(Some(memory.clone()), true);
        assert!(crate::memory::MEMORY_TOOLS.iter().all(|tool| names.iter().any(|name| name == tool)));
        let at = system.iter().position(|part| part.text.starts_with("## Project memory")).unwrap();
        assert_eq!(at, system.len() - 2, "memory sits just before Ultra");
        assert!(system[at].text.contains("(name: user_tabs)") && !system[at].text.contains("width 4"));
        assert!(system.last().unwrap().text.starts_with("# Ultra"));

        let (_, system) = build(Some(memory), false);
        assert!(system.last().unwrap().text.starts_with("## Project memory"));

        let (names, system) = build(None, false);
        assert!(!names.iter().any(|name| name.starts_with("memory_")));
        assert!(!system.iter().any(|part| part.text.contains("## Project memory")));
    }

    /// Returns its `text` input.
    struct Echo(&'static str, Loading);

    #[async_trait]
    impl Tool for Echo {
        fn spec(&self) -> ToolSpec {
            spec(self.0, self.1, true)
        }

        async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            Ok(input["text"].as_str().unwrap_or("ok").to_string().into())
        }
    }

    /// Finishes only once two calls are inside it at the same moment.
    struct Together(Barrier);

    #[async_trait]
    impl Tool for Together {
        fn spec(&self) -> ToolSpec {
            spec("together", Loading::Always, true)
        }

        async fn run(&self, _: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            self.0.wait().await;
            Ok("met".to_string().into())
        }
    }

    /// Counts how many calls are inside it at once.
    #[derive(Default)]
    struct Alone {
        inside: AtomicUsize,
        most: AtomicUsize,
    }

    #[async_trait]
    impl Tool for Alone {
        fn spec(&self) -> ToolSpec {
            spec("alone", Loading::Always, false)
        }

        async fn run(&self, _: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            let inside = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(inside, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.inside.fetch_sub(1, Ordering::SeqCst);
            Ok("ran".to_string().into())
        }
    }

    /// Never finishes, and notes when its work is abandoned.
    #[derive(Default)]
    struct Forever {
        started: Arc<Notify>,
        abandoned: Arc<AtomicBool>,
    }

    struct Guard(Arc<AtomicBool>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl Tool for Forever {
        fn spec(&self) -> ToolSpec {
            spec("forever", Loading::Always, false)
        }

        async fn run(&self, _: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            let _guard = Guard(self.abandoned.clone());
            self.started.notify_one();
            std::future::pending().await
        }
    }

    struct Rig {
        agent: Agent,
        provider: ScriptedProvider,
        events: Arc<Mutex<Vec<Event>>>,
        _folder: tempfile::TempDir,
    }

    fn rig_with(provider: ScriptedProvider, tools: Vec<Arc<dyn Tool>>, context_window: Option<u64>) -> Rig {
        rig_with_recorder(provider, tools, context_window, None)
    }

    fn rig_with_recorder(
        provider: ScriptedProvider,
        tools: Vec<Arc<dyn Tool>>,
        context_window: Option<u64>,
        recorder: Option<Arc<dyn Recorder>>,
    ) -> Rig {
        let folder = tempfile::tempdir().unwrap();
        let session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let settings = Settings {
            model: "test-model".into(),
            effort: None,
            max_output_tokens: None,
            context_window,
            accepts_images: true,
            ultra: false,
            ultra_prompt: None,
            compaction_prompt: None,
            commit_attribution: false,
        };
        let agent = Agent::new(
            Arc::new(provider.clone()),
            Arc::new(Registry::new(tools).unwrap()),
            session,
            &Profile::code(),
            settings,
        );
        let agent = match recorder {
            Some(recorder) => agent.with_recorder(recorder),
            None => agent,
        };
        Rig { agent, provider, events: Arc::default(), _folder: folder }
    }

    fn rig(steps: Vec<Step>, tools: Vec<Arc<dyn Tool>>) -> Rig {
        rig_with(ScriptedProvider::new(steps), tools, None)
    }

    /// A rig that reports its model calls to `calls`, for the ledger's tests.
    fn rig_recording(steps: Vec<Step>, calls: Arc<Calls>) -> Rig {
        rig_with_recorder(ScriptedProvider::new(steps), Vec::new(), None, Some(calls))
    }

    /// Collects the model calls a recorder is handed.
    #[derive(Default)]
    struct Calls(Mutex<Vec<crate::ModelCall>>);

    impl Recorder for Calls {
        fn record(&self, call: ModelCall) {
            self.0.lock().unwrap().push(call);
        }
    }

    impl Calls {
        fn summary(&self) -> Vec<(Purpose, CallOutcome, Option<Usage>)> {
            self.0.lock().unwrap().iter().map(|call| (call.purpose, call.outcome, call.usage)).collect()
        }
    }

    impl Rig {
        async fn run(&mut self, prompt: &str, control: &Control) -> Result<Outcome, String> {
            let events = self.events.clone();
            self.agent.run(prompt, control, &move |event| events.lock().unwrap().push(event)).await
        }

        fn results(&self) -> Vec<(String, bool)> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::ToolResult { content, is_error, .. } => Some((content.clone(), *is_error)),
                    _ => None,
                })
                .collect()
        }

        fn saw(&self, wanted: impl Fn(&Event) -> bool) -> bool {
            self.events.lock().unwrap().iter().any(wanted)
        }

        fn tool_names(&self, request: usize) -> Vec<String> {
            self.provider.requests()[request].tools.iter().map(|tool| tool.name.clone()).collect()
        }
    }

    /// Every tool call in the conversation is answered in the very next message.
    fn assert_calls_answered(messages: &[Message]) {
        for (index, message) in messages.iter().enumerate() {
            let calls: Vec<&str> = message.tool_uses().map(|(id, _, _)| id).collect();
            if calls.is_empty() {
                continue;
            }
            let answers: Vec<&str> = messages
                .get(index + 1)
                .map(|next| {
                    next.content
                        .iter()
                        .filter_map(|block| match block {
                            Block::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            assert_eq!(calls, answers, "call and result ids must pair up in order");
        }
    }

    #[tokio::test]
    async fn safe_tools_run_side_by_side_and_results_keep_the_models_order() {
        let steps = vec![
            call(&[("a", "together", json!({})), ("b", "together", json!({})), ("c", "echo", json!({ "text": "third" }))]),
            say("All done."),
        ];
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Together(Barrier::new(2))), Arc::new(Echo("echo", Loading::Always))];
        let mut rig = rig(steps, tools);
        // If the two calls ran one after the other, the first would wait for ever.
        let outcome = tokio::time::timeout(Duration::from_secs(5), rig.run("go", &Control::new())).await.unwrap();
        assert_eq!(outcome, Ok(Outcome::Done));
        assert_calls_answered(rig.agent.session().messages());
        let sent = &rig.provider.requests()[1].messages[2];
        let order: Vec<&str> = sent
            .content
            .iter()
            .map(|block| match block {
                Block::ToolResult { tool_use_id, .. } => tool_use_id.as_str(),
                _ => panic!("only results are expected"),
            })
            .collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn tools_that_change_things_run_one_at_a_time() {
        let alone = Arc::new(Alone::default());
        let steps = vec![call(&[("a", "alone", json!({})), ("b", "alone", json!({})), ("c", "alone", json!({}))]), say("Done.")];
        let mut rig = rig(steps, vec![alone.clone()]);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(alone.most.load(Ordering::SeqCst), 1);
        assert_eq!(rig.results().len(), 3);
    }

    #[tokio::test]
    async fn stopping_mid_tool_abandons_it_and_still_answers_every_call() {
        let forever = Arc::new(Forever::default());
        let (started, abandoned) = (forever.started.clone(), forever.abandoned.clone());
        let steps = vec![call(&[("a", "forever", json!({})), ("b", "echo", json!({}))])];
        let mut rig = rig(steps, vec![forever, Arc::new(Echo("echo", Loading::Always))]);
        let control = Control::new();
        let stopper = control.clone();
        tokio::spawn(async move {
            started.notified().await;
            stopper.stop();
        });
        let outcome = tokio::time::timeout(Duration::from_secs(5), rig.run("go", &control)).await.unwrap();
        assert_eq!(outcome, Ok(Outcome::Stopped));
        assert!(abandoned.load(Ordering::SeqCst), "the running tool was dropped");
        assert_eq!(rig.results(), [(STOPPED.to_string(), true), (STOPPED.to_string(), true)]);
        assert_calls_answered(rig.agent.session().messages());
        assert_eq!(rig.provider.requests().len(), 1, "nothing more is sent after a stop");
    }

    #[tokio::test]
    async fn stopping_mid_reply_keeps_the_text_already_shown() {
        let mut rig = rig(vec![Step::Hang { text: "I will start by".into() }, say("Resumed.")], vec![]);
        let control = Control::new();
        let stopper = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.stop();
        });
        assert_eq!(rig.run("go", &control).await, Ok(Outcome::Stopped));
        assert_eq!(rig.agent.session().messages().last().unwrap().text(), "I will start by");

        // The chat carries on from there.
        assert_eq!(rig.run("continue", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(rig.provider.requests()[1].messages.len(), 3);
    }

    #[tokio::test]
    async fn the_loop_reports_each_model_call() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(vec![say("hello"), say("again")], calls.clone());
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(rig.run("more", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(
            calls.summary(),
            [(Purpose::Chat, CallOutcome::Completed, Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 })); 2]
        );
        let calls = calls.0.lock().unwrap();
        assert_eq!(calls.len(), 2, "one report per model invocation");
        assert_eq!(calls[0].model, "test-model");
    }

    #[tokio::test]
    async fn a_failed_request_is_reported_once() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(vec![Step::Fail(LlmError::new(ErrorKind::Auth, "no."))], calls.clone());
        assert!(rig.run("go", &Control::new()).await.is_err());
        assert_eq!(calls.summary(), [(Purpose::Chat, CallOutcome::Failed, None)]);
    }

    #[tokio::test]
    async fn retries_inside_one_request_are_one_call() {
        let calls = Arc::new(Calls::default());
        let mut rig =
            rig_recording(vec![Step::Fail(LlmError::new(ErrorKind::RateLimited, "slow")), say("done")], calls.clone());
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(
            calls.summary(),
            [(Purpose::Chat, CallOutcome::Completed, Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 }))]
        );
    }

    #[tokio::test]
    async fn a_request_stopped_mid_reply_is_reported_cancelled() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(vec![Step::Hang { text: "writ".into() }], calls.clone());
        let control = Control::new();
        let stopper = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.stop();
        });
        assert_eq!(rig.run("go", &control).await, Ok(Outcome::Stopped));
        assert_eq!(calls.summary(), [(Purpose::Chat, CallOutcome::Cancelled, None)]);
    }

    #[tokio::test]
    async fn compaction_reports_its_own_call_even_when_its_summary_is_empty() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(vec![say("hello"), say("")], calls.clone());
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        assert!(rig.agent.compact(None, true, &Control::new(), &|_| {}).await.is_err());
        assert_eq!(
            calls.summary()[1..],
            [(Purpose::Compaction, CallOutcome::Completed, Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 }))]
        );
    }

    #[tokio::test]
    async fn a_compaction_stopped_mid_request_is_reported_cancelled() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(vec![say("hello"), Step::Hang { text: "sum".into() }], calls.clone());
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let control = Control::new();
        let stopper = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.stop();
        });
        assert!(rig.agent.compact(None, true, &control, &|_| {}).await.is_err());
        assert_eq!(calls.summary()[1..], [(Purpose::Compaction, CallOutcome::Cancelled, None)]);
    }

    #[tokio::test]
    async fn an_overflowing_request_fails_then_compacts_then_carries_on() {
        let calls = Arc::new(Calls::default());
        let mut rig = rig_recording(
            vec![
                Step::Fail(LlmError::new(ErrorKind::ContextOverflow, "too long")),
                say("summary"),
                say("done"),
            ],
            calls.clone(),
        );
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let usage = Some(Usage { input: 100, output: 10, cache_read: 0, cache_write: 0 });
        assert_eq!(
            calls.summary(),
            [
                (Purpose::Chat, CallOutcome::Failed, None),
                (Purpose::Compaction, CallOutcome::Completed, usage),
                (Purpose::Chat, CallOutcome::Completed, usage),
            ]
        );
    }

    #[tokio::test]
    async fn a_message_sent_mid_run_rides_with_the_tool_results() {
        let steps = vec![call(&[("a", "echo", json!({ "text": "one" }))]), say("Switching."), say("Done.")];
        let mut rig = rig(steps, vec![Arc::new(Echo("echo", Loading::Always))]);
        let control = Control::new();
        control.steer("use the other file");
        assert_eq!(rig.run("go", &control).await, Ok(Outcome::Done));

        let requests = rig.provider.requests();
        let carried = &requests[1].messages[2].content;
        assert!(matches!(&carried[0], Block::ToolResult { .. }), "results come before the user's words");
        assert_eq!(carried[1], Block::Text { text: "use the other file".into() });
        assert_eq!(requests.len(), 2);
        assert!(rig.saw(|event| *event == Event::Steered("use the other file".into())));
    }

    #[tokio::test]
    async fn a_message_sent_as_the_model_finishes_starts_another_turn() {
        let mut rig = rig(vec![say("Finished."), say("Also done.")], vec![]);
        let control = Control::new();
        control.steer("one more thing");
        control.steer("and another");
        assert_eq!(rig.run("go", &control).await, Ok(Outcome::Done));
        let requests = rig.provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].messages[2].text(), "one more thing\n\nand another");
    }

    #[tokio::test]
    async fn a_rate_limit_is_retried_and_reported() {
        let limited = LlmError { kind: ErrorKind::RateLimited, message: "Slow down.".into(), retry_after: Some(Duration::ZERO) };
        let mut rig = rig(vec![Step::Fail(limited), say("Here.")], vec![]);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(rig.provider.requests().len(), 2);
        assert!(rig.saw(|event| matches!(event, Event::Retrying(notice) if notice.reason == "Slow down.")));
    }

    #[tokio::test]
    async fn a_failure_that_cannot_be_retried_ends_the_run_with_its_reason() {
        let mut rig = rig(vec![Step::Fail(LlmError::new(ErrorKind::Auth, "The provider rejected the API key."))], vec![]);
        assert_eq!(rig.run("go", &Control::new()).await, Err("The provider rejected the API key.".to_string()));
        assert_eq!(rig.agent.session().messages().len(), 1, "the user's message is kept for the next try");
    }

    #[tokio::test]
    async fn an_overflow_compacts_and_carries_on() {
        let overflow = LlmError::new(ErrorKind::ContextOverflow, "prompt is too long");
        let steps = vec![
            call(&[("a", "echo", json!({ "text": "found it" }))]),
            Step::Fail(overflow),
            say("## Goal\nFix the parser."),
            say("Fixed."),
        ];
        let mut rig = rig(steps, vec![Arc::new(Echo("echo", Loading::Always))]);
        assert_eq!(rig.run("Fix the parser in a.rs", &Control::new()).await, Ok(Outcome::Done));

        let requests = rig.provider.requests();
        assert_eq!(requests.len(), 4);
        // The summary request carries the conversation as text and offers no tools.
        assert!(requests[2].tools.is_empty());
        assert!(requests[2].messages[0].text().contains("[tool result]\nfound it"));
        // The retried request starts from the summary, with the user's request quoted in it.
        let resumed = &requests[3].messages;
        assert_eq!(resumed.len(), 1);
        let summary = resumed[0].text();
        assert!(summary.contains("## Goal\nFix the parser."));
        assert!(summary.contains("<user-message>\nFix the parser in a.rs\n</user-message>"));
        assert!(rig.saw(|event| *event == Event::Compacting));
        assert_eq!(rig.agent.session().messages().len(), 2);
    }

    #[tokio::test]
    async fn an_overflow_straight_after_compacting_is_an_error_not_a_loop() {
        let overflow = || Step::Fail(LlmError::new(ErrorKind::ContextOverflow, "prompt is too long"));
        let mut rig = rig(vec![overflow(), say("Summary."), overflow()], vec![]);
        assert_eq!(rig.run("go", &Control::new()).await, Err("prompt is too long".to_string()));
        assert_eq!(rig.provider.requests().len(), 3);
    }

    #[tokio::test]
    async fn a_full_context_window_compacts_before_the_next_request() {
        let big = "x".repeat(2_000);
        let reply = |text: &str, input: u64| {
            Step::Reply(Response {
                message: Message { role: Role::Assistant, content: vec![Block::Text { text: text.into() }] },
                stop: StopReason::EndTurn,
                usage: Some(Usage { input, output: 10, cache_read: 0, cache_write: 0 }),
            })
        };
        let steps = vec![reply("First answer.", 900), reply("Summary body.", 50), reply("Second answer.", 100)];
        let mut rig = rig_with(ScriptedProvider::new(steps), vec![], Some(1_000));
        assert_eq!(rig.run(&big, &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(rig.provider.requests().len(), 1, "the first message fits, so nothing is compacted");

        assert_eq!(rig.run("next", &Control::new()).await, Ok(Outcome::Done));
        let requests = rig.provider.requests();
        assert_eq!(requests.len(), 3);
        let last = &requests[2].messages;
        assert!(last[0].text().contains("Summary body."));
        assert_eq!(last[1].text(), "next", "the newest turn is kept word for word");
    }

    #[tokio::test]
    async fn found_tools_are_added_after_the_ones_already_sent() {
        let steps = vec![
            call(&[("s", TOOL_SEARCH, json!({ "query": "open a web page" }))]),
            call(&[("b", "browser_open", json!({ "text": "opened" }))]),
            say("Done."),
        ];
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(Echo("browser_open", Loading::Deferred)),
            Arc::new(Echo("echo", Loading::Always)),
            Arc::new(Echo("canvas", Loading::Deferred)),
        ];
        let mut rig = rig(steps, tools);
        assert_eq!(rig.run("open example.com", &Control::new()).await, Ok(Outcome::Done));

        assert_eq!(rig.tool_names(0), ["echo", TOOL_SEARCH]);
        assert_eq!(rig.tool_names(1), ["echo", TOOL_SEARCH, "browser_open"]);
        assert_eq!(rig.results()[1], ("opened".to_string(), false));
        assert!(rig.results()[0].0.contains("- browser_open: use browser_open for browser_open things"));
        // The model can read what exists to search for.
        assert!(rig.provider.requests()[0].system[1].text.contains("## More tools"));

        // A reopened chat offers the same tools.
        let path = rig.agent.session().path().to_path_buf();
        assert_eq!(Session::open(&path).unwrap().loaded_tools(), ["browser_open"]);
    }

    #[tokio::test]
    async fn a_provider_with_its_own_search_gets_every_deferred_tool_marked() {
        let steps = vec![call(&[("b", "browser_open", json!({}))]), say("Done.")];
        let tools: Vec<Arc<dyn Tool>> =
            vec![Arc::new(Echo("browser_open", Loading::Deferred)), Arc::new(Echo("echo", Loading::Always))];
        let mut rig = rig_with(ScriptedProvider::new(steps).with_native_tool_search(), tools, None);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));

        let sent = &rig.provider.requests()[0].tools;
        let marks: Vec<(&str, bool)> = sent.iter().map(|tool| (tool.name.as_str(), tool.deferred)).collect();
        assert_eq!(marks, [("echo", false), ("browser_open", true)]);
        assert_eq!(rig.provider.requests()[1].tools, *sent, "the list never changes on this path");
        assert_eq!(rig.agent.session().loaded_tools(), ["browser_open"]);
    }

    #[tokio::test]
    async fn a_chat_killed_mid_tool_resumes_with_the_call_answered() {
        let steps = vec![call(&[("a", "forever", json!({}))])];
        let mut rig = rig(steps, vec![Arc::new(Forever::default())]);
        let path = rig.agent.session().path().to_path_buf();
        let cwd = rig.agent.session().cwd().to_path_buf();
        // The process dies while the tool runs: the run's future is dropped, nothing is cleaned up.
        assert!(tokio::time::timeout(Duration::from_millis(50), rig.run("build it", &Control::new())).await.is_err());
        let folder = rig._folder;
        drop(rig.agent);

        let provider = ScriptedProvider::new(vec![say("Picking up where I left off.")]);
        let settings = Settings {
            model: "test-model".into(),
            effort: None,
            max_output_tokens: None,
            context_window: None,
            accepts_images: true,
            ultra: false,
            ultra_prompt: None,
            compaction_prompt: None,
            commit_attribution: false,
        };
        let session = Session::open(&path).unwrap();
        assert_eq!(session.cwd(), cwd);
        let mut agent = Agent::new(
            Arc::new(provider.clone()),
            Arc::new(Registry::new(vec![]).unwrap()),
            session,
            &Profile::code(),
            settings,
        );
        assert_eq!(agent.run("carry on", &Control::new(), &|_| {}).await, Ok(Outcome::Done));
        let sent = &provider.requests()[0].messages;
        assert_calls_answered(sent);
        assert_eq!(sent.len(), 4);
        assert!(matches!(&sent[2].content[0], Block::ToolResult { is_error: true, content, .. } if content.contains("interrupted")));
        drop(folder);
    }

    struct ReadOnly;

    impl Feature for ReadOnly {
        fn before_request(&self, request: &mut Request) {
            request.system.push(SystemPart { text: "Read-only mode.".into() });
        }

        fn on_tool_call(&self, tool: &ToolSpec, _: &Value) -> Verdict {
            if tool.name == "alone" { Verdict::Deny("This mode does not allow changes.".into()) } else { Verdict::Allow }
        }
    }

    #[tokio::test]
    async fn calls_that_must_not_run_are_answered_with_the_reason() {
        let alone = Arc::new(Alone::default());
        let steps = vec![
            call(&[
                ("a", "alone", json!({})),
                ("b", "missing", json!({})),
                ("c", "echo", Value::String("{\"text\": \"cut".into())),
                ("d", "echo", json!({ "text": "fine" })),
            ]),
            say("Done."),
        ];
        let mut rig = rig(steps, vec![alone.clone(), Arc::new(Echo("echo", Loading::Always))]);
        rig.agent = rig.agent.with_feature(Box::new(ReadOnly));
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));

        let results = rig.results();
        assert_eq!(results[0], ("This mode does not allow changes.".to_string(), true));
        assert_eq!(results[1], ("There is no tool named missing.".to_string(), true));
        assert!(results[2].1 && results[2].0.contains("not valid JSON"));
        assert_eq!(results[3], ("fine".to_string(), false));
        assert_eq!(alone.most.load(Ordering::SeqCst), 0);
        assert_eq!(rig.provider.requests()[0].system.last().unwrap().text, "Read-only mode.");
    }

    /// Ends the run when it succeeds; fails when told to.
    struct Finish;

    #[async_trait]
    impl Tool for Finish {
        fn spec(&self) -> ToolSpec {
            ToolSpec { ends_run: true, ..spec("finish", Loading::Always, false) }
        }

        async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            if input["fail"] == true { Err("Not yet.".to_string()) } else { Ok("Submitted.".into()) }
        }
    }

    #[tokio::test]
    async fn a_tool_that_ends_the_run_is_answered_and_nothing_more_is_asked() {
        let steps = vec![call(&[("a", "finish", json!({ "fail": true }))]), call(&[("b", "finish", json!({}))])];
        let mut rig = rig(steps, vec![Arc::new(Finish)]);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        // The failed call did not end the run; the second did, with its result saved.
        assert_eq!(rig.provider.requests().len(), 2);
        assert_eq!(rig.results(), [("Not yet.".to_string(), true), ("Submitted.".to_string(), false)]);
        assert_calls_answered(rig.agent.session().messages());
        assert_eq!(rig.agent.session().messages().len(), 5);
    }

    #[tokio::test]
    async fn a_message_sent_as_the_run_would_end_keeps_it_going() {
        let steps = vec![call(&[("a", "finish", json!({}))]), say("Revised.")];
        let mut rig = rig(steps, vec![Arc::new(Finish)]);
        let control = Control::new();
        control.steer("one change first");
        assert_eq!(rig.run("go", &control).await, Ok(Outcome::Done));
        assert_eq!(rig.provider.requests().len(), 2);
    }

    /// Asks the user one question and returns what came back.
    struct Asks;

    #[async_trait]
    impl Tool for Asks {
        fn spec(&self) -> ToolSpec {
            spec("asks", Loading::Always, false)
        }

        async fn run(&self, _: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
            if !context.user.present() {
                return Ok("nobody".to_string().into());
            }
            let question = Question {
                id: "colour".into(),
                header: "Colour".into(),
                question: "Which colour?".into(),
                options: Vec::new(),
                multi_select: false,
            };
            Ok(format!("{:?}", context.user.ask(vec![question], true).await).into())
        }
    }

    #[tokio::test]
    async fn a_tool_can_ask_the_user_and_wait_for_the_answer() {
        let steps = vec![call(&[("a", "asks", json!({}))]), say("Blue it is."), call(&[("b", "asks", json!({}))]), say("Fine.")];
        let mut rig = rig(steps, vec![Arc::new(Asks)]);

        let control = Control::with_user();
        let (answerer, events) = (control.clone(), rig.events.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let asked = events.lock().unwrap().iter().find_map(|event| match event {
                    Event::Question { id, questions, wrap_up: true } if questions[0].id == "colour" => Some(id.clone()),
                    _ => None,
                });
                if let Some(id) = asked {
                    assert!(answerer.answer(&id, Answer::WrapUp));
                    assert!(!answerer.answer(&id, Answer::Cancelled), "a question is answered once");
                    return;
                }
            }
        });
        let outcome = tokio::time::timeout(Duration::from_secs(5), rig.run("go", &control)).await.unwrap();
        assert_eq!(outcome, Ok(Outcome::Done));
        assert_eq!(rig.results()[0], ("WrapUp".to_string(), false));

        // With nobody watching, the tool is told so instead of waiting for ever.
        assert_eq!(rig.run("again", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(rig.results()[1], ("nobody".to_string(), false));
    }

    #[tokio::test]
    async fn stopping_while_a_question_waits_drops_it() {
        let mut rig = rig(vec![call(&[("a", "asks", json!({}))])], vec![Arc::new(Asks)]);
        let control = Control::with_user();
        let (stopper, events) = (control.clone(), rig.events.clone());
        tokio::spawn(async move {
            while !events.lock().unwrap().iter().any(|event| matches!(event, Event::Question { .. })) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            stopper.stop();
        });
        let outcome = tokio::time::timeout(Duration::from_secs(5), rig.run("go", &control)).await.unwrap();
        assert_eq!(outcome, Ok(Outcome::Stopped));
        assert_eq!(rig.results(), [(STOPPED.to_string(), true)]);
        assert!(!control.answer("q1", Answer::Cancelled), "nothing is left waiting");
    }

    /// A mode a tool switches on mid-reply: once on, `alone` may not run.
    #[derive(Clone, Default)]
    struct Lockable(Arc<AtomicBool>);

    #[async_trait]
    impl Tool for Lockable {
        fn spec(&self) -> ToolSpec {
            spec("lock", Loading::Always, false)
        }

        async fn run(&self, _: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            self.0.store(true, Ordering::SeqCst);
            Ok("Locked.".to_string().into())
        }
    }

    impl Feature for Lockable {
        fn on_tool_call(&self, tool: &ToolSpec, _: &Value) -> Verdict {
            if self.0.load(Ordering::SeqCst) && tool.name == "alone" { Verdict::Deny("Locked.".into()) } else { Verdict::Allow }
        }
    }

    #[tokio::test]
    async fn a_call_is_judged_when_it_runs_so_an_earlier_call_in_the_reply_can_forbid_it() {
        let (alone, lock) = (Arc::new(Alone::default()), Lockable::default());
        let steps = vec![call(&[("a", "alone", json!({})), ("b", "lock", json!({})), ("c", "alone", json!({}))]), say("Done.")];
        let mut rig = rig(steps, vec![alone.clone(), Arc::new(lock.clone())]);
        rig.agent = rig.agent.with_feature(Box::new(lock));
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let results = rig.results();
        assert_eq!(results[0], ("ran".to_string(), false));
        assert_eq!(results[2], ("Locked.".to_string(), true));
    }

    /// Counts tool batches as its state, and owes the model one note per session.
    #[derive(Default)]
    struct Keeper {
        batches: AtomicUsize,
    }

    impl Feature for Keeper {
        fn notes(&self, session: &Session) -> Vec<Note> {
            if session.latest_note("rules").is_some() {
                return Vec::new();
            }
            vec![Note { tag: "rules".into(), text: "House rules.".into() }]
        }

        fn after_turn(&self, reply: &Message) {
            if reply.tool_uses().next().is_some() {
                self.batches.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn state_to_save(&self) -> Vec<(String, Value)> {
            vec![("batches".into(), json!(self.batches.load(Ordering::SeqCst)))]
        }

        fn summary_sections(&self) -> Vec<String> {
            vec![format!("## Batches\n{}", self.batches.load(Ordering::SeqCst))]
        }

        fn on_run_end(&self, reply: &str) -> Option<String> {
            reply.contains("oops").then(|| "The reply said oops.".to_string())
        }
    }

    #[tokio::test]
    async fn notes_come_before_the_users_message_and_state_is_saved_as_it_changes() {
        let steps = vec![call(&[("a", "echo", json!({ "text": "one" }))]), say("Done, oops."), say("Again.")];
        let mut rig = rig(steps, vec![Arc::new(Echo("echo", Loading::Always))]);
        rig.agent = rig.agent.with_feature(Box::new(Keeper::default()));
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));

        let first = &rig.provider.requests()[0].messages;
        assert_eq!(first[0].text(), "House rules.");
        assert_eq!(first[1].text(), "go", "the user's message is last");
        let saved: Vec<Value> = rig
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::State { key, value } if key == "batches" => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(saved, [json!(0), json!(1)], "written when it changes, not at every step");
        assert!(rig.saw(|event| *event == Event::Notice("The reply said oops.".into())));

        // The note is owed once: the next run does not repeat it.
        assert_eq!(rig.run("more", &Control::new()).await, Ok(Outcome::Done));
        let texts: Vec<String> = rig.provider.requests()[2].messages.iter().map(Message::text).collect();
        assert_eq!(texts.iter().filter(|text| *text == "House rules.").count(), 1);
        assert_eq!(Session::open(rig.agent.session().path()).unwrap().state("batches"), Some(&json!(1)));
    }

    #[tokio::test]
    async fn a_compaction_keeps_feature_sections_and_the_note_is_sent_again() {
        let mut rig = rig(vec![say("First."), say("Summary body."), say("Second.")], vec![]);
        rig.agent = rig.agent.with_feature(Box::new(Keeper::default()));
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        rig.agent.compact(None, true, &Control::new(), &|_| {}).await.unwrap();

        // The summariser is not shown the app's note, and the summary never quotes it.
        let summarised = rig.provider.requests()[1].messages[0].text();
        assert!(!summarised.contains("House rules."));
        let summary = rig.agent.session().messages()[0].text();
        assert!(summary.contains("## Batches\n0"));
        assert!(summary.contains("<user-message>\ngo\n</user-message>") && !summary.contains("House rules."));

        assert_eq!(rig.run("next", &Control::new()).await, Ok(Outcome::Done));
        let texts: Vec<String> = rig.provider.requests()[2].messages.iter().map(Message::text).collect();
        assert_eq!(texts[1..], ["House rules.", "next"]);
    }

    #[tokio::test]
    async fn a_planning_run_refuses_changes_asks_and_ends_with_a_plan_for_review() {
        use crate::{
            kit::Kit,
            plan::{AgentMode, Contract, PLAN_STATE, PlanState, prompt as plan_prompt},
        };

        let alone = Arc::new(Alone::default());
        let build = |session: Session, provider: &ScriptedProvider, mode: AgentMode, alone: Arc<Alone>| {
            let kit = Kit::new(&session, Some(mode), false, None);
            let mut tools: Vec<Arc<dyn Tool>> = vec![alone, Arc::new(Echo("echo", Loading::Always))];
            tools.extend(kit.tools());
            let settings = Settings {
                model: "m".into(),
                effort: None,
                max_output_tokens: None,
                context_window: None,
                accepts_images: true,
                ultra: false,
                ultra_prompt: None,
                compaction_prompt: None,
                commit_attribution: false,
            };
            let agent = Agent::new(Arc::new(provider.clone()), Arc::new(Registry::new(tools).unwrap()), session, &Profile::code(), settings);
            kit.attach(agent)
        };
        let question = json!({ "questions": [{ "id": "scope", "header": "Scope", "question": "How far?", "options": [
            { "label": "Small", "description": "Just the bug." }, { "label": "Large", "description": "The module." }
        ] }] });
        let provider = ScriptedProvider::new(vec![
            call(&[("a", "alone", json!({})), ("b", "echo", json!({ "text": "read" }))]),
            call(&[("c", crate::ask::ASK_USER_QUESTION, question)]),
            call(&[("d", crate::plan::PLAN_MODE_COMPLETE, json!({ "plan": "# Fix\n\n1. Do it" }))]),
            say("Implemented."),
        ]);
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let mut agent = build(Session::create(&path, folder.path()).unwrap(), &provider, AgentMode::Plan, alone.clone());

        let control = Control::with_user();
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let (answerer, events) = (control.clone(), seen.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let asked = events.lock().unwrap().iter().find_map(|event| match event {
                    Event::Question { id, wrap_up, .. } => Some((id.clone(), *wrap_up)),
                    _ => None,
                });
                if let Some((id, wrap_up)) = asked {
                    assert!(!wrap_up, "only Ultra Plan offers to wrap up");
                    let picked = crate::ask::QuestionAnswer { question_id: "scope".into(), selected: vec!["Small".into()], custom: None };
                    answerer.answer(&id, Answer::Answered(vec![picked]));
                    return;
                }
            }
        });
        let sink = seen.clone();
        let outcome = tokio::time::timeout(Duration::from_secs(5), agent.run("Plan the fix", &control, &move |event| sink.lock().unwrap().push(event)))
            .await
            .unwrap();
        assert_eq!(outcome, Ok(Outcome::Done));

        // The contract came first, the request last; the same tools as any other mode.
        let requests = provider.requests();
        assert_eq!(requests.len(), 3, "submitting the plan ended the run");
        assert_eq!(requests[0].messages[0].text(), plan_prompt::contract(Contract::Plan));
        assert_eq!(requests[0].messages[1].text(), "Plan the fix");
        let results: Vec<(String, bool)> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::ToolResult { content, is_error, .. } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect();
        assert_eq!(results[0], ("Plan mode blocks the mutating tool 'alone'.".to_string(), true));
        assert_eq!(results[1], ("read".to_string(), false));
        assert_eq!(results[2].0, "The user answered your questions:\n\n**Scope** — How far?\n- Small");
        assert_eq!(results[3], ("**Proposed Plan**\n\n# Fix\n\n1. Do it".to_string(), false));
        assert_eq!(alone.most.load(Ordering::SeqCst), 0);
        let waiting = PlanState { mode: AgentMode::Plan, plan: Some("# Fix\n\n1. Do it".into()) };
        assert!(seen.lock().unwrap().contains(&Event::State { key: PLAN_STATE.into(), value: waiting.to_value() }));
        drop(agent);

        // Approved: the next run is in Build. The model is told planning is over, the proposal
        // is consumed, and changes run again.
        let mut agent = build(Session::open(&path).unwrap(), &provider, AgentMode::Build, alone.clone());
        assert_eq!(PlanState::from_value(agent.session().state(PLAN_STATE).unwrap()), Some(waiting));
        assert_eq!(agent.run("Implement the plan.", &Control::new(), &|_| {}).await, Ok(Outcome::Done));
        let sent = &provider.requests()[3].messages;
        assert_calls_answered(sent);
        let tail: Vec<String> = sent[sent.len() - 2..].iter().map(Message::text).collect();
        assert_eq!(tail, [plan_prompt::contract(Contract::Normal), "Implement the plan.".to_string()]);
        assert_eq!(agent.session().state(PLAN_STATE), Some(&json!({ "mode": "build" })));
        assert_eq!(requests[0].tools, provider.requests()[3].tools, "the tool list is the same in every mode");
        assert_eq!(requests[0].system, provider.requests()[3].system);
    }

    #[tokio::test]
    async fn a_reply_cut_off_mid_call_runs_nothing() {
        let Step::Reply(mut cut) = call(&[("a", "echo", json!({ "text": "half" }))]) else { unreachable!() };
        cut.stop = StopReason::MaxTokens;
        let mut rig = rig(vec![Step::Reply(cut), say("Done.")], vec![Arc::new(Echo("echo", Loading::Always))]);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let results = rig.results();
        assert!(results[0].1 && results[0].0.contains("length limit"));
    }

    /// Leaves a background task's news in the chat's inbox, as if a task just ended.
    struct Tell;

    #[async_trait]
    impl Tool for Tell {
        fn spec(&self) -> ToolSpec {
            spec("tell", Loading::Always, false)
        }

        async fn run(&self, _: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
            context.tasks.tell(Note { tag: "task:t1:end".into(), text: "Task t1 finished.".into() });
            Ok("told".to_string().into())
        }
    }

    fn with_tasks(rig: &mut Rig, tasks: &crate::tasks::Tasks) {
        let feature = crate::tasks::TasksFeature::new(rig.agent.session(), tasks.clone());
        let agent = std::mem::replace(&mut rig.agent, rig_with(ScriptedProvider::new(Vec::new()), Vec::new(), None).agent);
        rig.agent = agent.with_tasks(tasks.clone()).with_feature(Box::new(feature));
    }

    fn noted(messages: &[Message], text: &str) -> usize {
        messages.iter().filter(|message| message.text() == text).count()
    }

    #[tokio::test]
    async fn task_news_reaches_the_model_once_before_its_next_request() {
        let tasks = crate::tasks::Tasks::scratch();
        let mut rig = rig(vec![call(&[("a", "tell", json!({}))]), say("Seen it.")], vec![Arc::new(Tell)]);
        with_tasks(&mut rig, &tasks);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let second = &rig.provider.requests()[1].messages;
        assert_eq!(noted(second, "Task t1 finished."), 1);
        assert_eq!(second.last().unwrap().text(), "Task t1 finished.", "news follows the tool results");
        assert_eq!(noted(rig.agent.session().messages(), "Task t1 finished."), 1);
        assert!(rig.saw(|event| matches!(event, Event::Noted { tag, .. } if tag == "task:t1:end")));
        assert!(!tasks.has_news());
    }

    /// Brings news while the model writes a reply that would end the run.
    struct LateNews(crate::tasks::Tasks, AtomicBool);

    impl Feature for LateNews {
        fn after_turn(&self, _: &Message) {
            if !self.1.swap(true, Ordering::SeqCst) {
                self.0.tell(Note { tag: "task:t2:end".into(), text: "Task t2 finished.".into() });
            }
        }
    }

    #[tokio::test]
    async fn news_arriving_as_the_model_finishes_keeps_the_run_going() {
        let tasks = crate::tasks::Tasks::scratch();
        let mut rig = rig(vec![say("All done."), say("t2 is done too.")], Vec::new());
        with_tasks(&mut rig, &tasks);
        rig.agent = std::mem::replace(&mut rig.agent, rig_with(ScriptedProvider::new(Vec::new()), Vec::new(), None).agent)
            .with_feature(Box::new(LateNews(tasks.clone(), AtomicBool::new(false))));
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let requests = rig.provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].messages.last().unwrap().text(), "Task t2 finished.");
    }

    #[tokio::test]
    async fn waking_runs_on_news_alone_and_sends_nothing_without_it() {
        let tasks = crate::tasks::Tasks::scratch();
        let mut rig = rig(vec![say("Started it."), say("The build passed.")], Vec::new());
        with_tasks(&mut rig, &tasks);
        assert_eq!(rig.run("build it", &Control::new()).await, Ok(Outcome::Done));
        let events = rig.events.clone();
        let sink = move |event| events.lock().unwrap().push(event);
        assert_eq!(rig.agent.wake(&Control::new(), &sink).await, Ok(Outcome::Done));
        assert_eq!(rig.provider.requests().len(), 1, "no news, no request");

        tasks.tell(Note { tag: "task:t3:end".into(), text: "Task t3 finished: exit code 0.".into() });
        assert_eq!(rig.agent.wake(&Control::new(), &sink).await, Ok(Outcome::Done));
        let requests = rig.provider.requests();
        assert_eq!(requests.len(), 2);
        let last = requests[1].messages.last().unwrap();
        assert_eq!(last.text(), "Task t3 finished: exit code 0.");
        assert_eq!(rig.agent.session().messages().last().unwrap().text(), "The build passed.");
    }

    #[tokio::test]
    async fn a_task_left_running_by_a_quit_is_reported_on_the_next_run() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        {
            let mut session = Session::create(&path, folder.path()).unwrap();
            let info = crate::tasks::TaskInfo {
                id: "gone1234".into(),
                command: "npm run dev".into(),
                description: String::new(),
                kind: crate::tasks::Kind::Shell,
                started_at: 1,
                ended_at: None,
                status: crate::tasks::Status::Running,
                background: Some(crate::tasks::Why::Asked),
                log: folder.path().join("gone1234.log"),
                bytes: 0,
                last_line: String::new(),
            };
            session.set_state(crate::tasks::TASKS_STATE, json!([info])).unwrap();
        }
        let session = Session::open(&path).unwrap();
        let tasks = crate::tasks::Tasks::scratch();
        let feature = crate::tasks::TasksFeature::new(&session, tasks.clone());
        let notes = feature.notes(&session);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("gone1234") && notes[0].text.contains("stopped when WackCode quit"), "{}", notes[0].text);
        assert!(feature.notes(&session).is_empty(), "told once");
        assert_eq!(feature.state_to_save()[0].1, json!([]));
    }

    #[tokio::test]
    async fn commit_attribution_follows_each_runs_model_and_setting() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("chat.jsonl");
        let provider = ScriptedProvider::new([say("1"), say("2"), say("3"), say("4")]);
        let run_once = |session: Session, model: &str, attribution: bool| {
            let settings = Settings {
                model: model.into(),
                effort: None,
                max_output_tokens: None,
                context_window: None,
                accepts_images: true,
                ultra: false,
                ultra_prompt: None,
                compaction_prompt: None,
                commit_attribution: attribution,
            };
            let mut agent = Agent::new(
                Arc::new(provider.clone()),
                Arc::new(Registry::new(Vec::new()).unwrap()),
                session,
                &Profile::code(),
                settings,
            );
            async move { agent.run("Commit it.", &Control::new(), &|_| {}).await.unwrap() }
        };

        run_once(Session::create(&path, folder.path()).unwrap(), "zai-org/GLM-5.3-Flash", true).await;
        run_once(Session::open(&path).unwrap(), "Second <model>", true).await;
        run_once(Session::open(&path).unwrap(), "Second <model>", false).await;
        run_once(Session::open(&path).unwrap(), "Second <model>", true).await;

        let guide_of = |index: usize| provider.requests()[index].system[1].text.clone();
        assert!(guide_of(0).contains(&prompt::co_author_trailer("GLM-5.3-Flash")));
        assert!(guide_of(1).contains(&prompt::co_author_trailer("Second model")));
        assert!(!guide_of(2).contains("Co-authored-by"));
        assert!(!guide_of(2).contains("Commit attribution"));
        assert!(guide_of(3).contains(&prompt::co_author_trailer("Second model")));
    }

    /// Returns a picture beside its text, as a browser screenshot will.
    struct Shot;

    #[async_trait]
    impl Tool for Shot {
        fn spec(&self) -> ToolSpec {
            spec("shot", Loading::Always, true)
        }

        async fn run(&self, _: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
            let mut png = Vec::new();
            image::DynamicImage::new_rgb8(1, 1).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            let image = wack_llm::Image::new("image/png", crate::images::view::encode(&png));
            Ok(ToolOutput::with_images("The page.", vec![image]))
        }
    }

    fn sent_result(request: &Request) -> (String, usize) {
        request
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|block| match block {
                Block::ToolResult { content, images, .. } => Some((content.clone(), images.len())),
                _ => None,
            })
            .unwrap()
    }

    #[tokio::test]
    async fn a_tools_picture_reaches_the_model_and_survives_reopening_the_session() {
        let steps = vec![call(&[("s1", "shot", json!({}))]), say("I see it.")];
        let mut rig = rig(steps, vec![Arc::new(Shot)]);
        assert_eq!(rig.run("look", &Control::new()).await, Ok(Outcome::Done));
        assert_eq!(sent_result(&rig.provider.requests()[1]), ("The page.".to_string(), 1));
        // The transcript gets the text; the picture travels only in the conversation.
        assert_eq!(rig.results(), [("The page.".to_string(), false)]);

        let reopened = Session::open(rig.agent.session().path()).unwrap();
        assert_eq!(reopened.messages(), rig.agent.session().messages());
        assert!(matches!(&reopened.messages()[2].content[0], Block::ToolResult { images, .. } if images.len() == 1));
    }

    #[tokio::test]
    async fn the_users_pictures_follow_their_words_saved_for_the_chat() {
        let mut rig = rig(vec![say("A duck.")], Vec::new());
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let attached = Image::new("image/png", crate::images::view::encode(&png)).labelled("duck.png");
        let outcome = rig.agent.run_with("What is this?", vec![attached], None, &Control::new(), &|_| {}).await;
        assert_eq!(outcome, Ok(Outcome::Done));
        let sent = &rig.provider.requests()[0].messages;
        let Some(Block::Image { label, id, .. }) = sent.last().unwrap().content.get(1) else { panic!("no picture: {sent:?}") };
        assert_eq!(label.as_deref(), Some("duck.png"));
        let store = crate::images::Images::for_session(rig.agent.session().path());
        assert_eq!(std::fs::read(store.find(id.as_deref().unwrap()).unwrap()).unwrap(), png);
        assert_eq!(sent.last().unwrap().text(), "What is this?");
    }

    #[tokio::test]
    async fn a_model_without_images_is_told_the_picture_was_left_out_but_the_session_keeps_it() {
        let steps = vec![call(&[("s1", "shot", json!({}))]), say("Noted.")];
        let mut rig = rig(steps, vec![Arc::new(Shot)]);
        rig.agent.settings.accepts_images = false;
        assert_eq!(rig.run("look", &Control::new()).await, Ok(Outcome::Done));
        let (text, images) = sent_result(&rig.provider.requests()[1]);
        assert_eq!(images, 0);
        assert_eq!(text, "The page.\n[1 image left out: this model does not accept images.]");
        // Saved as it came, so a model that reads pictures sees it after a switch.
        assert!(matches!(&rig.agent.session().messages()[2].content[0], Block::ToolResult { images, .. } if images.len() == 1));
    }
}
