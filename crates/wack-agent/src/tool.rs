//! What a tool is, as the agent loop sees it.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{Control, Event, EventSink},
    agents::Agents,
    ask::{Answer, Question},
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
}

impl ToolContext<'static> {
    /// A context with nobody on the other end, for running a tool outside a run.
    pub fn detached(cwd: impl Into<PathBuf>) -> Self {
        Self { call_id: String::new(), cwd: cwd.into(), cancel: CancellationToken::new(), user: User::nobody(), tasks: Tasks::scratch(), agents: Agents::new() }
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

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// Runs the tool. Both outcomes go back to the model as text; an error is marked as one. The
    /// future may be dropped at any await point when the user stops the run, so anything that
    /// must be cleaned up (a child process) needs a guard that does it on drop.
    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<String, String>;
}
