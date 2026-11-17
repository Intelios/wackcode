//! The features every coding chat runs with, assembled for one run: the question tool, the todo
//! list, Plan mode, the skills index and, when the engine hands it a folder, memory. The app and the command line
//! both build their agent from this, so a chat behaves the same in either.
//!
//! A kit is built from the session at the start of a run and dropped at its end: what must
//! outlast the run is in the session's state lines, and what must not (Ultra Plan's "Write the
//! plan now") goes with the kit. Memory's index is read here, once per run, so the system prompt
//! stays fixed for the run while a save made during it shows from the next one. Skills work the
//! same way: the engine loads the catalogue as the run starts, and its index joins the prompt.

use std::{path::PathBuf, sync::Arc};

use crate::{
    agent::{Agent, Feature},
    agents::{AgentTool, AgentsFeature, AgentsSetup, Delegation},
    ask::AskUserQuestion,
    memory::Memory,
    plan::{AgentMode, Plan},
    session::Session,
    skills::{self, Catalog},
    tasks::{Tasks, TasksFeature},
    todo::Todo,
    tool::Tool,
};

pub struct Kit {
    pub plan: Plan,
    pub todo: Todo,
    /// The project's memory, and its index as it stood when the run started.
    memory: Option<(Memory, Option<String>)>,
    /// The skills index as it stood when the run started.
    skills: Option<String>,
    /// The chat's shell tasks, which outlive the run, and what the session said of them.
    tasks: Option<(Tasks, TasksFeature)>,
    /// Sub-agents, with the session file their sessions go beside, and what the session said of
    /// them.
    agents: Option<(AgentsSetup, PathBuf, AgentsFeature)>,
    unrestricted: bool,
}

impl Kit {
    /// `mode` is the chat's mode as its owner has it; `None` keeps the mode the session last
    /// saved. `unrestricted` lifts planning's read-only policy. `memory` is the project's memory
    /// folder, or `None` when memory is off for it.
    pub fn new(session: &Session, mode: Option<AgentMode>, unrestricted: bool, memory: Option<PathBuf>) -> Self {
        let memory = memory.map(|root| {
            let memory = Memory::new(root);
            let index = memory.index();
            (memory, index)
        });
        Self { plan: Plan::new(session, mode, unrestricted), todo: Todo::new(session), memory, skills: None, tasks: None, agents: None, unrestricted }
    }

    /// Lists `catalog`'s skills in the system prompt, for the model to load when a task matches.
    pub fn with_skills(mut self, catalog: &Catalog) -> Self {
        self.skills = skills::index(catalog);
        self
    }

    /// Runs shell commands as tasks of the chat's registry, which outlives the run, and tells the
    /// model their news. Without it a run's commands go with the run.
    pub fn with_tasks(mut self, session: &Session, tasks: Tasks) -> Self {
        let feature = TasksFeature::new(session, tasks.clone());
        self.tasks = Some((tasks, feature));
        self
    }

    /// Lets the chat start sub-agents. Their tools come from [`Kit::agent_tools`], once the
    /// chat's other tools are assembled.
    pub fn with_agents(mut self, session: &Session, setup: AgentsSetup) -> Self {
        let feature = AgentsFeature::new(session, setup.agents.clone());
        self.agents = Some((setup, session.path().to_path_buf(), feature));
        self
    }

    /// The sub-agent tools, given every other tool the chat has (children get their share of
    /// those). Nothing when sub-agents are off or no role is switched on.
    pub fn agent_tools(&self, pool: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
        let Some((setup, session, _)) = &self.agents else { return Vec::new() };
        if setup.delegation == Delegation::Off || setup.catalog.is_empty() {
            return Vec::new();
        }
        let agent = AgentTool::new(setup.clone(), pool.to_vec(), self.plan.clone(), self.unrestricted, self.skills.clone(), session.clone());
        let messages = agent.messages();
        vec![Arc::new(agent), Arc::new(messages)]
    }

    /// The tools these features add to a chat's own, in the order the model is shown them.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let plan = self.plan.clone();
        let interviewing = Arc::new(move || plan.mode() == AgentMode::UltraPlan);
        let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(AskUserQuestion::new(interviewing)), self.todo.tool()];
        tools.extend(self.plan.tools());
        if let Some((memory, _)) = &self.memory {
            tools.extend(memory.tools());
        }
        tools
    }

    fn features(&self) -> Vec<Box<dyn Feature>> {
        let mut features: Vec<Box<dyn Feature>> = vec![Box::new(self.plan.clone()), Box::new(self.todo.clone())];
        if let Some((_, feature)) = &self.tasks {
            features.push(Box::new(feature.clone()));
        }
        if let Some((_, _, feature)) = &self.agents {
            features.push(Box::new(feature.clone()));
        }
        features
    }

    /// Hooks the features into an agent whose registry holds [`Kit::tools`].
    pub fn attach(&self, mut agent: Agent) -> Agent {
        if let Some((_, Some(index))) = &self.memory {
            agent = agent.with_system_part(index.clone());
        }
        if let Some(index) = &self.skills {
            agent = agent.with_system_part(index.clone());
        }
        if let Some((tasks, _)) = &self.tasks {
            agent = agent.with_tasks(tasks.clone());
        }
        if let Some((setup, _, _)) = &self.agents {
            agent = agent.with_agents(setup.agents.clone());
        }
        self.features().into_iter().fold(agent, Agent::with_feature)
    }
}
