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
    path::{Path, PathBuf},
    sync::Arc,
};

use wack_agent::{
    Agent, AgentMode, Control, Event, Kit, Outcome, Profile, Registry, Session, Settings,
    plan::{PLAN_STATE, PlanState},
};
use wack_llm::Provider;

use crate::store::{Chat, Store};

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
}

pub enum Job {
    /// Send the user's message and work until the model is done.
    Prompt(String),
    /// Summarise the older part of the conversation now.
    Compact,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Agent(Event),
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
        Ok(RunSetup {
            provider: Arc::new(provider),
            session_path: self.session_path(chat),
            cwd: project.path.clone(),
            settings: Settings {
                model: resolved.model.id.clone(),
                effort: resolved.effort.map(|reasoning| reasoning.effort()),
                max_output_tokens: None,
                context_window: resolved.model.context_window,
                ultra: resolved.effort.is_some_and(|reasoning| reasoning.is_ultra()),
            },
            mode: chat.mode,
            unrestricted_planning: self.planning_unrestricted(),
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
    let kit = Kit::new(&session, Some(setup.mode), setup.unrestricted_planning);
    let mut tools = wack_tools::core_tools();
    tools.extend(kit.tools());
    let registry = match Registry::new(tools) {
        Ok(registry) => Arc::new(registry),
        Err(message) => return finished(Err(message), None),
    };
    let mut agent = kit.attach(Agent::new(
        setup.provider,
        registry,
        session,
        &Profile::code(),
        setup.settings,
    ));
    let sink = |event: Event| events(RunEvent::Agent(event));
    let result = match job {
        Job::Prompt(prompt) => agent.run(&prompt, &control, &sink).await,
        Job::Compact => agent
            .compact(None, false, &control, &sink)
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

    use wack_agent::history;
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
                ultra: false,
            },
            mode: AgentMode::Build,
            unrestricted_planning: false,
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
    async fn a_failure_still_finishes() {
        let folder = tempfile::tempdir().unwrap();
        // Nothing to compact in an empty chat: the run reports why rather than hanging.
        let (seen, sink) = collect();
        run(
            setup(ScriptedProvider::new([]), folder.path()),
            Job::Compact,
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
        assert_eq!(names[7..], ["ask_user_question", "todo", "plan_mode_enter", "plan_mode_complete"]);

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
}
