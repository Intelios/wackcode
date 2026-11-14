//! The turn loop: send the conversation, run the tools the model calls, repeat until it stops.
//!
//! Invariants the loop keeps, which the session and every provider rely on:
//! - Every tool call gets exactly one result, in the message straight after the call, whatever
//!   happens: a refusal, a failure, or the user stopping the run.
//! - The session is written before anything acts on it, so a crash never loses a step that ran.
//! - The system prompt and the start of the tool list never change during a run (prompt caching).
//!
//! Nothing here asks the user for approval before a tool runs: that is out of scope by design.
//! `Feature::on_tool_call` exists for modes that forbid tools outright (a plan mode, a confined
//! chat), not for prompts.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use futures::future::join_all;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use wack_llm::{
    Block, Delta, Effort, ErrorKind, Message, Provider, Request, RetryNotice, Role, StopReason, SystemPart, Usage,
    stream_with_retry,
};

use crate::{
    compaction::{self, Summary},
    prompt,
    registry::{Registry, TOOL_SEARCH},
    session::Session,
    tool::{Loading, Tool, ToolContext},
};

/// Compaction starts when the conversation fills this share of the model's context window.
const COMPACT_AT_PERCENT: u64 = 80;
/// The conversation text sent to the summariser when the window's size is unknown.
const DEFAULT_SUMMARY_BUDGET_CHARS: usize = 300_000;
const STOPPED: &str = "Stopped by the user before this tool finished.";

/// What kind of agent this is. Code is the only profile so far; a chat-only agent and sub-agent
/// roles are further profiles, not branches inside the loop.
#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    /// The core of the system prompt.
    pub prompt: String,
}

impl Profile {
    pub fn code() -> Self {
        Self { name: "code".to_string(), prompt: prompt::CODE_PROMPT.to_string() }
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
}

pub enum Verdict {
    Allow,
    /// The tool does not run, and the model is told why.
    Deny(String),
}

/// A compiled-in feature's hooks into the loop. All default to doing nothing.
pub trait Feature: Send + Sync {
    /// Called with each request just before it is sent.
    fn before_request(&self, _request: &mut Request) {}

    /// Called before each tool runs.
    fn on_tool_call(&self, _name: &str, _input: &Value) -> Verdict {
        Verdict::Allow
    }

    /// Called with each reply once it is saved.
    fn after_turn(&self, _reply: &Message) {}
}

/// How the user reaches into a run from outside: to stop it, or to send a message while it works.
#[derive(Clone, Default)]
pub struct Control {
    cancel: CancellationToken,
    steering: Arc<Mutex<VecDeque<String>>>,
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stops the run. Running tools are cut off and the reply being written is dropped.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn stopped(&self) -> bool {
        self.cancel.is_cancelled()
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
    ToolResult { id: String, name: String, content: String, is_error: bool },
    /// One reply from the model is complete and saved.
    Reply { usage: Option<Usage> },
    Retrying(RetryNotice),
    /// A message the user sent mid-run was handed to the model.
    Steered(String),
    Compacting,
    Compacted { usage: Option<Usage> },
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
}

enum Planned {
    Run { tool: Arc<dyn Tool>, parallel_safe: bool },
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
        let specs: Vec<_> = registry.specs().collect();
        let system = prompt::system(&profile.prompt, &cwd, &prompt::today(), &specs);
        Self { provider, registry, session, settings, features: Vec::new(), system, cwd }
    }

    pub fn with_feature(mut self, feature: Box<dyn Feature>) -> Self {
        self.features.push(feature);
        self
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Runs one user message to completion: until the model stops calling tools, or the user
    /// stops it. An `Err` is a failure the user has to see; the session stays valid either way.
    pub async fn run(&mut self, prompt: &str, control: &Control, events: EventSink<'_>) -> Result<Outcome, String> {
        self.session.push(Message::user(prompt), None)?;
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
            let result = tokio::select! {
                biased;
                () = control.cancel.cancelled() => None,
                result = stream_with_retry(self.provider.as_ref(), &request, &sink, &retrying) => Some(result),
            };
            let response = match result {
                None => {
                    // The user saw this text appear, so it stays in the conversation.
                    let text = written.into_inner().unwrap();
                    if !text.trim().is_empty() {
                        self.session.push(Message { role: Role::Assistant, content: vec![Block::Text { text }] }, None)?;
                    }
                    return Ok(Outcome::Stopped);
                }
                Some(Err(error)) if error.kind == ErrorKind::ContextOverflow && !compacted_for_overflow => {
                    compacted_for_overflow = true;
                    match self.compact(None, true, control, events).await {
                        Ok(()) => continue,
                        Err(_) if control.stopped() => return Ok(Outcome::Stopped),
                        Err(reason) => {
                            return Err(format!("The conversation is too long for this model, and compacting it failed: {reason}"));
                        }
                    }
                }
                Some(Err(error)) => return Err(error.message),
                Some(Ok(response)) => response,
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
                return Ok(match response.stop {
                    StopReason::MaxTokens => Outcome::CutOff,
                    StopReason::Refusal => Outcome::Refused,
                    _ => Outcome::Done,
                });
            }

            let cut_off = response.stop == StopReason::MaxTokens;
            let mut content = self.run_tools(&calls, cut_off, control, events).await?;
            if control.stopped() {
                self.session.push(Message { role: Role::User, content }, None)?;
                return Ok(Outcome::Stopped);
            }
            if let Some(text) = control.take_steering() {
                // Results come first: a provider reads the message's start as the calls' answers.
                content.push(Block::Text { text: text.clone() });
                events(Event::Steered(text));
            }
            self.session.push(Message { role: Role::User, content }, None)?;
        }
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
        for feature in &self.features {
            feature.before_request(&mut request);
        }
        request
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
            } else if let Some(reason) = self.denied(name, input) {
                Planned::Settled(Err(reason))
            } else if name == TOOL_SEARCH {
                Planned::Settled(self.tool_search(input))
            } else if let Some((spec, tool)) = self.registry.get(name) {
                let (deferred, parallel_safe, tool) = (spec.loading == Loading::Deferred, spec.parallel_safe, tool.clone());
                if deferred {
                    // Also on the provider's own search path, so a chat that later moves to a
                    // provider without one still offers the tools it was using.
                    self.session.load_tool(name)?;
                }
                Planned::Run { tool, parallel_safe }
            } else {
                Planned::Settled(Err(format!("There is no tool named {name}.")))
            };
            events(Event::ToolCall { id: id.clone(), name: name.clone(), input: input.clone() });
            plans.push(plan);
        }

        let context = ToolContext { cwd: self.cwd.clone(), cancel: control.cancel.clone() };
        let run_one = |index: usize| {
            let (id, name, input) = &calls[index];
            let plan = &plans[index];
            let context = &context;
            async move {
                let result = match plan {
                    Planned::Settled(result) => result.clone(),
                    Planned::Run { .. } if control.stopped() => Err(STOPPED.to_string()),
                    Planned::Run { tool, .. } => {
                        tokio::select! {
                            biased;
                            () = control.cancel.cancelled() => Err(STOPPED.to_string()),
                            result = tool.run(input.clone(), context) => result,
                        }
                    }
                };
                let (content, is_error) = match result {
                    Ok(content) => (content, false),
                    Err(content) => (content, true),
                };
                events(Event::ToolResult { id: id.clone(), name: name.clone(), content: content.clone(), is_error });
                Block::ToolResult { tool_use_id: id.clone(), content, is_error }
            }
        };

        // Calls run in the model's order. Neighbours that are all safe to run side by side do;
        // anything that changes state runs alone, after what came before it has finished.
        let safe = |index: usize| matches!(plans[index], Planned::Run { parallel_safe: true, .. });
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

    fn denied(&self, name: &str, input: &Value) -> Option<String> {
        self.features.iter().find_map(|feature| match feature.on_tool_call(name, input) {
            Verdict::Allow => None,
            Verdict::Deny(reason) => Some(reason),
        })
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
        let request =
            compaction::request(&self.settings.model, self.session.summary(), &messages[start..cut], focus, budget);
        let retrying = |notice: RetryNotice| events(Event::Retrying(notice));
        let response = tokio::select! {
            biased;
            () = control.cancel.cancelled() => return Err("Compaction was stopped.".to_string()),
            result = stream_with_retry(self.provider.as_ref(), &request, &|_| {}, &retrying) => {
                result.map_err(|error| error.message)?
            }
        };
        let body = response.message.text();
        if body.trim().is_empty() {
            return Err("The model returned an empty summary.".to_string());
        }
        let summary = Summary::build(self.session.summary(), &messages[start..cut], body);
        let keep = messages.len() - cut;
        self.session.compact(summary, keep)?;
        events(Event::Compacted { usage: response.usage });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::{Barrier, Notify};
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
        }
    }

    /// Returns its `text` input.
    struct Echo(&'static str, Loading);

    #[async_trait]
    impl Tool for Echo {
        fn spec(&self) -> ToolSpec {
            spec(self.0, self.1, true)
        }

        async fn run(&self, input: Value, _: &ToolContext) -> Result<String, String> {
            Ok(input["text"].as_str().unwrap_or("ok").to_string())
        }
    }

    /// Finishes only once two calls are inside it at the same moment.
    struct Together(Barrier);

    #[async_trait]
    impl Tool for Together {
        fn spec(&self) -> ToolSpec {
            spec("together", Loading::Always, true)
        }

        async fn run(&self, _: Value, _: &ToolContext) -> Result<String, String> {
            self.0.wait().await;
            Ok("met".to_string())
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

        async fn run(&self, _: Value, _: &ToolContext) -> Result<String, String> {
            let inside = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(inside, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(5)).await;
            self.inside.fetch_sub(1, Ordering::SeqCst);
            Ok("ran".to_string())
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

        async fn run(&self, _: Value, _: &ToolContext) -> Result<String, String> {
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
        let folder = tempfile::tempdir().unwrap();
        let session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let settings = Settings { model: "test-model".into(), effort: None, max_output_tokens: None, context_window };
        let agent = Agent::new(
            Arc::new(provider.clone()),
            Arc::new(Registry::new(tools).unwrap()),
            session,
            &Profile::code(),
            settings,
        );
        Rig { agent, provider, events: Arc::default(), _folder: folder }
    }

    fn rig(steps: Vec<Step>, tools: Vec<Arc<dyn Tool>>) -> Rig {
        rig_with(ScriptedProvider::new(steps), tools, None)
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
        let settings = Settings { model: "test-model".into(), effort: None, max_output_tokens: None, context_window: None };
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

        fn on_tool_call(&self, name: &str, _: &Value) -> Verdict {
            if name == "alone" { Verdict::Deny("This mode does not allow changes.".into()) } else { Verdict::Allow }
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

    #[tokio::test]
    async fn a_reply_cut_off_mid_call_runs_nothing() {
        let Step::Reply(mut cut) = call(&[("a", "echo", json!({ "text": "half" }))]) else { unreachable!() };
        cut.stop = StopReason::MaxTokens;
        let mut rig = rig(vec![Step::Reply(cut), say("Done.")], vec![Arc::new(Echo("echo", Loading::Always))]);
        assert_eq!(rig.run("go", &Control::new()).await, Ok(Outcome::Done));
        let results = rig.results();
        assert!(results[0].1 && results[0].0.contains("length limit"));
    }
}
