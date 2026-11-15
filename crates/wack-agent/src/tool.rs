//! What a tool is, as the agent loop sees it.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{Control, Event, EventSink},
    ask::{Answer, Question},
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
}

/// What a running tool is given.
pub struct ToolContext<'a> {
    /// The chat's working directory. Relative paths resolve against it.
    pub cwd: PathBuf,
    /// Fires when the user stops the run. A tool that starts anything long-lived must watch it.
    pub cancel: CancellationToken,
    /// The way to the user, for a tool that has to ask before it can finish.
    pub user: User<'a>,
}

impl ToolContext<'static> {
    /// A context with nobody on the other end, for running a tool outside a run.
    pub fn detached(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into(), cancel: CancellationToken::new(), user: User::nobody() }
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
