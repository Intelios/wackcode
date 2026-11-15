//! The features every coding chat runs with, assembled for one run: the question tool, the todo
//! list and Plan mode. The app and the command line both build their agent from this, so a chat
//! behaves the same in either.
//!
//! A kit is built from the session at the start of a run and dropped at its end: what must
//! outlast the run is in the session's state lines, and what must not (Ultra Plan's "Write the
//! plan now") goes with the kit.

use std::sync::Arc;

use crate::{
    agent::{Agent, Feature},
    ask::AskUserQuestion,
    plan::{AgentMode, Plan},
    session::Session,
    todo::Todo,
    tool::Tool,
};

pub struct Kit {
    pub plan: Plan,
    pub todo: Todo,
}

impl Kit {
    /// `mode` is the chat's mode as its owner has it; `None` keeps the mode the session last
    /// saved. `unrestricted` lifts planning's read-only policy.
    pub fn new(session: &Session, mode: Option<AgentMode>, unrestricted: bool) -> Self {
        Self { plan: Plan::new(session, mode, unrestricted), todo: Todo::new(session) }
    }

    /// The tools these features add to a chat's own, in the order the model is shown them.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let plan = self.plan.clone();
        let interviewing = Arc::new(move || plan.mode() == AgentMode::UltraPlan);
        let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(AskUserQuestion::new(interviewing)), self.todo.tool()];
        tools.extend(self.plan.tools());
        tools
    }

    fn features(&self) -> Vec<Box<dyn Feature>> {
        vec![Box::new(self.plan.clone()), Box::new(self.todo.clone())]
    }

    /// Hooks the features into an agent whose registry holds [`Kit::tools`].
    pub fn attach(&self, agent: Agent) -> Agent {
        self.features().into_iter().fold(agent, Agent::with_feature)
    }
}
