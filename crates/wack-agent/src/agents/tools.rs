//! The parent model's tools for sub-agents.
//!
//! - `agent` starts a child and either waits for its report or lets it work in the background
//!   and report when it ends. With `with_conversation` the child starts from a copy of the
//!   conversation instead of a fresh brief.
//! - `agent_message` talks to a child the chat already has: a working one reads the message
//!   before its next step; a finished one works another round in its own session, with all it
//!   learned, under its role's rules as they are now (access revoked since is not kept), on the
//!   tier it started on.
//! - `model` picks the tier (`lead` or `crew`) and `files` names what the child owns. Both are in
//!   the schema whether or not the chat's Team is on, so the tool list never changes with it.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    AgentDef, AgentsSetup, Child, ChildModel, Delegation, MAX_PER_MESSAGE, ModelPref, Spec, Tier, Waiting, child_tools, children_folder,
    child::{self, Launch, Start},
    prompt, team,
};
use crate::{
    agent::Control,
    plan::Plan,
    todo::{self, Todo},
    tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec},
};

/// The most files one child may be given.
const MAX_FILES: usize = 50;

pub const AGENT: &str = "agent";
pub const AGENT_MESSAGE: &str = "agent_message";
/// The role name a child started from a copy of the conversation is listed under.
pub const COPY_ROLE: &str = "copy";

/// What both tools share: the roles, the models, the parent's tools and mode.
struct Shared {
    setup: AgentsSetup,
    /// Every tool the parent has but these; each child gets its role's share.
    pool: Vec<Arc<dyn Tool>>,
    /// The parent's mode, read when a child starts.
    plan: Plan,
    /// The parent's todo list, for a Team worker's briefing.
    todo: Todo,
    unrestricted: bool,
    skills: Option<String>,
    /// The parent's session file: children's sessions go in a folder beside it.
    parent_session: PathBuf,
}

impl Shared {
    fn planning(&self) -> bool {
        self.plan.mode().planning() && !self.unrestricted
    }

    /// The tier a role runs on when the call doesn't say: its `team` line while Team is on (the
    /// crew when it has none), its `model` line otherwise.
    fn default_tier(&self, def: &AgentDef) -> Tier {
        if self.setup.team() {
            return def.team.unwrap_or(Tier::Crew);
        }
        match def.model {
            ModelPref::Helper => Tier::Crew,
            ModelPref::Chat => Tier::Lead,
        }
    }

    /// The model a tier runs on. The crew is Team's crew, else the helper, else the chat's own.
    fn model(&self, tier: Tier) -> ChildModel {
        match tier {
            Tier::Lead => self.setup.chat.clone(),
            Tier::Crew => self.setup.crew.clone().or_else(|| self.setup.helper.clone()).unwrap_or_else(|| self.setup.chat.clone()),
        }
    }

    /// What a fresh child of a Team is told before its brief.
    fn briefing(&self, context: &ToolContext<'_>, files: &[String], brief: &str) -> String {
        let todo: Vec<String> = self.todo.list().visible().filter(|task| task.status != todo::Status::Completed).map(todo::list_line).collect();
        let mut siblings: Vec<team::Sibling> = self
            .setup
            .agents
            .active()
            .into_iter()
            .map(|info| team::Sibling { who: format!("{} ({} · {})", info.id, info.role, info.description), files: info.files, call_id: info.call_id })
            .collect();
        // Agents the same reply starts may not be listed yet: the reply itself names them.
        let reply = context.seen.messages.last().into_iter().flat_map(|reply| reply.tool_uses());
        for (id, name, input) in reply {
            if name != AGENT || id == context.call_id || input["with_conversation"].as_bool() == Some(true) || siblings.iter().any(|sibling| sibling.call_id == id) {
                continue;
            }
            let who = format!("starting with you ({} · {})", str_input(input, "agent"), str_input(input, "description"));
            siblings.push(team::Sibling { who, files: files_input(input, &context.cwd).unwrap_or_default(), call_id: id.to_string() });
        }
        team::briefing(team::Briefing { request: context.seen.request, plan: self.plan.implementing(), todo, siblings, files }, brief)
    }

    fn role(&self, name: &str) -> Result<&AgentDef, String> {
        self.setup.catalog.iter().find(|agent| agent.name == name).ok_or_else(|| {
            let names: Vec<&str> = self.setup.catalog.iter().map(|agent| agent.name.as_str()).collect();
            format!("There is no agent named \"{name}\". Choose one of: {}.", names.join(", "))
        })
    }

    /// `Err` when Plan mode keeps this role from working.
    fn check_planning(&self, def: &AgentDef) -> Result<(), String> {
        if !self.planning() || !def.edits() {
            return Ok(());
        }
        let readers: Vec<&str> = self.setup.catalog.iter().filter(|agent| !agent.edits()).map(|agent| agent.name.as_str()).collect();
        Err(format!(
            "Plan mode only starts agents that make no edits ({}). {} can edit files.",
            if readers.is_empty() { "none are switched on".to_string() } else { readers.join(", ") },
            def.name
        ))
    }

    fn ledger_id(&self, child: &str) -> String {
        let chat = self.parent_session.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{chat}/{child}")
    }

    fn spend(&self) -> Result<(), String> {
        if self.setup.agents.spend() {
            return Ok(());
        }
        Err(format!(
            "You have started {MAX_PER_MESSAGE} agents since the user's last message, the most allowed. Carry on with what you have, or ask the user before starting more."
        ))
    }

    /// Starts `child`, then waits for its report, or returns at once for one in the background.
    async fn launch(&self, child: Arc<Child>, launch: Launch, control: Control, background: bool) -> Result<String, String> {
        child::start(child.clone(), self.setup.agents.clone(), launch, control);
        if background {
            return Ok(format!(
                "Agent {} ({}) is working in the background. Its report will arrive as a message when it finishes. Carry on with other work meanwhile; don't poll for it or guess what it will say.",
                child.id, child.role
            ));
        }
        let waiting = Waiting(child.clone());
        tokio::select! {
            () = child.ended() => {}
            () = child.detached() => {}
        }
        drop(waiting);
        // Moving and ending decide under one lock: a child that moved reports as a note.
        if child.in_background() {
            return Ok(moved_to_background(&child.id, &child.role));
        }
        child.take_report().unwrap_or_else(|| Err("The agent's report was already delivered.".to_string()))
    }
}

pub struct AgentTool(Arc<Shared>);

impl AgentTool {
    #[allow(clippy::too_many_arguments)]
    pub fn new(setup: AgentsSetup, pool: Vec<Arc<dyn Tool>>, plan: Plan, todo: Todo, unrestricted: bool, skills: Option<String>, parent_session: PathBuf) -> Self {
        Self(Arc::new(Shared { setup, pool, plan, todo, unrestricted, skills, parent_session }))
    }

    /// `agent_message`, sharing this tool's roles and models.
    pub fn messages(&self) -> AgentMessage {
        AgentMessage(self.0.clone())
    }
}

fn str_input<'a>(input: &'a Value, key: &str) -> &'a str {
    input[key].as_str().unwrap_or_default().trim()
}

/// The tier a call asks for, if it asks.
fn tier_input(input: &Value) -> Result<Option<Tier>, String> {
    match input.get("model") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Tier::parse(text).map(Some).ok_or_else(|| format!("model must be \"lead\" or \"crew\", not \"{text}\".")),
        Some(_) => Err("model must be \"lead\" or \"crew\".".to_string()),
    }
}

/// The files a call gives its child, named from the project and without repeats.
fn files_input(input: &Value, cwd: &std::path::Path) -> Result<Vec<String>, String> {
    let list = match input.get("files") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err("files must be a list of paths.".to_string()),
    };
    let mut files: Vec<String> = Vec::new();
    for entry in list {
        let path = entry.as_str().ok_or("files must be a list of paths.")?.trim();
        if path.is_empty() {
            continue;
        }
        let path = child::relative(cwd, path);
        if !files.contains(&path) {
            files.push(path);
        }
    }
    if files.len() > MAX_FILES {
        return Err(format!("Give an agent at most {MAX_FILES} files; name a folder instead of its every file."));
    }
    Ok(files)
}

#[async_trait]
impl Tool for AgentTool {
    fn spec(&self) -> ToolSpec {
        let setup = &self.0.setup;
        let names: Vec<&str> = setup.catalog.iter().map(|agent| agent.name.as_str()).collect();
        ToolSpec {
            name: AGENT.to_string(),
            description: prompt::agent_description(&setup.catalog),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "enum": names, "description": "Which agent to start. Not needed with with_conversation." },
                    "description": { "type": "string", "description": "A short label for the task, three to five words, shown to the user." },
                    "prompt": { "type": "string", "description": "The complete brief: the goal, the context and paths it needs, and the report you want back. A fresh agent sees nothing else. With with_conversation, a directive: what to do now." },
                    "model": { "type": "string", "enum": ["lead", "crew"], "description": "Which model runs it. lead: your own, for hard or judgment-heavy work and reviews. crew: the cheaper model beside yours, for well-specified work. Leave it out for the role's own." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "The files (or folders) this agent owns: the ones it may change. Shown to the user and to agents working beside it. Give agents working at the same time separate files." },
                    "background": { "type": "boolean", "description": "Return at once and let the agent work while you do other things. Its report arrives as a message when it ends. Leave this out when your next step needs the report." },
                    "with_conversation": { "type": "boolean", "description": "Start the agent from a copy of this conversation instead of a fresh context, with your tools and model: for a side task that needs what you already know, whose working you don't need to keep. Cheaper than briefing a fresh agent when the context is large." }
                },
                "required": ["description", "prompt"],
                "additionalProperties": false
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: prompt::AGENT_HINT.to_string(),
            guidelines: prompt::guidelines(setup.delegation == Delegation::OnRequest),
            ends_run: false,
            together: true,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let shared = &self.0;
        let brief = str_input(&input, "prompt");
        if brief.is_empty() {
            return Err("The prompt is empty. Give the agent a complete brief.".to_string());
        }
        let background = input["background"].as_bool() == Some(true);
        let copy = input["with_conversation"].as_bool() == Some(true);
        let requested = tier_input(&input)?;
        let files = files_input(&input, &context.cwd)?;
        let (def, model, tier) = if copy {
            if requested == Some(Tier::Crew) {
                return Err("A copy of the conversation runs on your own model, so its requests match yours. Leave out model, or brief a fresh agent to use the crew.".to_string());
            }
            // A copy is the parent's twin: its tools, model and settings, with General's limits.
            let settings = context.seen.settings.ok_or("This chat can't copy its conversation.")?;
            let model = ChildModel { settings: settings.clone(), ..shared.setup.chat.clone() };
            (definition_for_copy(&shared.setup), model, Tier::Lead)
        } else {
            let def = shared.role(str_input(&input, "agent"))?.clone();
            shared.check_planning(&def)?;
            let tier = requested.unwrap_or_else(|| shared.default_tier(&def));
            (def, shared.model(tier), tier)
        };
        shared.spend()?;
        let description = match str_input(&input, "description") {
            "" => def.name.clone(),
            description => description.to_string(),
        };
        let control = Control::new();
        let folder = children_folder(&shared.parent_session);
        // A Team worker's briefing is read before the child is listed, so it never names itself.
        let brief = if shared.setup.team() && !copy { shared.briefing(context, &files, brief) } else { brief.to_string() };
        let spec = Spec {
            role: def.name.clone(),
            description,
            call_id: context.call_id.clone(),
            model: model.settings.model.clone(),
            edits: def.edits(),
            tier,
            files,
        };
        let child = shared.setup.agents.add(&folder, spec, background, control.clone());
        let start = if copy {
            let registry = context.seen.registry.ok_or("This chat can't copy its conversation.")?;
            Start::Copy {
                system: context.seen.system.to_vec(),
                messages: context.seen.messages.to_vec(),
                loaded: context.seen.loaded.to_vec(),
                registry: registry.clone(),
                settings: model.settings.clone(),
                call_id: context.call_id.clone(),
                directive: if shared.setup.team() { format!("{}\n\n{brief}", team::COPY_LINE) } else { brief },
            }
        } else {
            Start::Fresh { brief, tools: child_tools(&def, &shared.pool) }
        };
        let launch = Launch {
            def,
            start,
            model,
            cwd: context.cwd.clone(),
            skills: shared.skills.clone(),
            planning: shared.planning(),
            ledger_id: shared.ledger_id(&child.id),
            child_prompt: shared.setup.child_prompt.clone(),
        };
        shared.launch(child, launch, control, background).await.map(ToolOutput::from)
    }
}

/// The limits a copy of the conversation works under: General's, named as a copy.
fn definition_for_copy(setup: &AgentsSetup) -> AgentDef {
    let general = super::definition::builtins().into_iter().find(|agent| agent.name == "general").expect("General is built in");
    let max_turns = setup.catalog.iter().find(|agent| agent.name == "general").map_or(general.max_turns, |agent| agent.max_turns);
    AgentDef { name: COPY_ROLE.to_string(), description: "A copy of the conversation.".to_string(), tools: None, max_turns, ..general }
}

pub struct AgentMessage(Arc<Shared>);

#[async_trait]
impl Tool for AgentMessage {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: AGENT_MESSAGE.to_string(),
            description: "Send a message to one of this chat's agents, by its id (a1, a2…). One still working reads it before its next step, and this returns at once. One that has finished, was stopped or was interrupted works another round with everything it learned, and this waits for its new report (or returns at once with background). Use it for feedback on a report, a follow-up question, or to finish work an agent left, instead of briefing a new agent from scratch.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The agent's id, as its report or start gave it (a1, a2…)." },
                    "message": { "type": "string", "description": "What to tell it. It remembers its own task and work, so say what's new." },
                    "background": { "type": "boolean", "description": "For a finished agent: return at once; its new report arrives as a message." }
                },
                "required": ["agent_id", "message"],
                "additionalProperties": false
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "message one of this chat's agents: steer one that's working, or give one that finished another round".to_string(),
            guidelines: Vec::new(),
            ends_run: false,
            together: true,
        }
    }

    async fn run(&self, input: Value, context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let shared = &self.0;
        let id = str_input(&input, "agent_id");
        let message = str_input(&input, "message");
        if message.is_empty() {
            return Err("The message is empty.".to_string());
        }
        let agents = &shared.setup.agents;
        let child = agents.get(id).ok_or_else(|| {
            let known: Vec<String> = agents.list().into_iter().map(|info| info.id).collect();
            if known.is_empty() {
                "This chat has no agents yet. Start one with agent.".to_string()
            } else {
                format!("There is no agent {id} in this chat. Its agents: {}.", known.join(", "))
            }
        })?;
        // A call with no id is the user's own, from the agent's pane.
        let from_user = context.call_id.is_empty();
        let message = if from_user { self::from_user(message) } else { message.to_string() };
        if child.active() {
            child.steer(&message);
            return Ok(format!("Sent to agent {id}, which is still working: it reads the message before its next step. Its report will come as before.").into());
        }
        let background = input["background"].as_bool() == Some(true);
        let (def, model, tools, system) = if child.role == COPY_ROLE {
            // A copy keeps its parent's tools, so its requests still match the parent's.
            let settings = context.seen.settings.ok_or("This chat can't continue a copy of its conversation.")?;
            let registry = context.seen.registry.ok_or("This chat can't continue a copy of its conversation.")?;
            let model = ChildModel { settings: settings.clone(), ..shared.setup.chat.clone() };
            let tools = registry.specs().filter_map(|spec| registry.get(&spec.name).map(|(_, tool)| tool.clone())).collect();
            (definition_for_copy(&shared.setup), model, tools, Some(context.seen.system.to_vec()))
        } else {
            let def = shared
                .role(&child.role)
                .map_err(|_| format!("Agent {id}'s role, {}, is no longer switched on, so it can't work another round.", child.role))?
                .clone();
            shared.check_planning(&def)?;
            // It stays on the tier it started on, so its own cache holds and the crew's work
            // never moves to the lead's model unasked.
            let tier = child.tier.unwrap_or_else(|| shared.default_tier(&def));
            let model = shared.model(tier);
            let tools = child_tools(&def, &shared.pool);
            (def, model, tools, None)
        };
        // The user's own follow-ups don't count against the agent's budget.
        if !from_user {
            shared.spend()?;
        }
        let control = Control::new();
        let child = agents.resume(id, &context.call_id, &model.settings.model, background, control.clone())?;
        let launch = Launch {
            def,
            start: Start::Continue { message, tools, system },
            model,
            cwd: context.cwd.clone(),
            skills: shared.skills.clone(),
            planning: shared.planning(),
            ledger_id: shared.ledger_id(id),
            child_prompt: shared.setup.child_prompt.clone(),
        };
        shared.launch(child, launch, control, background).await.map(ToolOutput::from)
    }
}

/// A message the user sent an agent from its pane, as the agent reads it.
pub(crate) fn from_user(text: &str) -> String {
    format!("The user, who can see your work in WackCode, sends you this message:\n\n{text}")
}

/// The result of a call whose child the user moved to the background (⌘B).
pub fn moved_to_background(id: &str, role: &str) -> String {
    format!(
        "The user moved agent {id} ({role}) to the background; it is still working. Its report will arrive as a message when it finishes. Carry on with other work meanwhile; don't poll for it or guess what it will say."
    )
}
