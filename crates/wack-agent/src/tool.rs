//! What a tool is, as the agent loop sees it.

use std::path::PathBuf;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use wack_llm::{Image, Message, SystemPart};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{Control, Event, EventSink, Settings},
    registry::Registry,
    agents::Agents,
    ask::{Answer, Question},
    images::Images,
    tasks::Tasks,
};

/// Whether the model sees a tool from the first request or has to search for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loading {
    Always,
    /// Kept out of the request until the model finds it, so a large tool set costs no context.
    Deferred,
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    /// The full description the model reads once the tool is loaded.
    pub description: String,
    pub input_schema: Value,
    pub loading: Loading,
    /// True when calls can run side by side with other safe calls: the tool changes nothing.
    /// It is also Plan mode's read-only test: a tool that sets this may run while planning.
    pub parallel_safe: bool,
    /// One line for the prompt's tool list. For a deferred tool it is all the model knows of it
    /// before searching, so it should say when to reach for the tool.
    pub hint: String,
    /// Rules for when and how to use the tool, listed in the prompt under the tool list. They
    /// are part of the system prompt, so they must never change during a chat.
    pub guidelines: Vec<String>,
    /// A call that succeeds ends the run: its result is saved and the model is not asked again.
    pub ends_run: bool,
    /// Calls run side by side with their neighbours that are also `together` or `parallel_safe`,
    /// though the tool may change things: each call does separate work the model asked for at
    /// once (sub-agents). Unlike `parallel_safe`, it says nothing to Plan mode.
    pub together: bool,
}

/// What a running tool is given.
pub struct ToolContext<'a> {
    /// The id of the call being answered, which the transcript knows it by.
    pub call_id: String,
    /// The chat's working directory. Relative paths resolve against it.
    pub cwd: PathBuf,
    /// Fires when the user stops the run. A tool that starts anything long-lived must watch it.
    pub cancel: CancellationToken,
    /// The way to the user, for a tool that has to ask before it can finish.
    pub user: User<'a>,
    /// The chat's shell tasks, which outlive the run.
    pub tasks: Tasks,
    /// The chat's sub-agents, so a wait can include the ones in the background.
    pub agents: Agents,
    /// The run this call came from, as the model sees it.
    pub seen: Seen<'a>,
    /// The sub-agent making the call, or `None` for the chat's own agent.
    pub caller: Option<String>,
    /// Where the chat keeps its pictures' originals.
    pub images: Images,
}

/// What the model sent to make this call: its instructions, its tools and the conversation,
/// ending with the reply that made the call. A sub-agent that starts from a copy of the
/// conversation sends the same, so the provider's prompt cache holds.
#[derive(Clone, Copy, Default)]
pub struct Seen<'a> {
    pub system: &'a [SystemPart],
    pub messages: &'a [Message],
    pub registry: Option<&'a Arc<Registry>>,
    /// Deferred tools the conversation has loaded.
    pub loaded: &'a [String],
    pub settings: Option<&'a Settings>,
}

impl ToolContext<'static> {
    /// A context with nobody on the other end, for running a tool outside a run.
    pub fn detached(cwd: impl Into<PathBuf>) -> Self {
        Self { call_id: String::new(), cwd: cwd.into(), cancel: CancellationToken::new(), user: User::nobody(), tasks: Tasks::scratch(), agents: Agents::new(), seen: Seen::default(), caller: None, images: Images::none() }
    }
}

/// A running tool's line to the user.
pub struct User<'a> {
    control: Control,
    events: EventSink<'a>,
}

impl<'a> User<'a> {
    pub(crate) fn new(control: Control, events: EventSink<'a>) -> Self {
        Self { control, events }
    }

    pub fn nobody() -> User<'static> {
        User { control: Control::new(), events: &|_| {} }
    }

    /// True when someone can answer. A piped command-line run has nobody.
    pub fn present(&self) -> bool {
        self.control.interactive()
    }

    /// Shows the user questions and waits for the answer. Stopping the run drops the wait, and
    /// the question with it.
    pub async fn ask(&self, questions: Vec<Question>, wrap_up: bool) -> Answer {
        let (id, answer) = self.control.open_question();
        (self.events)(Event::Question { id, questions, wrap_up });
        answer.await
    }
}

/// What a tool that succeeded hands back: text, and pictures for a tool that has them (a
/// screenshot). Most tools return text alone, through `From<String>`.
///
/// Pictures reach a model only when it accepts them; otherwise the agent drops them and says so
/// in the text (`images::prepare`), so `text` must make sense on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolOutput {
    pub text: String,
    pub images: Vec<Image>,
}

impl ToolOutput {
    pub fn with_images(text: impl Into<String>, images: Vec<Image>) -> Self {
        Self { text: text.into(), images }
    }
}

impl From<String> for ToolOutput {
    fn from(text: String) -> Self {
        Self { text, images: Vec::new() }
    }
}

impl From<&str> for ToolOutput {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// Runs the tool. Both outcomes go back to the model; an error is text alone, marked as one.
    /// The future may be dropped at any await point when the user stops the run, so anything that
    /// must be cleaned up (a child process) needs a guard that does it on drop.
    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String>;
}
