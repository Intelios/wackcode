//! What a tool is, as the agent loop sees it.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

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
    pub parallel_safe: bool,
    /// One line for the prompt's tool list. For a deferred tool it is all the model knows of it
    /// before searching, so it should say when to reach for the tool.
    pub hint: String,
}

/// What a running tool is given.
pub struct ToolContext {
    /// The chat's working directory. Relative paths resolve against it.
    pub cwd: PathBuf,
    /// Fires when the user stops the run. A tool that starts anything long-lived must watch it.
    pub cancel: CancellationToken,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// Runs the tool. Both outcomes go back to the model as text; an error is marked as one. The
    /// future may be dropped at any await point when the user stops the run, so anything that
    /// must be cleaned up (a child process) needs a guard that does it on drop.
    async fn run(&self, input: Value, context: &ToolContext) -> Result<String, String>;
}
