//! Settings › Sub-agents: how freely the agent delegates, the helper model the roles that ask for
//! it run on, which roles are switched off, and which other tools' agent folders are read.
//!
//! The roles themselves are files (`wack_agent::agents::definition`): the user's own in
//! `<data>/agents`, the only agent folder WackCode writes to, then other tools' folders the user
//! switched on, then the built-ins. A project's own agent folders are never read.
//!
//! A helper model's key goes the same way as the chat's: [`Store::provider`] builds the provider
//! for the run, and nothing else sees it.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
pub use wack_agent::{Agents, Delegation};
use wack_agent::{
    AgentsSetup, ChildModel, Settings,
    agents::definition::{self, Catalog},
    skills::Root,
};

use crate::store::{ModelChoice, Reasoning, Resolved, Store};

// What the UI needs of sub-agents, so it depends on the engine alone.
pub use wack_agent::agents::{
    AGENT, AGENTS_STATE, Activity, AgentDef, AgentInfo, ModelPref, NOTE_PREFIX, Signal, Status, StoppedBy,
};

const LIBRARY_LABEL: &str = "Your agents";

/// Other tools' agent folders, which the user may switch on.
pub const TOOL_FOLDERS: &[(&str, &str, &str)] = &[("claude", "Claude Code", ".claude/agents")];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub delegation: Delegation,
    /// The model roles marked `model: helper` run on. `None` runs them on the chat's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<ModelChoice>,
    /// Names of roles switched off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<String>,
    /// Ids of other tools' folders that are switched on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_folders: Vec<String>,
}

impl Store {
    pub fn agents_config(&self) -> &AgentsConfig {
        self.agents_state()
    }

    pub fn set_agents_config(&mut self, config: AgentsConfig) -> Result<(), String> {
        *self.agents_state_mut() = config;
        self.save_state()
    }

    /// `<data>/agents`, where the user's own roles live.
    pub fn agents_folder(&self) -> PathBuf {
        self.folder().join("agents")
    }

    /// The folders roles load from, switched on, in order.
    pub fn agent_roots(&self) -> Vec<Root> {
        let home = self.home();
        let config = self.agents_config();
        let mut roots = vec![Root { path: self.agents_folder(), label: LIBRARY_LABEL.to_string() }];
        for (id, label, path) in TOOL_FOLDERS {
            if config.tool_folders.iter().any(|on| on == id) {
                roots.push(Root { path: home.join(path), label: label.to_string() });
            }
        }
        roots
    }

    /// Every role a chat may use now, and what could not load. Reads the disk.
    pub fn agent_catalog(&self) -> Catalog {
        let off: HashSet<String> = self.agents_config().off.iter().cloned().collect();
        definition::load(&self.agent_roots(), &off)
    }

    /// The connection and model a choice names, only if both still exist and the connection is
    /// on: unlike the chat's choice, a helper never falls back to some other model.
    fn resolve_exact(&self, choice: &ModelChoice) -> Option<Resolved> {
        let connection = self.enabled_connections().find(|connection| Some(&connection.id) == choice.connection.as_ref())?;
        let model = connection.models.iter().find(|model| Some(&model.id) == choice.model.as_ref())?;
        Some(Resolved { connection: connection.clone(), effort: model.pick(choice.effort), model: model.clone() })
    }

    /// A model a sub-agent can run on. An Ultra choice runs at High: Ultra is the chat's own.
    pub fn child_model(&self, resolved: &Resolved, session: &str, workspace: &Path) -> Result<ChildModel, String> {
        let level = match resolved.effort {
            Reasoning::Ultra => resolved.model.pick(Some(Reasoning::High)),
            level => level,
        };
        Ok(ChildModel {
            provider: Arc::new(self.provider(&resolved.connection)?),
            settings: Settings {
                model: resolved.model.id.clone(),
                effort: resolved.model.effort_for(level),
                max_output_tokens: None,
                context_window: resolved.model.context_window,
                ultra: false,
            },
            recorder: Some(self.usage_recorder(&resolved.connection, session, workspace)),
        })
    }

    /// What a run needs to start sub-agents, or `None` when delegation is off. `chat` is the chat's
    /// own model; `agents` the chat's registry.
    pub fn agents_setup(&self, chat: ChildModel, session: &str, workspace: &Path, agents: Agents) -> Option<AgentsSetup> {
        let config = self.agents_config();
        if config.delegation == Delegation::Off {
            return None;
        }
        // A helper that no longer resolves (its connection is gone or off) falls back to the chat's.
        let helper = config
            .helper
            .as_ref()
            .and_then(|choice| self.resolve_exact(choice))
            .and_then(|resolved| self.child_model(&resolved, session, workspace).ok());
        Some(AgentsSetup { catalog: self.agent_catalog().agents, delegation: config.delegation, chat, helper, agents })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Model, NewConnection};
    use wack_llm::WireFormat;

    fn store(folder: &Path) -> Store {
        let mut store = Store::open(folder).unwrap();
        store.set_home_override(folder.join("home"));
        store
            .add_connection(
                NewConnection {
                    name: "Testing".into(),
                    format: WireFormat::OpenaiChat,
                    base_url: "https://example.invalid/v1".into(),
                    models: vec![Model::new("big", None), Model::new("small", None)],
                },
                "key",
            )
            .unwrap();
        store
    }

    #[test]
    fn the_users_roles_come_first_and_other_tools_folders_only_when_switched_on() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = store(folder.path());
        std::fs::create_dir_all(store.agents_folder()).unwrap();
        std::fs::write(store.agents_folder().join("docs.md"), "---\ndescription: Docs\n---\nCheck docs.").unwrap();
        let claude = folder.path().join("home/.claude/agents");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("tester.md"), "---\nname: tester\ndescription: Tests\ntools: Read, Bash\n---\nTest.").unwrap();

        let names = |store: &Store| store.agent_catalog().agents.iter().map(|agent| agent.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&store), ["docs", "explore", "review", "general"]);

        let mut config = store.agents_config().clone();
        config.tool_folders = vec!["claude".into()];
        config.off = vec!["review".into()];
        store.set_agents_config(config).unwrap();
        let reopened = Store::open(folder.path()).unwrap();
        assert_eq!(reopened.agents_config().off, ["review"]);
        assert_eq!(names(&store), ["docs", "tester", "explore", "general"]);
    }

    #[test]
    fn a_helper_runs_on_its_own_model_and_off_means_no_sub_agents() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = store(folder.path());
        let connection = store.connections()[0].id.clone();
        let chat = store.child_model(&store.resolve(&ModelChoice::default()).unwrap(), "chat-1", folder.path()).unwrap();
        assert_eq!(chat.settings.model, "big");

        let setup = store.agents_setup(chat.clone(), "chat-1", folder.path(), Agents::new()).unwrap();
        assert!(setup.helper.is_none());
        assert_eq!(setup.catalog.len(), 3);

        let mut config = store.agents_config().clone();
        config.helper = Some(ModelChoice { connection: Some(connection), model: Some("small".into()), effort: None });
        store.set_agents_config(config.clone()).unwrap();
        let setup = store.agents_setup(chat.clone(), "chat-1", folder.path(), Agents::new()).unwrap();
        assert_eq!(setup.helper.unwrap().settings.model, "small");

        config.delegation = Delegation::Off;
        store.set_agents_config(config).unwrap();
        assert!(store.agents_setup(chat, "chat-1", folder.path(), Agents::new()).is_none());
    }
}
