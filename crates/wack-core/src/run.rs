//! Running the agent for one chat: a turn, or a compaction the user asked for.
//!
//! A run opens the chat's session file itself and owns it until it ends; nothing else writes to
//! that file meanwhile, and the UI reads it only through `wack_agent::history`, which never
//! writes. A new [`wack_agent::Agent`] is built for every run, so a model picked between turns
//! simply applies to the next one.
//!
//! The provider (and so the key) is built on the caller's side by [`Store::run_setup`] and moved
//! into the run: the run never sees the store.
//!
//! The chat's mode travels the same way: the store holds it, the run is told it, and the session
//! records what the model was told (`wack_agent::plan`). The two things done to a plan between
//! runs, discarding it and saving it as a file, are here too; both need the chat to be idle.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use wack_agent::{
    Agent, AgentMode, Control, Event, Kit, Outcome, Profile, Registry, Session, Settings, Tasks,
    commands::CommandUse,
    plan::{PLAN_STATE, PlanState},
    skills::{self, Root},
};
use wack_llm::Provider;

use crate::store::{Chat, Store};

/// Reports each model invocation to the usage ledger.
pub type Usage = std::sync::Arc<dyn wack_agent::Recorder>;

/// Everything a run needs, resolved before it starts.
pub struct RunSetup {
    pub provider: Arc<dyn Provider>,
    pub session_path: PathBuf,
    /// Where the project lives; a new session starts there.
    pub cwd: PathBuf,
    pub settings: Settings,
    /// Build, Plan or Ultra Plan, as the chat has it now.
    pub mode: AgentMode,
    /// Settings › Tools: planning may change things.
    pub unrestricted_planning: bool,
    /// The project's memory folder, or `None` when memory is off for it (Settings › Memory).
    pub memory: Option<PathBuf>,
    /// The folders skills load from and the skills switched off (Settings › Skills). They are
    /// read when the run starts, off the UI thread.
    pub skill_roots: Vec<Root>,
    pub disabled_skills: HashSet<PathBuf>,
    /// The chat's shell tasks, which outlive the run. `None` gives the run its own, which go
    /// with it.
    pub tasks: Option<Tasks>,
    /// Reports the run's model calls to the usage ledger.
    pub usage: Option<Usage>,
    /// Which web tools the run offers (Settings › Web).
    pub web: wack_web::WebSetup,
    /// The MCP servers this project's runs reach (Settings › MCP servers), with their secrets.
    pub mcp_servers: Vec<wack_mcp::ServerSpec>,
    /// The connections to them, which outlive the run. `None` offers no MCP tools; the app
    /// shares its own, and `wack run` makes one for the run.
    pub mcp_hub: Option<wack_mcp::Hub>,
    /// Sub-agents (Settings › Sub-agents), or `None` when they are off. Its registry is the
    /// chat's, which the app swaps in so children outlive the run.
    pub agents: Option<wack_agent::AgentsSetup>,
    /// The chat's browser tabs, which outlive the run. `None` offers no browser; the app hands
    /// the chat's own.
    pub browser: Option<wack_browser::ChatTabs>,
}

pub enum Job {
    /// Send the user's message and work until the model is done.
    Prompt(String),
    /// Send a message the user wrote as a slash command: `text` is what the model gets, already
    /// expanded, and `command` what they typed, kept for the transcript.
    Command { text: String, command: CommandUse },
    /// Summarise the older part of the conversation now, with an optional focus (`/compact`'s).
    Compact(Option<String>),
    /// Tell the model what its background tasks reported while the chat was idle, and let it
    /// carry on. No request is sent when there is nothing to tell.
    Wake,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Agent(Event),
    /// The run is waiting for these MCP servers to start; an empty list when it no longer is.
    McpStarting(Vec<String>),
    /// The run is over. `context_tokens` is how full the model's context now is, when the
    /// session could be read.
    Finished {
        result: Result<Outcome, String>,
        context_tokens: Option<u64>,
    },
}

impl Store {
    /// Resolves a chat's project and model into a run, building the provider that holds the key.
    pub fn run_setup(&self, chat: &Chat) -> Result<RunSetup, String> {
        let project = self
            .project(&chat.project)
            .ok_or("This chat's project is no longer in WackCode.")?;
        let resolved = self
            .resolve(&chat.choice)
            .ok_or("Add a connection in Settings › Providers to start chatting.")?;
        let provider = self.provider(&resolved.connection)?;
        let usage = self.usage_recorder(&resolved.connection, &chat.id, &project.path);
        let children = self.child_model(&resolved, &chat.id, &project.path)?;
        Ok(RunSetup {
            provider: Arc::new(provider),
            session_path: self.session_path(chat),
            cwd: project.path.clone(),
            settings: Settings {
                model: resolved.model.id.clone(),
                effort: resolved.model.effort_for(resolved.effort),
                max_output_tokens: None,
                context_window: resolved.model.context_window,
                accepts_images: resolved.model.accepts_images,
                ultra: resolved.effort.is_ultra(),
                commit_attribution: self.git_config().commit_attribution,
            },
            mode: chat.mode,
            unrestricted_planning: self.planning_unrestricted(),
            memory: self.memory_for(&project.path),
            skill_roots: self.skill_roots(),
            disabled_skills: self.disabled_skills(),
            tasks: None,
            usage: Some(usage),
            web: self.web_setup(),
            mcp_servers: self.mcp_specs_for(&project.path),
            mcp_hub: None,
            agents: self.agents_setup(children, &chat.id, &project.path, wack_agent::Agents::new()),
            browser: None,
        })
    }
}

/// Runs `job` to the end, reporting everything to `events`, the last event always being
/// [`RunEvent::Finished`].
pub async fn run(
    setup: RunSetup,
    job: Job,
    control: Control,
    events: Arc<dyn Fn(RunEvent) + Send + Sync>,
) {
    let finished = |result: Result<Outcome, String>, context_tokens: Option<u64>| {
        events(RunEvent::Finished {
            result,
            context_tokens,
        })
    };
    let session = if setup.session_path.exists() {
        Session::open(&setup.session_path)
    } else {
        Session::create(&setup.session_path, &setup.cwd)
    };
    let session = match session {
        Ok(session) => session,
        Err(message) => return finished(Err(message), None),
    };
    let catalog = skills::load(&setup.skill_roots, &setup.disabled_skills);
    let mut kit = Kit::new(&session, Some(setup.mode), setup.unrestricted_planning, setup.memory).with_skills(&catalog);
    if let Some(tasks) = setup.tasks {
        kit = kit.with_tasks(&session, tasks);
    }
    if let Some(agents) = setup.agents {
        // A message from the user lets sub-agents start again.
        if matches!(job, Job::Prompt(_) | Job::Command { .. }) {
            agents.agents.reset_budget();
        }
        kit = kit.with_agents(&session, agents);
    }
    let mut tools = wack_tools::core_tools();
    tools.extend(kit.tools());
    match wack_web::tools(&setup.web) {
        Ok(web) => tools.extend(web),
        Err(message) => return finished(Err(message), None),
    }
    // MCP servers: their tools, waiting only for a server with no tool list yet. A compaction
    // calls no tools, so it starts nothing.
    let mut mcp = wack_mcp::Assembly::default();
    if let Some(hub) = setup.mcp_hub.as_ref().filter(|_| !matches!(job, Job::Compact(_))) {
        let starting: wack_mcp::Starting = {
            let events = events.clone();
            Arc::new(move |servers| events(RunEvent::McpStarting(servers)))
        };
        mcp = wack_mcp::assemble(hub, &setup.mcp_servers, &setup.cwd, &control.stop_token(), starting).await;
    }
    tools.extend(mcp.tools);
    // Sub-agents last: each child gets its role's share of everything above.
    let agent_tools = kit.agent_tools(&tools);
    tools.extend(agent_tools);
    let registry = match Registry::new(tools) {
        Ok(registry) => Arc::new(registry),
        Err(message) => return finished(Err(message), None),
    };
    let agent = Agent::new(
        setup.provider,
        registry,
        session,
        &Profile::code(),
        setup.settings,
    );
    let agent = match setup.usage {
        Some(usage) => agent.with_recorder(usage),
        None => agent,
    };
    let agent = match mcp.feature {
        Some(feature) => agent.with_feature(Box::new(feature)),
        None => agent,
    };
    let mut agent = kit.attach(agent);
    let sink = |event: Event| events(RunEvent::Agent(event));
    let result = match job {
        Job::Prompt(prompt) => agent.run(&prompt, &control, &sink).await,
        Job::Command { text, command } => agent.run_as(&text, Some(command), &control, &sink).await,
        Job::Wake => agent.wake(&control, &sink).await,
        Job::Compact(focus) => agent
            .compact(focus.as_deref().filter(|focus| !focus.trim().is_empty()), false, &control, &sink)
            .await
            .map(|()| Outcome::Done),
    };
    finished(result, Some(agent.session().context_tokens()));
}

/// Abandons the plan awaiting review and returns the chat's session to Build. Call it only
/// while the chat is idle: a run owns the session file while it works.
pub fn discard_plan(session_path: &Path) -> Result<(), String> {
    let mut session = Session::open(session_path)?;
    let state = PlanState { mode: AgentMode::Build, plan: None };
    session.set_state(PLAN_STATE, state.to_value()).map(|_| ())
}

/// Writes a plan to `PLAN.md` in the project folder and returns where. An existing file is
/// never replaced.
pub fn save_plan(project: &Path, plan: &str) -> Result<PathBuf, String> {
    let path = project.join("PLAN.md");
    if path.exists() {
        return Err("PLAN.md already exists in this project. Rename or remove it first.".to_string());
    }
    std::fs::write(&path, format!("{plan}\n")).map_err(|error| format!("Could not write PLAN.md: {error}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use wack_agent::{HistoryEntry, history};
    use wack_llm::testing::{ScriptedProvider, say};

    use super::*;

    type Sink = Arc<dyn Fn(RunEvent) + Send + Sync>;

    fn setup(provider: ScriptedProvider, folder: &std::path::Path) -> RunSetup {
        RunSetup {
            provider: Arc::new(provider),
            session_path: folder.join("sessions/chat.jsonl"),
            cwd: folder.to_path_buf(),
            settings: Settings {
                model: "m".into(),
                effort: None,
                max_output_tokens: None,
                context_window: None,
                accepts_images: true,
                ultra: false,
                commit_attribution: false,
            },
            mode: AgentMode::Build,
            unrestricted_planning: false,
            memory: None,
            skill_roots: Vec::new(),
            disabled_skills: HashSet::new(),
            tasks: None,
            usage: None,
            web: wack_web::WebSetup::default(),
            mcp_servers: Vec::new(),
            mcp_hub: None,
            agents: None,
            browser: None,
        }
    }

    fn collect() -> (Arc<Mutex<Vec<RunEvent>>>, Sink) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            Arc::new(move |event| seen.lock().unwrap().push(event))
                as Sink
        };
        (seen, sink)
    }

    #[tokio::test]
    async fn a_second_run_continues_the_same_chat() {
        let folder = tempfile::tempdir().unwrap();
        let provider = ScriptedProvider::new([say("hello"), say("again")]);

        let (seen, sink) = collect();
        run(
            setup(provider.clone(), folder.path()),
            Job::Prompt("hi".into()),
            Control::new(),
            sink.clone(),
        )
        .await;
        run(
            setup(provider.clone(), folder.path()),
            Job::Prompt("more".into()),
            Control::new(),
            sink,
        )
        .await;

        let finished: Vec<_> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                RunEvent::Finished {
                    result,
                    context_tokens,
                } => Some((result.clone(), context_tokens.is_some())),
                _ => None,
            })
            .collect();
        assert_eq!(
            finished,
            [(Ok(Outcome::Done), true), (Ok(Outcome::Done), true)]
        );
        // The second request carried the whole conversation.
        assert_eq!(provider.requests()[1].messages.len(), 3);
        assert_eq!(
            history(&folder.path().join("sessions/chat.jsonl"))
                .unwrap()
                .len(),
            4
        );
    }

    #[tokio::test]
    async fn a_command_keeps_what_the_user_typed_and_skills_reach_the_prompt() {
        let folder = tempfile::tempdir().unwrap();
        let skills = folder.path().join("skills/pdf");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("SKILL.md"), "---\nname: pdf\ndescription: Fill PDF forms\n---\nBody").unwrap();
        let provider = ScriptedProvider::new([say("done")]);
        let mut setup = setup(provider.clone(), folder.path());
        setup.skill_roots = vec![Root { path: folder.path().join("skills"), label: "Your skills".into() }];
        let command = CommandUse { name: "review".into(), args: "main.rs".into(), kind: wack_agent::commands::Kind::Template };
        let (_, sink) = collect();
        run(setup, Job::Command { text: "Review main.rs.".into(), command: command.clone() }, Control::new(), sink).await;

        let request = &provider.requests()[0];
        assert!(request.system.iter().any(|part| part.text.contains("<name>pdf</name>")));
        assert_eq!(request.messages[0].text(), "Review main.rs.");
        let entries = history(&folder.path().join("sessions/chat.jsonl")).unwrap();
        assert!(matches!(&entries[0], HistoryEntry::Message { command: Some(used), .. } if *used == command));
    }

    #[tokio::test]
    async fn a_failure_still_finishes() {
        let folder = tempfile::tempdir().unwrap();
        // Nothing to compact in an empty chat: the run reports why rather than hanging.
        let (seen, sink) = collect();
        run(
            setup(ScriptedProvider::new([]), folder.path()),
            Job::Compact(None),
            Control::new(),
            sink,
        )
        .await;
        let events = seen.lock().unwrap();
        assert!(
            matches!(events.last(), Some(RunEvent::Finished { result: Err(message), .. }) if message.contains("nothing to compact"))
        );
    }

    #[tokio::test]
    async fn a_plan_is_submitted_then_discarded_or_saved() {
        use wack_agent::{HistoryEntry, plan::PLAN_MODE_COMPLETE};
        use wack_llm::testing::call;

        let folder = tempfile::tempdir().unwrap();
        let plan = serde_json::json!({ "plan": "# Fix\n\n1. Do it" });
        let provider = ScriptedProvider::new([call(&[("a", PLAN_MODE_COMPLETE, plan)])]);
        let mut planning = setup(provider.clone(), folder.path());
        planning.mode = AgentMode::Plan;
        let session_path = planning.session_path.clone();
        let (seen, sink) = collect();
        run(planning, Job::Prompt("Plan it".into()), Control::new(), sink).await;

        // The run ended on the submission, and the app was told the plan is waiting.
        let waiting = PlanState { mode: AgentMode::Plan, plan: Some("# Fix\n\n1. Do it".into()) };
        assert!(seen.lock().unwrap().contains(&RunEvent::Agent(Event::State { key: PLAN_STATE.into(), value: waiting.to_value() })));
        assert_eq!(provider.requests().len(), 1);
        let names: Vec<String> = provider.requests()[0].tools.iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names[10..], ["ask_user_question", "todo", "plan_mode_enter", "plan_mode_complete", "suggest_task"]);

        // With Web Fetch on, the tool waits behind tool_search, after everything always loaded.
        let provider = ScriptedProvider::new([say("ok")]);
        let mut fetching = setup(provider.clone(), folder.path());
        fetching.web.fetch = true;
        let (_, sink) = collect();
        run(fetching, Job::Prompt("hi".into()), Control::new(), sink).await;
        let names: Vec<String> = provider.requests()[0].tools.iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names.last().map(String::as_str), Some(wack_agent::TOOL_SEARCH));
        assert!(!names.iter().any(|name| name == wack_web::WEB_FETCH));

        let saved = save_plan(folder.path(), waiting.plan.as_deref().unwrap()).unwrap();
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), "# Fix\n\n1. Do it\n");
        assert!(save_plan(folder.path(), "again").unwrap_err().contains("already exists"));

        discard_plan(&session_path).unwrap();
        let last = history(&session_path).unwrap().into_iter().rev().find_map(|entry| match entry {
            HistoryEntry::State { key, value } if key == PLAN_STATE => PlanState::from_value(&value),
            _ => None,
        });
        assert_eq!(last, Some(PlanState::default()));
    }

    #[test]
    fn run_setup_propagates_commit_attribution() {
        let folder = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let mut store = Store::open(folder.path()).unwrap();
        store
            .add_connection(
                crate::NewConnection {
                    name: "Testing".into(),
                    format: wack_llm::WireFormat::OpenaiChat,
                    base_url: "https://example.invalid/v1".into(),
                    models: vec![crate::Model::new("zai-org/GLM-5.3-Flash", None)],
                },
                "k",
            )
            .unwrap();
        let project = store.add_project(work.path()).unwrap();
        let chat = store
            .create_chat(&project.id, "Commit", crate::ModelChoice::default(), AgentMode::Build, None)
            .unwrap();

        assert!(store.run_setup(&chat).unwrap().settings.commit_attribution);
        store.set_git_config(crate::GitConfig { commit_attribution: false }).unwrap();
        assert!(!store.run_setup(&chat).unwrap().settings.commit_attribution);
    }
}
