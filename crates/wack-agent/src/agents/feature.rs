//! What the parent model is told about its children between tool calls: reports from those in
//! the background, the ones a quit interrupted, and the list the session keeps.

use std::sync::Mutex;

use serde_json::Value;

use super::{Agents, AgentInfo, NOTE_PREFIX};
use crate::{agent::Feature, session::Note, session::Session};

/// The session state that keeps a chat's children, so the list survives a relaunch.
pub const AGENTS_STATE: &str = "agents";

#[derive(Clone)]
pub struct AgentsFeature {
    agents: Agents,
    /// Notes for children a quit interrupted, told once.
    orphans: std::sync::Arc<Mutex<Vec<Note>>>,
}

impl AgentsFeature {
    /// Restores the children the session saved into `agents`, and prepares a note for each that
    /// was working when the app quit.
    pub fn new(session: &Session, agents: Agents) -> Self {
        let saved: Vec<AgentInfo> = session.state(AGENTS_STATE).and_then(|value| serde_json::from_value(value.clone()).ok()).unwrap_or_default();
        let orphans = agents
            .restore(saved)
            .into_iter()
            .map(|info| Note {
                tag: format!("{NOTE_PREFIX}{}:interrupted", info.id),
                text: format!(
                    "Agent {} ({} · {}) was stopped when WackCode quit, before it reported. What it did is in its session, {}. Start a new agent if you still need its work.",
                    info.id,
                    info.role,
                    info.description,
                    info.session.display()
                ),
            })
            .collect();
        Self { agents, orphans: std::sync::Arc::new(Mutex::new(orphans)) }
    }
}

impl Feature for AgentsFeature {
    fn notes(&self, _session: &Session) -> Vec<Note> {
        let mut notes: Vec<Note> = self.orphans.lock().unwrap().drain(..).collect();
        notes.extend(self.agents.take_news());
        notes
    }

    fn pending(&self) -> bool {
        self.agents.wakes()
    }

    fn state_to_save(&self) -> Vec<(String, Value)> {
        let list = self.agents.list();
        if list.is_empty() {
            return Vec::new();
        }
        vec![(AGENTS_STATE.to_string(), serde_json::to_value(list).unwrap_or(Value::Null))]
    }

    fn summary_sections(&self) -> Vec<String> {
        let working: Vec<String> = self
            .agents
            .active()
            .into_iter()
            .filter(|info| info.background)
            .map(|info| format!("- {} ({} · {})", info.id, info.role, info.description))
            .collect();
        if working.is_empty() {
            return Vec::new();
        }
        vec![format!("## Agents working in the background\n\nTheir reports will arrive as messages.\n\n{}", working.join("\n"))]
    }
}
