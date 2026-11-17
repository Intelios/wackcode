//! A parent starting children end to end, with scripted models.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use wack_llm::testing::{ScriptedProvider, Step, call, say};

use super::*;
use crate::{
    Agent, AgentMode, Control, Event, Kit, ModelCall, Outcome, Profile, Purpose, Recorder, Registry, Session, Settings,
    tool::{Loading, Tool, ToolContext, ToolSpec},
};

/// A read-only tool that answers with what it was asked.
struct Look;

#[async_trait]
impl Tool for Look {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".into(),
            description: "Reads.".into(),
            input_schema: json!({ "type": "object" }),
            loading: Loading::Always,
            parallel_safe: true,
            hint: "Reads a file.".into(),
            guidelines: Vec::new(),
            ends_run: false,
        }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<String, String> {
        Ok(format!("contents of {}", input["path"].as_str().unwrap_or("?")))
    }
}

#[derive(Default)]
struct Calls(Mutex<Vec<ModelCall>>);

impl Recorder for Calls {
    fn record(&self, call: ModelCall) {
        self.0.lock().unwrap().push(call);
    }
}

fn settings(model: &str) -> Settings {
    Settings { model: model.into(), effort: None, max_output_tokens: None, context_window: None, ultra: false }
}

struct Parent {
    agent: Agent,
    provider: ScriptedProvider,
    events: Arc<Mutex<Vec<Event>>>,
    agents: Agents,
    folder: tempfile::TempDir,
}

/// A parent whose children run on `child`, in `mode`, with the built-in roles.
fn parent(steps: Vec<Step>, child: ScriptedProvider, mode: AgentMode, recorder: Option<Arc<dyn Recorder>>) -> Parent {
    let folder = tempfile::tempdir().unwrap();
    let session = Session::create(&folder.path().join("sessions/chat-1.jsonl"), folder.path()).unwrap();
    let agents = Agents::new();
    let setup = AgentsSetup {
        catalog: definition::builtins(),
        delegation: Delegation::Auto,
        chat: ChildModel { provider: Arc::new(child.clone()), settings: settings("chat-model"), recorder: recorder.clone() },
        helper: Some(ChildModel { provider: Arc::new(child), settings: settings("helper-model"), recorder: recorder.clone() }),
        agents: agents.clone(),
    };
    let kit = Kit::new(&session, Some(mode), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look)];
    tools.extend(kit.tools());
    tools.extend(kit.agent_tools(&tools));
    let provider = ScriptedProvider::new(steps);
    let agent = Agent::new(Arc::new(provider.clone()), Arc::new(Registry::new(tools).unwrap()), session, &Profile::code(), settings("parent-model"));
    let agent = match recorder {
        Some(recorder) => agent.with_recorder(recorder),
        None => agent,
    };
    Parent { agent: kit.attach(agent), provider, events: Arc::default(), agents, folder }
}

impl Parent {
    async fn run(&mut self, prompt: &str, control: &Control) -> Result<Outcome, String> {
        let events = self.events.clone();
        self.agent.run(prompt, control, &move |event| events.lock().unwrap().push(event)).await
    }

    fn results(&self) -> Vec<(String, bool)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::ToolResult { content, is_error, .. } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .collect()
    }
}

fn start(agent: &str, prompt: &str) -> Step {
    call(&[("p1", AGENT, json!({ "agent": agent, "description": "Find the loop", "prompt": prompt }))])
}

#[tokio::test]
async fn a_child_works_with_its_own_tools_and_its_report_comes_back() {
    let child = ScriptedProvider::new([call(&[("c1", "read", json!({ "path": "src/agent.rs" }))]), say("The loop is in src/agent.rs.")]);
    let calls = Arc::new(Calls::default());
    let mut parent = parent(vec![start("explore", "Where is the turn loop?"), say("It's in agent.rs.")], child.clone(), AgentMode::Build, Some(calls.clone()));
    assert_eq!(parent.run("Find the turn loop", &Control::new()).await, Ok(Outcome::Done));

    let (report, is_error) = &parent.results()[0];
    assert!(!is_error);
    assert!(report.starts_with("The loop is in src/agent.rs."), "{report}");
    assert!(report.contains("[agent a1 · explore · 1 tool call · "), "{report}");

    // The child ran on the helper model, with its brief as its only message and none of the
    // parent's own tools.
    let first = &child.requests()[0];
    assert_eq!(first.model, "helper-model");
    assert_eq!(first.messages[0].text(), "Where is the turn loop?");
    let tools: Vec<&str> = first.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(tools, ["read"]);
    assert!(first.system[0].text.starts_with("You are a sub-agent working inside WackCode"));
    assert!(first.system[0].text.contains("You are Explore"));
    // The parent saw the roster.
    let parent_tools: Vec<String> = parent.provider.requests()[0].tools.iter().map(|tool| tool.name.clone()).collect();
    assert!(parent_tools.contains(&AGENT.to_string()));

    // Its session sits beside the chat's, and the registry remembers it.
    let session = parent.folder.path().join("sessions/chat-1/a1.jsonl");
    assert!(session.exists());
    let info = &parent.agents.list()[0];
    assert_eq!((info.status.clone(), info.role.as_str(), info.call_id.as_str(), info.tool_calls), (Status::Done, "explore", "p1", 1));

    // The ledger knows which calls were the child's.
    let recorded: Vec<(Purpose, Option<String>, String)> =
        calls.0.lock().unwrap().iter().map(|call| (call.purpose, call.subagent_id.clone(), call.model.clone())).collect();
    assert_eq!(
        recorded,
        [
            (Purpose::Chat, None, "parent-model".into()),
            (Purpose::Subagent, Some("chat-1/a1".into()), "helper-model".into()),
            (Purpose::Subagent, Some("chat-1/a1".into()), "helper-model".into()),
            (Purpose::Chat, None, "parent-model".into()),
        ]
    );
}

#[tokio::test]
async fn plan_mode_starts_only_roles_that_make_no_edits() {
    let child = ScriptedProvider::new([say("Read it.")]);
    let mut parent = parent(
        vec![start("general", "Change it"), start("explore", "Look at it"), say("Planned.")],
        child,
        AgentMode::Plan,
        None,
    );
    parent.run("Plan a change", &Control::new()).await.unwrap();
    let results = parent.results();
    assert!(results[0].1 && results[0].0.contains("Plan mode only starts agents that make no edits (explore, review)"), "{:?}", results[0]);
    assert!(!results[1].1 && results[1].0.starts_with("Read it."));
}

#[tokio::test]
async fn a_child_that_will_not_stop_is_held_to_its_steps() {
    let looping = |id: &str| call(&[(id, "read", json!({ "path": "x" }))]);
    // Explore's limit is high; a role of the user's has a tiny one.
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("tiny.md"), "---\ndescription: Tiny\ntools: [read]\nmax_turns: 1\n---\nLook.").unwrap();
    let catalog = definition::load(&[crate::skills::Root { path: folder.path().into(), label: "Yours".into() }], &Default::default());
    let child = ScriptedProvider::new([looping("c1"), looping("c2"), looping("c3")]);
    let mut parent = parent(vec![start("tiny", "Look"), say("ok")], child.clone(), AgentMode::Build, None);
    // Swap in the catalogue with the tiny role.
    let setup = AgentsSetup {
        catalog: catalog.agents,
        delegation: Delegation::Auto,
        chat: ChildModel { provider: Arc::new(child.clone()), settings: settings("m"), recorder: None },
        helper: None,
        agents: parent.agents.clone(),
    };
    let session = Session::open(&parent.folder.path().join("sessions/chat-1.jsonl")).unwrap();
    let kit = Kit::new(&session, Some(AgentMode::Build), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look)];
    tools.extend(kit.agent_tools(&tools));
    parent.agent = Agent::new(Arc::new(parent.provider.clone()), Arc::new(Registry::new(tools).unwrap()), session, &Profile::code(), settings("p"));

    parent.run("Go", &Control::new()).await.unwrap();
    // It only ever called tools, so there is no report: the call fails, saying why.
    let (report, is_error) = &parent.results()[0];
    assert!(is_error);
    assert!(report.starts_with("It was stopped after using all 1 of its steps. It wrote no report."), "{report}");
    // After its one step every call was refused, telling it to report.
    let denied = child.requests()[2].messages.last().unwrap().clone();
    assert!(format!("{denied:?}").contains("You have used all 1 of your steps"));
    assert_eq!(parent.agents.list()[0].status, Status::Stopped { by: StoppedBy::Limit });
}

#[tokio::test]
async fn stopping_the_parent_stops_the_child_it_waits_for() {
    let child = ScriptedProvider::new([Step::Hang { text: "Looking".into() }]);
    let mut parent = parent(vec![start("explore", "Look forever")], child, AgentMode::Build, None);
    let control = Control::new();
    let stopper = control.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        stopper.stop();
    });
    assert_eq!(parent.run("Go", &control).await, Ok(Outcome::Stopped));
    let agents = parent.agents.clone();
    let child = agents.get("a1").unwrap();
    tokio::time::timeout(Duration::from_secs(5), child.ended()).await.expect("the child stopped");
    assert_eq!(agents.list()[0].status, Status::Stopped { by: StoppedBy::User });
    assert!(Path::new(&agents.list()[0].session).exists());
}
