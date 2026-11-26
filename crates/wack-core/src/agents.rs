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
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
pub use wack_agent::{Agents, Delegation};
use wack_agent::{
    AgentsSetup, ChildModel, Settings,
    agents::definition::{self, Catalog, DEFAULT_MAX_TURNS, MAX_MAX_TURNS},
    skills::{
        Root,
        frontmatter::{Value, compose},
    },
};

use crate::store::{ModelChoice, Reasoning, Resolved, Store};

// What the UI needs of sub-agents, so it depends on the engine alone.
pub use wack_agent::agents::definition::{BUILTIN_LABEL, Problem, builtins};
pub use wack_agent::agents::{
    AGENT, AGENT_MESSAGE, AGENTS_STATE, COPY_DIRECTIVE, Activity, AgentDef, AgentInfo, ModelPref, NOTE_PREFIX, Signal, Status, StoppedBy, children_folder,
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

/// One role as Settings › Sub-agents lists it: the one that answers to its name.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentEntry {
    pub agent: AgentDef,
    pub enabled: bool,
    /// A file of the user's that replaces the built-in of the same name.
    pub customised: bool,
}

impl AgentEntry {
    /// The user's own file, which WackCode may change and trash.
    pub fn yours(&self) -> bool {
        self.agent.source == LIBRARY_LABEL
    }
}

/// A role as the editor has it, before it is checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentDraft {
    pub name: String,
    pub description: String,
    /// Tool names separated by commas or spaces; empty is every tool.
    pub tools: String,
    pub model: ModelPref,
    /// Empty is the default.
    pub max_turns: String,
    pub prompt: String,
}

impl AgentDraft {
    pub fn of(agent: &AgentDef) -> Self {
        Self {
            name: agent.name.clone(),
            description: agent.description.clone(),
            tools: agent.tools.as_ref().map(|tools| tools.join(", ")).unwrap_or_default(),
            model: agent.model,
            max_turns: if agent.max_turns == DEFAULT_MAX_TURNS { String::new() } else { agent.max_turns.to_string() },
            prompt: agent.prompt.clone(),
        }
    }

    /// The tools it names, or `None` for every tool.
    pub fn tool_list(&self) -> Option<Vec<String>> {
        let tools: Vec<String> = self.tools.split([',', ' ', '\n']).map(str::trim).filter(|tool| !tool.is_empty()).map(str::to_string).collect();
        (!tools.is_empty()).then_some(tools)
    }

    /// The file it saves as, checked by the same reader that loads it.
    pub fn file_text(&self, original: Option<&str>) -> Result<String, String> {
        let name = self.name.trim();
        definition::check_name(name).map_err(|message| message.replacen("Its name", "The name", 1))?;
        let turns = self.max_turns.trim();
        if !turns.is_empty() && !turns.parse::<u32>().is_ok_and(|turns| (1..=MAX_MAX_TURNS).contains(&turns)) {
            return Err(format!("Steps must be a whole number from 1 to {MAX_MAX_TURNS}."));
        }
        let tools = self.tool_list().map(|tools| format!("[{}]", tools.join(", ")));
        let description = self.description.trim();
        let text = compose(
            original,
            &[
                ("name", Some(Value::Text(name))),
                ("description", Some(Value::Text(description))),
                ("tools", tools.as_deref().map(Value::Raw)),
                ("model", Some(Value::Text(self.model.as_str()))),
                ("max_turns", (!turns.is_empty()).then_some(Value::Raw(turns))),
            ],
            &self.prompt,
        );
        definition::parse(&text, None, LIBRARY_LABEL, None).map_err(|message| message.replacen("It has", "The agent has", 1).replacen("Its ", "The agent's ", 1))?;
        Ok(text)
    }
}

/// Every role by the name it answers to, switched off ones included, and the files that failed.
#[derive(Clone, Debug, Default)]
pub struct AgentsOverview {
    pub entries: Vec<AgentEntry>,
    pub problems: Vec<Problem>,
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

    /// Settings › Sub-agents' list. Reads the disk.
    pub fn agents_overview(&self) -> AgentsOverview {
        let catalog = definition::load(&self.agent_roots(), &HashSet::new());
        let off = &self.agents_config().off;
        // The built-ins first in their own order, customised or not, so editing one never moves
        // it; then the rest in the order they load.
        let order: Vec<String> = definition::builtins().into_iter().map(|agent| agent.name).collect();
        let mut entries: Vec<AgentEntry> = catalog
            .agents
            .iter()
            .map(|agent| AgentEntry {
                enabled: !off.contains(&agent.name),
                customised: !agent.builtin() && catalog.shadowed.iter().any(|shadowed| shadowed.agent.name == agent.name && shadowed.agent.builtin()),
                agent: agent.clone(),
            })
            .collect();
        entries.sort_by_key(|entry| order.iter().position(|name| *name == entry.agent.name).unwrap_or(order.len()));
        AgentsOverview { entries, problems: catalog.problems }
    }

    /// The role that answers to `name` now, for the editor.
    pub fn read_agent(&self, name: &str) -> Option<AgentDef> {
        definition::load(&self.agent_roots(), &HashSet::new()).agents.into_iter().find(|agent| agent.name == name)
    }

    /// The built-in named `name`, for the editor's Restore.
    pub fn builtin_agent(&self, name: &str) -> Option<AgentDef> {
        definition::builtins().into_iter().find(|agent| agent.name == name)
    }

    /// Saves a role as one of the user's files. `previous` is the name it had: a file of the
    /// user's by that name is renamed (its switch moves with it); otherwise, as for a built-in
    /// being customised, the file is new.
    pub fn save_agent(&mut self, previous: Option<&str>, draft: &AgentDraft) -> Result<PathBuf, String> {
        let name = draft.name.trim();
        let folder = self.agents_folder();
        let file = folder.join(format!("{name}.md"));
        let old = previous.filter(|previous| definition::check_name(previous).is_ok()).map(|previous| folder.join(format!("{previous}.md"))).filter(|old| old.is_file());
        let renaming = old.as_ref().is_some_and(|old| *old != file);
        if (old.is_none() || renaming) && fs::symlink_metadata(&file).is_ok() {
            return Err(format!("You already have an agent named {name}."));
        }
        let original = old.as_ref().and_then(|old| fs::read_to_string(old).ok());
        let text = draft.file_text(original.as_deref())?;
        fs::create_dir_all(&folder).map_err(|error| format!("Could not create the agents folder: {error}"))?;
        crate::skills::write_atomic(&file, &text)?;
        if let (Some(old), Some(previous)) = (old.filter(|_| renaming), previous) {
            let _ = fs::remove_file(&old);
            let config = self.agents_state_mut();
            if config.off.iter().any(|off| off == previous) {
                config.off.retain(|off| off != previous);
                config.off.push(name.to_string());
                self.save_state()?;
            }
        }
        Ok(file)
    }

    /// Moves one of the user's roles to the Trash with `trash`. For one that customised a
    /// built-in, the built-in answers to the name again.
    pub fn delete_agent(&mut self, name: &str, trash: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
        definition::check_name(name).map_err(|_| "That agent is not recognised.".to_string())?;
        let file = self.agents_folder().join(format!("{name}.md"));
        if fs::symlink_metadata(&file).is_err() {
            return Err("That agent could not be found. It may have been deleted already.".into());
        }
        trash(&file)?;
        if self.builtin_agent(name).is_none() && self.agents_config().off.iter().any(|off| off == name) {
            self.agents_state_mut().off.retain(|off| off != name);
            self.save_state()?;
        }
        Ok(())
    }

    pub fn set_agent_enabled(&mut self, name: &str, on: bool) -> Result<(), String> {
        let config = self.agents_state_mut();
        config.off.retain(|off| off != name);
        if !on {
            config.off.push(name.to_string());
        }
        self.save_state()
    }

    pub fn set_delegation(&mut self, delegation: Delegation) -> Result<(), String> {
        self.agents_state_mut().delegation = delegation;
        self.save_state()
    }

    /// The helper model, or `None` to run helper roles on the chat's model. A choice must name a
    /// model of a connection that is on.
    pub fn set_helper(&mut self, choice: Option<ModelChoice>) -> Result<(), String> {
        if let Some(choice) = &choice
            && self.resolve_exact(choice).is_none()
        {
            return Err("That model is no longer available. Choose another.".into());
        }
        self.agents_state_mut().helper = choice;
        self.save_state()
    }

    /// Switches another tool's agent folder (an id from [`TOOL_FOLDERS`]) on or off.
    pub fn set_agent_folder(&mut self, id: &str, on: bool) -> Result<(), String> {
        if !TOOL_FOLDERS.iter().any(|(known, _, _)| *known == id) {
            return Err("That folder is not one WackCode knows.".into());
        }
        let config = self.agents_state_mut();
        config.tool_folders.retain(|folder| folder != id);
        if on {
            config.tool_folders.push(id.to_string());
        }
        self.save_state()
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
                accepts_images: resolved.model.accepts_images,
                ultra: false,
                ultra_prompt: None,
                compaction_prompt: self.prompts().compaction.clone(),
                commit_attribution: false,
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
        Some(AgentsSetup {
            catalog: self.agent_catalog().agents,
            delegation: config.delegation,
            chat,
            helper,
            agents,
            child_prompt: self.prompts().subagent.clone(),
        })
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
        assert!(!chat.settings.commit_attribution);

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
    #[test]
    fn the_editor_customises_a_builtin_renames_and_restores() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = store(folder.path());
        let trash = |path: &Path| fs::remove_file(path).map_err(|error| error.to_string());

        // Editing the built-in writes the user's own file, which answers to the name now.
        let mut draft = AgentDraft::of(&store.read_agent("review").unwrap());
        draft.description = "Reviews the way the team does.".into();
        draft.tools = "read, grep bash".into();
        draft.max_turns = "30".into();
        store.save_agent(Some("review"), &draft).unwrap();
        let review = store.agents_overview().entries.into_iter().find(|entry| entry.agent.name == "review").unwrap();
        assert!(review.customised && review.yours());
        assert_eq!(review.agent.tools.as_deref(), Some(&["read".to_string(), "grep".into(), "bash".into()][..]));
        assert_eq!((review.agent.max_turns, review.agent.model), (30, ModelPref::Helper));

        // A new role can't take a name the user already has; a rename carries its switch.
        let mut docs = AgentDraft { name: "review".into(), description: "Docs".into(), prompt: "Check the docs.".into(), ..Default::default() };
        assert!(store.save_agent(None, &docs).unwrap_err().contains("already have"));
        docs.name = "docs".into();
        store.save_agent(None, &docs).unwrap();
        store.set_agent_enabled("docs", false).unwrap();
        docs.name = "docs-check".into();
        store.save_agent(Some("docs"), &docs).unwrap();
        assert_eq!(store.agents_config().off, ["docs-check"]);
        assert!(!store.agents_folder().join("docs.md").exists());

        // Restoring the built-in is trashing the user's file.
        store.delete_agent("review", &trash).unwrap();
        let review = store.agents_overview().entries.into_iter().find(|entry| entry.agent.name == "review").unwrap();
        assert!(review.agent.builtin() && !review.customised);
        assert!(store.delete_agent("explore", &trash).is_err(), "a built-in has no file to trash");
    }

    #[test]
    fn a_draft_is_checked_by_the_reader_that_loads_it() {
        let draft = |name: &str, turns: &str, prompt: &str| AgentDraft { name: name.into(), description: "Does it".into(), max_turns: turns.into(), prompt: prompt.into(), ..Default::default() };
        assert!(draft("Bad Name", "", "Go.").file_text(None).unwrap_err().starts_with("The name"));
        assert!(draft("ok", "0", "Go.").file_text(None).unwrap_err().starts_with("Steps"));
        assert!(draft("ok", "", " ").file_text(None).unwrap_err().contains("instructions"));
        let text = draft("ok", "", "Go.").file_text(None).unwrap();
        assert_eq!(text, "---\nname: ok\ndescription: Does it\nmodel: chat\n---\n\nGo.\n");
    }

    #[test]
    fn settings_switch_folders_and_refuse_a_helper_that_is_gone() {
        let folder = tempfile::tempdir().unwrap();
        let mut store = store(folder.path());
        assert!(store.set_agent_folder("nowhere", true).is_err());
        store.set_agent_folder("claude", true).unwrap();
        assert_eq!(store.agents_config().tool_folders, ["claude"]);
        let gone = ModelChoice { connection: Some("missing".into()), model: Some("big".into()), effort: None };
        assert!(store.set_helper(Some(gone)).is_err());
        store.set_delegation(Delegation::OnRequest).unwrap();
        assert_eq!(Store::open(folder.path()).unwrap().agents_config().delegation, Delegation::OnRequest);
    }
}
