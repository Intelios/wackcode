//! The features every coding chat runs with, assembled for one run: the question tool, the todo
//! list, Plan mode and, when the engine hands it a folder, memory. The app and the command line
//! both build their agent from this, so a chat behaves the same in either.
//!
//! A kit is built from the session at the start of a run and dropped at its end: what must
//! outlast the run is in the session's state lines, and what must not (Ultra Plan's "Write the
//! plan now") goes with the kit. Memory's index is read here, once per run, so the system prompt
//! stays fixed for the run while a save made during it shows from the next one.

use std::{path::PathBuf, sync::Arc};

use crate::{
    agent::{Agent, Feature},
    ask::AskUserQuestion,
    memory::Memory,
    plan::{AgentMode, Plan},
    session::Session,
    todo::Todo,
    tool::Tool,
};

pub struct Kit {
    pub plan: Plan,
    pub todo: Todo,
    /// The project's memory, and its index as it stood when the run started.
    memory: Option<(Memory, Option<String>)>,
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
        Self { plan: Plan::new(session, mode, unrestricted), todo: Todo::new(session), memory }
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
        vec![Box::new(self.plan.clone()), Box::new(self.todo.clone())]
    }

    /// Hooks the features into an agent whose registry holds [`Kit::tools`].
    pub fn attach(&self, mut agent: Agent) -> Agent {
        if let Some((_, Some(index))) = &self.memory {
            agent = agent.with_system_part(index.clone());
        }
        self.features().into_iter().fold(agent, Agent::with_feature)
    }
}
