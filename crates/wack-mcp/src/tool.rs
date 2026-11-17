//! MCP tools as the agent sees them, and how a run gets them.
//!
//! Invariants:
//! - A run's tool list is fixed when it starts ([`assemble`]): servers with a catalogue (live or
//!   cached) are offered at once; only a server with none holds the run up, and only until it
//!   connects or fails, or the user stops.
//! - Every MCP tool is deferred. Its hint (one short line, in the system prompt) and its order
//!   (server order, then tool name) are stable, so the prompt cache holds between runs.
//! - A call waits only for its own server, then checks the live tool against what the run
//!   offered: it must still exist, keep its input schema, and keep its read-only hint if it was
//!   offered as read-only (Plan mode let it through on that hint). Otherwise it is refused and the
//!   next message offers the new declaration.
//! - What went wrong with a server reaches each chat using it once, as a note tagged
//!   `mcp:<server>:<stamp>`: the model reads it and the transcript shows it as a notice.

use std::{collections::HashSet, path::Path, sync::Arc};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wack_agent::{Feature, Loading, Note, Session, Tool, ToolContext, ToolSpec};

use crate::{
    convert::{parameters, result_text, tool_name},
    hub::{Hub, Server},
    spec::{ServerSpec, ToolDecl},
};

/// The tag every MCP note starts with.
pub const NOTE_PREFIX: &str = "mcp:";
const MAX_DESCRIPTION_CHARS: usize = 2_000;
const MAX_HINT_CHARS: usize = 120;

/// Told the names of the servers a run is waiting for, and an empty list when it stops waiting.
pub type Starting = Arc<dyn Fn(Vec<String>) + Send + Sync>;

/// What MCP adds to one run.
#[derive(Default)]
pub struct Assembly {
    pub tools: Vec<Arc<dyn Tool>>,
    /// Tells the model (and through the transcript, the user) when a server fails or stops.
    pub feature: Option<McpFeature>,
}

/// Prepares the run's servers in `cwd` and builds their tools, waiting only for servers with no
/// catalogue at all.
pub async fn assemble(hub: &Hub, specs: &[ServerSpec], cwd: &Path, cancel: &CancellationToken, starting: Starting) -> Assembly {
    if specs.is_empty() {
        return Assembly::default();
    }
    let servers = hub.prepare(specs, cwd);
    let waiting: Vec<&Server> = servers.iter().filter(|server| server.connecting() && server.catalogue().is_none()).collect();
    if !waiting.is_empty() && !cancel.is_cancelled() {
        starting(waiting.iter().map(|server| server.name()).collect());
        futures::future::join_all(waiting.iter().map(|server| server.ready(cancel))).await;
        starting(Vec::new());
    }
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    let mut names = HashSet::new();
    for server in &servers {
        let Some(catalogue) = server.catalogue() else { continue };
        let spec = server.spec();
        let mut offered: Vec<ToolDecl> = catalogue.into_iter().filter(|tool| !spec.tool_disabled(&tool.name)).collect();
        offered.sort_by(|left, right| left.name.cmp(&right.name));
        for decl in offered {
            let tool = McpTool::new(server.clone(), &spec, decl, starting.clone());
            if names.insert(tool.name.clone()) {
                tools.push(Arc::new(tool));
            }
        }
    }
    Assembly { tools, feature: Some(McpFeature { servers }) }
}

pub struct McpTool {
    server: Server,
    name: String,
    server_name: String,
    decl: ToolDecl,
    parameters: Value,
    starting: Starting,
}

impl McpTool {
    fn new(server: Server, spec: &ServerSpec, decl: ToolDecl, starting: Starting) -> Self {
        Self { name: tool_name(&spec.slug, &decl.name), server_name: spec.name.clone(), parameters: parameters(&decl.input_schema), decl, server, starting }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        let about = self.decl.description.as_deref().unwrap_or_default().trim();
        let description: String = format!("(MCP server \"{}\") {about}", self.server_name).trim().chars().take(MAX_DESCRIPTION_CHARS).collect();
        ToolSpec {
            name: self.name.clone(),
            description,
            input_schema: self.parameters.clone(),
            loading: Loading::Deferred,
            parallel_safe: self.decl.read_only(),
            hint: hint(&self.server_name, &self.decl),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<String, String> {
        let spec = self.server.spec();
        if spec.tool_disabled(&self.decl.name) {
            return Err(format!("{} is switched off in Settings › MCP servers.", self.name));
        }
        let _busy = self.server.busy();
        let waits = self.server.connecting();
        if waits {
            (self.starting)(vec![spec.name.clone()]);
        }
        let connection = self.server.connection(&context.cancel).await;
        if waits {
            (self.starting)(Vec::new());
        }
        let connection = connection?;
        let tool = &self.decl.name;
        let current = connection
            .tools()
            .into_iter()
            .find(|current| current.name == *tool)
            .ok_or_else(|| format!("MCP tool \"{tool}\" is no longer offered by \"{}\". Try again with your next message.", spec.name))?;
        // A cached read-only hint is never an access grant: the server may have changed it.
        if self.decl.read_only() && !current.read_only() {
            return Err(format!("\"{}\" no longer marks MCP tool \"{tool}\" read-only. Try again with your next message.", spec.name));
        }
        if parameters(&current.input_schema) != self.parameters {
            return Err(format!("MCP tool \"{tool}\" changed its input schema. Try again with your next message."));
        }
        let arguments = if input.is_object() { input } else { json!({}) };
        let result = connection.call(tool, arguments, &context.cancel).await?;
        result_text(&result).map_err(|error| spec.redact(&error))
    }
}

/// One line for the system prompt: the server, then the start of what the tool does.
fn hint(server: &str, decl: &ToolDecl) -> String {
    let about = decl.description.as_deref().unwrap_or_default();
    let first = about.split_terminator(['\n']).next().unwrap_or_default().trim();
    let first = first.split_inclusive(". ").next().unwrap_or(first).trim();
    let text = if first.is_empty() { decl.label().to_string() } else { first.to_string() };
    let text: String = if text.chars().count() > MAX_HINT_CHARS { format!("{}…", text.chars().take(MAX_HINT_CHARS - 1).collect::<String>()) } else { text };
    format!("[{server}] {text}")
}

/// Tells the model, once per failure or drop, that one of its MCP servers is unavailable.
pub struct McpFeature {
    servers: Vec<Server>,
}

impl Feature for McpFeature {
    fn notes(&self, session: &Session) -> Vec<Note> {
        self.servers
            .iter()
            .filter_map(|server| {
                let (stamp, text) = server.news()?;
                let tag = format!("{NOTE_PREFIX}{}:{stamp:016x}", server.id());
                (!session.ever_noted(&tag)).then_some(Note { tag, text })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_are_one_short_line_naming_the_server() {
        let decl = |description: &str| ToolDecl::parse(&json!({ "name": "create_issue", "description": description })).unwrap();
        assert_eq!(hint("GitHub", &decl("Create an issue. Needs a repository.\nMore.")), "[GitHub] Create an issue.");
        assert_eq!(hint("GitHub", &decl("")), "[GitHub] create_issue");
        assert!(hint("GitHub", &decl(&"word ".repeat(100))).chars().count() <= "[GitHub] ".len() + MAX_HINT_CHARS);
    }
}
