//! `agent`: the parent model's way to start a sub-agent and wait for its report.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    AgentsSetup, Delegation, ModelPref, Waiting, child_tools, children_folder,
    child::{self, Launch},
    prompt,
};
use crate::{
    agent::Control,
    plan::Plan,
    tool::{Loading, Tool, ToolContext, ToolSpec},
};

pub const AGENT: &str = "agent";

pub struct AgentTool {
    setup: AgentsSetup,
    /// Every tool the parent has; each child gets its role's share.
    pool: Vec<Arc<dyn Tool>>,
    /// The parent's mode, read when a child starts.
    plan: Plan,
    unrestricted: bool,
    skills: Option<String>,
    /// The parent's session file: children's sessions go in a folder beside it.
    parent_session: PathBuf,
}

impl AgentTool {
    pub fn new(setup: AgentsSetup, pool: Vec<Arc<dyn Tool>>, plan: Plan, unrestricted: bool, skills: Option<String>, parent_session: PathBuf) -> Self {
        Self { setup, pool, plan, unrestricted, skills, parent_session }
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        let names: Vec<&str> = self.setup.catalog.iter().map(|agent| agent.name.as_str()).collect();
        ToolSpec {
            name: AGENT.to_string(),
            description: prompt::agent_description(&self.setup.catalog),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "enum": names, "description": "Which agent to start." },
                    "description": { "type": "string", "description": "A short label for the task, three to five words, shown to the user." },
                    "prompt": { "type": "string", "description": "The complete brief: the goal, the context and paths it needs, and the report you want back. The agent sees nothing else." }
                },
                "required": ["agent", "description", "prompt"],
                "additionalProperties": false
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: prompt::AGENT_HINT.to_string(),
            guidelines: prompt::guidelines(self.setup.delegation == Delegation::OnRequest),
            ends_run: false,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<String, String> {
        let name = input["agent"].as_str().unwrap_or_default().trim();
        let brief = input["prompt"].as_str().unwrap_or_default().trim();
        let description = input["description"].as_str().unwrap_or_default().trim();
        let Some(def) = self.setup.catalog.iter().find(|agent| agent.name == name) else {
            let names: Vec<&str> = self.setup.catalog.iter().map(|agent| agent.name.as_str()).collect();
            return Err(format!("There is no agent named \"{name}\". Choose one of: {}.", names.join(", ")));
        };
        if brief.is_empty() {
            return Err("The prompt is empty. Give the agent a complete brief.".to_string());
        }
        let planning = self.plan.mode().planning() && !self.unrestricted;
        if planning && def.edits() {
            let readers: Vec<&str> = self.setup.catalog.iter().filter(|agent| !agent.edits()).map(|agent| agent.name.as_str()).collect();
            return Err(format!(
                "Plan mode only starts agents that make no edits ({}). {} can edit files.",
                if readers.is_empty() { "none are switched on".to_string() } else { readers.join(", ") },
                def.name
            ));
        }
        let model = match def.model {
            ModelPref::Helper => self.setup.helper.clone().unwrap_or_else(|| self.setup.chat.clone()),
            ModelPref::Chat => self.setup.chat.clone(),
        };
        let description = if description.is_empty() { def.name.clone() } else { description.to_string() };
        let control = Control::new();
        let folder = children_folder(&self.parent_session);
        let child = self.setup.agents.add(&folder, &def.name, &description, &context.call_id, &model.settings.model, def.edits(), control.clone());
        let chat = self.parent_session.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let launch = Launch {
            tools: child_tools(def, &self.pool),
            def: def.clone(),
            brief: brief.to_string(),
            model,
            cwd: context.cwd.clone(),
            skills: self.skills.clone(),
            planning,
            ledger_id: format!("{chat}/{}", child.id),
        };
        child::start(child.clone(), launch, control);
        let waiting = Waiting(child.clone());
        waiting.0.ended().await;
        drop(waiting);
        child.take_report().unwrap_or_else(|| Err("The agent's report was already delivered.".to_string()))
    }
}
