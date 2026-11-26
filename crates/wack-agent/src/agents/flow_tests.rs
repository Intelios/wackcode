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
    tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec},
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
            together: false,
        }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        Ok(format!("contents of {}", input["path"].as_str().unwrap_or("?")).into())
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
    Settings {
        model: model.into(),
        effort: None,
        max_output_tokens: None,
        context_window: None,
        accepts_images: true,
        ultra: false,
        ultra_prompt: None,
        compaction_prompt: None,
        commit_attribution: false,
    }
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
        child_prompt: None,
        crew: None,
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
        child_prompt: None,
        crew: None,
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

/// Waits until the test opens it, for children that must still be working at a given moment.
struct Gate(Arc<tokio::sync::Semaphore>);

#[async_trait]
impl Tool for Gate {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "gate".into(), ..Look.spec() }
    }

    async fn run(&self, _input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let _pass = self.0.acquire().await.map_err(|_| "closed".to_string())?;
        Ok("through".into())
    }
}

/// Passes only once two calls are waiting in it at the same time.
struct Meet(Arc<tokio::sync::Barrier>);

#[async_trait]
impl Tool for Meet {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "meet".into(), ..Look.spec() }
    }

    async fn run(&self, _input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        tokio::time::timeout(Duration::from_secs(5), self.0.wait()).await.map(|_| "met".into()).map_err(|_| "alone".to_string())
    }
}

/// A parent with extra tools its children can use, and a role of the user's that may use them.
fn parent_with(steps: Vec<Step>, child: ScriptedProvider, extra: Vec<Arc<dyn Tool>>) -> Parent {
    let mut parent = parent(steps, child.clone(), AgentMode::Build, None);
    let roles = tempfile::tempdir().unwrap();
    std::fs::write(roles.path().join("helper.md"), "---\ndescription: Helps\ntools: [read, gate, meet]\n---\nHelp.").unwrap();
    let catalog = definition::load(&[crate::skills::Root { path: roles.path().into(), label: "Yours".into() }], &Default::default());
    let setup = AgentsSetup {
        catalog: catalog.agents,
        delegation: Delegation::Auto,
        chat: ChildModel { provider: Arc::new(child), settings: settings("m"), recorder: None },
        helper: None,
        agents: parent.agents.clone(),
        child_prompt: None,
        crew: None,
    };
    let session = Session::open(&parent.folder.path().join("sessions/chat-1.jsonl")).unwrap();
    let kit = Kit::new(&session, Some(AgentMode::Build), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look)];
    tools.extend(extra);
    tools.extend(kit.agent_tools(&tools));
    let agent = Agent::new(Arc::new(parent.provider.clone()), Arc::new(Registry::new(tools).unwrap()), session, &Profile::code(), settings("p"));
    parent.agent = kit.attach(agent);
    parent
}

fn helper(id: &str, prompt: &str, background: bool) -> (String, String, Value) {
    let mut input = json!({ "agent": "helper", "description": prompt, "prompt": prompt });
    if background {
        input["background"] = json!(true);
    }
    (id.to_string(), AGENT.to_string(), input)
}

fn calls(list: &[(String, String, Value)]) -> Step {
    let borrowed: Vec<(&str, &str, Value)> = list.iter().map(|(id, name, input)| (id.as_str(), name.as_str(), input.clone())).collect();
    call(&borrowed)
}

#[tokio::test]
async fn agents_started_in_one_reply_work_at_the_same_time() {
    // Each child waits for the other in `meet`: run one after the other, both would be alone.
    let child = ScriptedProvider::new([
        call(&[("m1", "meet", json!({}))]),
        call(&[("m2", "meet", json!({}))]),
        say("first done"),
        say("second done"),
    ]);
    let meet: Arc<dyn Tool> = Arc::new(Meet(Arc::new(tokio::sync::Barrier::new(2))));
    let mut parent = parent_with(vec![calls(&[helper("p1", "one", false), helper("p2", "two", false)]), say("both")], child, vec![meet]);
    parent.run("Go", &Control::new()).await.unwrap();
    let results = parent.results();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(text, is_error)| !is_error && text.contains("done")), "{results:?}");
    let saved = std::fs::read_to_string(parent.folder.path().join("sessions/chat-1/a1.jsonl")).unwrap();
    assert!(saved.contains("met") && !saved.contains("alone"));
}

#[tokio::test]
async fn a_background_report_arrives_as_a_note_and_wakes_the_parent() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let child = ScriptedProvider::new([call(&[("g", "gate", json!({}))]), say("Found it.")]);
    let mut parent = parent_with(
        vec![calls(&[helper("p1", "look", true)]), say("It's working; I'll report back."), say("The agent found it.")],
        child,
        vec![Arc::new(Gate(gate.clone()))],
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let (started, is_error) = &parent.results()[0];
    assert!(!is_error && started.starts_with("Agent a1 (helper) is working in the background."));
    assert_eq!(parent.agents.active().len(), 1);

    gate.add_permits(1);
    parent.agents.get("a1").unwrap().ended().await;
    assert!(parent.agents.wakes());
    assert_eq!(parent.agent.wake(&Control::new(), &|_| {}).await, Ok(Outcome::Done));
    let last = parent.provider.requests().last().unwrap().clone();
    let note = last.messages.iter().rev().find(|message| message.text().contains("finished in the background")).unwrap().text();
    assert!(note.contains("Agent a1 (helper · look) finished in the background. Its report:\n\nFound it."), "{note}");
    assert!(!parent.agents.has_news(), "told once");
}

#[tokio::test]
async fn the_user_can_move_a_waited_for_agent_to_the_background() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let child = ScriptedProvider::new([call(&[("g", "gate", json!({}))]), say("Done at last.")]);
    let mut parent = parent_with(vec![calls(&[helper("p1", "slow", false)]), say("ok")], child, vec![Arc::new(Gate(gate.clone()))]);
    let agents = parent.agents.clone();
    tokio::spawn(async move {
        while agents.active().first().is_none_or(|info| info.tool_calls == 0) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(agents.background_foreground());
    });
    parent.run("Go", &Control::new()).await.unwrap();
    let (moved, _) = &parent.results()[0];
    assert!(moved.starts_with("The user moved agent a1 (helper) to the background"), "{moved}");
    gate.add_permits(1);
    parent.agents.get("a1").unwrap().ended().await;
    assert!(parent.agents.take_news()[0].text.contains("Done at last."));
}

#[tokio::test]
async fn past_six_at_once_agents_queue_and_stop_all_reaches_the_queue() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let steps: Vec<Step> = (0..6).map(|index| call(&[(&format!("g{index}"), "gate", json!({}))])).collect();
    let child = ScriptedProvider::new(steps);
    let starts: Vec<(String, String, Value)> = (0..7).map(|index| helper(&format!("p{index}"), "wait", true)).collect();
    let mut parent = parent_with(vec![calls(&starts), say("All started.")], child, vec![Arc::new(Gate(gate))]);
    parent.run("Go", &Control::new()).await.unwrap();
    let agents = parent.agents.clone();
    tokio::time::timeout(Duration::from_secs(5), async {
        while agents.list().iter().filter(|info| info.status == Status::Running && info.tool_calls == 1).count() < 6 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(agents.list()[6].status, Status::Queued);

    agents.stop_all();
    for info in agents.list() {
        agents.get(&info.id).unwrap().ended().await;
    }
    assert!(agents.list().iter().all(|info| info.status == Status::Stopped { by: StoppedBy::User }));
    assert!(!agents.wakes(), "the user stopped them: their reports don't wake the chat");
    assert_eq!(agents.take_news().len(), 7, "each stopped agent still reports once");
}

#[tokio::test]
async fn waiting_for_tasks_also_waits_for_an_agent_in_the_background() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let child = ScriptedProvider::new([call(&[("g", "gate", json!({}))]), say("Found it.")]);
    let mut parent = parent_with(
        vec![
            calls(&[helper("p1", "look", true)]),
            call(&[("w", crate::tasks::WAIT_FOR_TASKS, json!({}))]),
            say("It found it."),
        ],
        child,
        vec![Arc::new(Gate(gate.clone())), Arc::new(crate::tasks::WaitForTasks)],
    );
    let opener = gate.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        opener.add_permits(1);
    });
    assert_eq!(parent.run("Go", &Control::new()).await, Ok(Outcome::Done));
    let (waited, is_error) = &parent.results()[1];
    assert!(!is_error, "{waited}");
    assert!(waited.contains("Agent a1 (helper · look) finished in the background. Its report:\n\nFound it."), "{waited}");
    assert!(!parent.agents.has_news(), "handed over once, by the wait");
}

#[tokio::test]
async fn a_finished_agent_works_another_round_with_what_it_learned() {
    let child = ScriptedProvider::new([say("The loop is in agent.rs."), say("It compacts at 80%.")]);
    let mut parent = parent_with(
        vec![
            calls(&[helper("p1", "Find the loop", false)]),
            call(&[("p2", AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "When does it compact?" }))]),
            say("Thanks."),
        ],
        child.clone(),
        Vec::new(),
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let (followed, is_error) = &parent.results()[1];
    assert!(!is_error && followed.starts_with("It compacts at 80%."), "{followed}");
    // The second round continued the same session: its request carried the first round.
    let second = &child.requests()[1];
    let texts: Vec<String> = second.messages.iter().map(|message| message.text()).collect();
    assert_eq!(texts, ["Find the loop", "The loop is in agent.rs.", "When does it compact?"]);
    let info = &parent.agents.list()[0];
    assert_eq!((info.round, info.round_call_id.as_str(), info.call_id.as_str()), (2, "p2", "p1"));
}

#[tokio::test]
async fn a_message_steers_a_working_agent_and_unknown_ids_are_explained() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let child = ScriptedProvider::new([call(&[("g", "gate", json!({}))]), say("Checked both.")]);
    let mut parent = parent_with(
        vec![
            calls(&[helper("p1", "Check", true)]),
            call(&[("p2", AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "Also check the tests." }))]),
            call(&[("p3", AGENT_MESSAGE, json!({ "agent_id": "a9", "message": "Hello?" }))]),
            say("Waiting."),
        ],
        child.clone(),
        vec![Arc::new(Gate(gate.clone()))],
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let results = parent.results();
    assert!(results[1].0.starts_with("Sent to agent a1, which is still working"), "{:?}", results[1]);
    assert!(results[2].1 && results[2].0.contains("There is no agent a9 in this chat. Its agents: a1."));
    gate.add_permits(1);
    parent.agents.get("a1").unwrap().ended().await;
    // The message rode along with the gate's result.
    let after_gate = child.requests()[1].messages.last().unwrap().clone();
    assert!(format!("{after_gate:?}").contains("Also check the tests."));
}

#[tokio::test]
async fn a_copy_of_the_conversation_sends_exactly_what_the_parent_sends() {
    let child = ScriptedProvider::new([say("Side task done.")]);
    let mut parent = parent_with(
        vec![
            call(&[
                ("p1", AGENT, json!({ "description": "Side task", "prompt": "Rename the helper.", "with_conversation": true })),
                ("p2", "read", json!({ "path": "x" })),
            ]),
            say("Both done."),
        ],
        child.clone(),
        Vec::new(),
    );
    parent.run("Do the main thing", &Control::new()).await.unwrap();
    // The two calls ran side by side, so the read may answer first.
    let results = parent.results();
    assert!(results.iter().any(|(text, is_error)| !is_error && text.starts_with("Side task done.")), "{results:?}");

    let parent_request = &parent.provider.requests()[0];
    let copy_request = &child.requests()[0];
    // The cached prefix is the same: instructions, tools, model, and the conversation so far.
    assert_eq!(copy_request.system, parent_request.system);
    assert_eq!(copy_request.tools, parent_request.tools);
    assert_eq!(copy_request.model, parent_request.model);
    assert_eq!(copy_request.messages[..parent_request.messages.len()], parent_request.messages[..]);
    // Then the parent's reply, every call in it answered, and the directive.
    let opening = copy_request.messages.last().unwrap();
    let text = format!("{opening:?}");
    assert!(text.contains("This call started you."));
    assert!(text.contains("This call is your parent's, not yours"));
    assert!(text.contains("Rename the helper."));
    assert_eq!(parent.agents.list()[0].role, super::tools::COPY_ROLE);
}

#[tokio::test]
async fn a_copy_cannot_use_the_parents_own_tools() {
    let child = ScriptedProvider::new([
        call(&[("c1", AGENT, json!({ "agent": "helper", "description": "Nested", "prompt": "Go deeper" }))]),
        say("Did it myself."),
    ]);
    let mut parent = parent_with(
        vec![call(&[("p1", AGENT, json!({ "description": "Side", "prompt": "Do it", "with_conversation": true }))]), say("ok")],
        child.clone(),
        Vec::new(),
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let refused = format!("{:?}", child.requests()[1].messages.last().unwrap());
    assert!(refused.contains("As a sub-agent you can't use agent"), "{refused}");
    assert_eq!(parent.agents.list().len(), 1, "it started no agent of its own");
}

#[tokio::test]
async fn an_agent_whose_role_was_switched_off_cannot_work_another_round() {
    let mut parent = parent(
        vec![call(&[("p1", AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "More, please." }))]), say("ok")],
        ScriptedProvider::new([]),
        AgentMode::Build,
        None,
    );
    parent.agents.restore(vec![AgentInfo {
        id: "a1".into(),
        role: "helper".into(),
        description: "Help".into(),
        call_id: "old".into(),
        round: 1,
        round_call_id: String::new(),
        session: parent.folder.path().join("sessions/chat-1/a1.jsonl"),
        model: "m".into(),
        edits: false,
        started_at: 0,
        ended_at: Some(1),
        status: Status::Done,
        background: false,
        tool_calls: 0,
        context_tokens: 0,
        activity: None,
        tier: None,
        files: Vec::new(),
        changed: Vec::new(),
        spent: Default::default(),
    }]);
    parent.run("Go", &Control::new()).await.unwrap();
    let (refused, is_error) = &parent.results()[0];
    assert!(*is_error && refused.contains("Agent a1's role, helper, is no longer switched on"), "{refused}");
}

#[tokio::test]
async fn the_user_can_follow_up_from_the_pane_and_its_report_is_quiet() {
    let child = ScriptedProvider::new([say("First report."), say("Second report.")]);
    let mut parent = parent_with(vec![calls(&[helper("p1", "Look", false)]), say("ok")], child.clone(), Vec::new());
    parent.run("Go", &Control::new()).await.unwrap();

    let sent = parent.agents.message_from_user("a1", "Look at the tests too.").await.unwrap();
    assert!(sent.contains("working in the background"), "{sent}");
    parent.agents.get("a1").unwrap().ended().await;
    let asked = child.requests()[1].messages.last().unwrap().text();
    assert!(asked.starts_with("The user, who can see your work in WackCode, sends you this message:\n\nLook at the tests too."));
    assert!(parent.agents.has_news() && !parent.agents.wakes(), "it rides along without waking the chat");
    assert_eq!(parent.agents.list()[0].round_call_id, "");
}

#[tokio::test]
async fn an_agent_the_app_already_restored_is_still_announced_as_interrupted() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("chat.jsonl");
    let mut session = Session::create(&path, folder.path()).unwrap();
    let working = AgentInfo {
        id: "a1".into(),
        role: "explore".into(),
        description: "Look".into(),
        call_id: "c".into(),
        round: 1,
        round_call_id: String::new(),
        session: folder.path().join("chat/a1.jsonl"),
        model: "m".into(),
        edits: false,
        started_at: 0,
        ended_at: None,
        status: Status::Running,
        background: true,
        tool_calls: 3,
        context_tokens: 0,
        activity: None,
        tier: None,
        files: Vec::new(),
        changed: Vec::new(),
        spent: Default::default(),
    };
    session.set_state(AGENTS_STATE, serde_json::to_value(vec![working.clone()]).unwrap()).unwrap();
    // The app restores the list as the chat opens, before any run.
    let agents = Agents::new();
    agents.restore(vec![working]);
    let feature = AgentsFeature::new(&session, agents);
    let notes = crate::agent::Feature::notes(&feature, &session);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].text.contains("was stopped when WackCode quit") && notes[0].text.contains("agent_message"));
    assert!(crate::agent::Feature::notes(&feature, &session).is_empty(), "told once");
}

#[tokio::test]
async fn fresh_subagents_omit_commit_attribution_while_conversation_copies_keep_the_parents_prompt() {
    let folder = tempfile::tempdir().unwrap();
    let session = Session::create(&folder.path().join("sessions/chat-1.jsonl"), folder.path()).unwrap();
    let agents = Agents::new();
    let child = ScriptedProvider::new([say("Explored."), say("Copied.")]);
    let parent_provider = ScriptedProvider::new([
        call(&[("p1", AGENT, json!({ "agent": "explore", "description": "Explore", "prompt": "Look" }))]),
        call(&[("p2", AGENT, json!({ "description": "Copy", "prompt": "Commit", "with_conversation": true }))]),
        say("Done."),
    ]);
    let setup = AgentsSetup {
        catalog: definition::builtins(),
        delegation: Delegation::Auto,
        chat: ChildModel { provider: Arc::new(child.clone()), settings: settings("chat-model"), recorder: None },
        helper: None,
        agents: agents.clone(),
        child_prompt: None,
        crew: None,
    };
    let kit = Kit::new(&session, Some(AgentMode::Build), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look)];
    tools.extend(kit.tools());
    tools.extend(kit.agent_tools(&tools));
    let parent_settings = Settings { commit_attribution: true, ..settings("parent-model") };
    let agent = Agent::new(
        Arc::new(parent_provider.clone()),
        Arc::new(Registry::new(tools).unwrap()),
        session,
        &Profile::code(),
        parent_settings,
    );
    let mut agent = kit.attach(agent);
    assert_eq!(agent.run("Go", &Control::new(), &|_| {}).await, Ok(Outcome::Done));

    let fresh_child = &child.requests()[0];
    assert!(!fresh_child.system.iter().any(|part| part.text.contains("Commit attribution")));

    let copy_child = &child.requests()[1];
    assert_eq!(copy_child.system, parent_provider.requests()[0].system);
    assert!(copy_child.system[1].text.contains("Co-authored-by: parent-model (WackCode)"));
}

#[tokio::test]
async fn custom_child_prompt_reaches_fresh_children_while_conversation_copies_keep_the_parents() {
    let folder = tempfile::tempdir().unwrap();
    let session = Session::create(&folder.path().join("sessions/chat-1.jsonl"), folder.path()).unwrap();
    let agents = Agents::new();
    let child = ScriptedProvider::new([say("Explored."), say("Copied.")]);
    let parent_provider = ScriptedProvider::new([
        call(&[("p1", AGENT, json!({ "agent": "explore", "description": "Explore", "prompt": "Look" }))]),
        call(&[("p2", AGENT, json!({ "description": "Copy", "prompt": "Finish", "with_conversation": true }))]),
        say("Done."),
    ]);
    let setup = AgentsSetup {
        catalog: definition::builtins(),
        delegation: Delegation::Auto,
        chat: ChildModel { provider: Arc::new(child.clone()), settings: settings("chat-model"), recorder: None },
        helper: None,
        agents: agents.clone(),
        child_prompt: Some("Custom sub-agent core rules.".into()),
        crew: None,
    };
    let kit = Kit::new(&session, Some(AgentMode::Build), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look)];
    tools.extend(kit.tools());
    tools.extend(kit.agent_tools(&tools));
    let mut agent = kit.attach(Agent::new(
        Arc::new(parent_provider.clone()),
        Arc::new(Registry::new(tools).unwrap()),
        session,
        &Profile::code_with(Some("Custom parent core.")),
        settings("parent-model"),
    ));
    assert_eq!(agent.run("Go", &Control::new(), &|_| {}).await, Ok(Outcome::Done));

    let fresh_child = &child.requests()[0];
    assert!(fresh_child.system[0].text.starts_with("Custom sub-agent core rules.\n\n# Your role\n\nYou are Explore"));

    let copy_child = &child.requests()[1];
    assert_eq!(copy_child.system, parent_provider.requests()[0].system);
    assert_eq!(copy_child.system[0].text, "Custom parent core.");
}

/// A tool named `write` that succeeds unless its path says `bad`.
struct Write;

#[async_trait]
impl Tool for Write {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: "write".into(), parallel_safe: false, ..Look.spec() }
    }

    async fn run(&self, input: Value, _context: &ToolContext<'_>) -> Result<ToolOutput, String> {
        match input["path"].as_str().unwrap_or_default() {
            path if path.contains("bad") => Err("could not write".into()),
            path => Ok(format!("wrote {path}").into()),
        }
    }
}

/// A parent with the built-in roles whose chat runs on `chat-model`, helper roles on
/// `helper-model` and, when `team`, a Team crew on `crew-model`. Children get `write` too.
fn team_parent(steps: Vec<Step>, child: ScriptedProvider, team: bool) -> Parent {
    let folder = tempfile::tempdir().unwrap();
    let session = Session::create(&folder.path().join("sessions/chat-1.jsonl"), folder.path()).unwrap();
    let agents = Agents::new();
    let model = |name: &str| ChildModel { provider: Arc::new(child.clone()), settings: settings(name), recorder: None };
    let setup = AgentsSetup {
        catalog: definition::builtins(),
        delegation: Delegation::Auto,
        chat: model("chat-model"),
        helper: Some(model("helper-model")),
        agents: agents.clone(),
        child_prompt: None,
        crew: team.then(|| model("crew-model")),
    };
    let kit = Kit::new(&session, Some(AgentMode::Build), false, None).with_agents(&session, setup);
    let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Look), Arc::new(Write)];
    tools.extend(kit.tools());
    tools.extend(kit.agent_tools(&tools));
    let provider = ScriptedProvider::new(steps);
    let agent = Agent::new(Arc::new(provider.clone()), Arc::new(Registry::new(tools).unwrap()), session, &Profile::code(), settings("chat-model"));
    Parent { agent: kit.attach(agent), provider, events: Arc::default(), agents, folder }
}

fn task(id: &str, agent: &str, extra: Value) -> Step {
    let mut input = json!({ "agent": agent, "description": format!("{agent} task"), "prompt": format!("Do the {agent} part.") });
    if let (Some(input), Some(extra)) = (input.as_object_mut(), extra.as_object()) {
        input.extend(extra.clone());
    }
    call(&[(id, AGENT, input)])
}

#[tokio::test]
async fn in_a_team_roles_run_on_the_crew_and_review_on_the_lead() {
    let child = ScriptedProvider::new([say("explored"), say("reviewed"), say("built"), say("hard part done")]);
    let mut parent = team_parent(
        vec![
            task("p1", "explore", json!({})),
            task("p2", "review", json!({})),
            task("p3", "general", json!({})),
            task("p4", "general", json!({ "model": "lead" })),
            say("All done."),
        ],
        child.clone(),
        true,
    );
    parent.run("Build it", &Control::new()).await.unwrap();
    let models: Vec<String> = child.requests().iter().map(|request| request.model.clone()).collect();
    assert_eq!(models, ["crew-model", "chat-model", "crew-model", "chat-model"]);
    let tiers: Vec<Option<Tier>> = parent.agents.list().iter().map(|info| info.tier).collect();
    assert_eq!(tiers, [Some(Tier::Crew), Some(Tier::Lead), Some(Tier::Crew), Some(Tier::Lead)]);
}

#[tokio::test]
async fn outside_a_team_the_crew_is_the_helper_and_roles_keep_their_own_models() {
    let child = ScriptedProvider::new([say("explored"), say("built"), say("built cheaply")]);
    let mut parent = team_parent(
        vec![task("p1", "explore", json!({})), task("p2", "general", json!({})), task("p3", "general", json!({ "model": "crew" })), say("ok")],
        child.clone(),
        false,
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let models: Vec<String> = child.requests().iter().map(|request| request.model.clone()).collect();
    assert_eq!(models, ["helper-model", "chat-model", "helper-model"]);
    // Nothing about a team reached anyone.
    assert!(!format!("{:?}", parent.provider.requests()).contains(TEAM_MARKER));
    assert_eq!(child.requests()[1].messages[0].text(), "Do the general part.");
}

#[tokio::test]
async fn a_crew_worker_stays_on_the_crew_for_another_round_and_a_copy_refuses_the_crew() {
    let child = ScriptedProvider::new([say("First round."), say("Second round.")]);
    let mut parent = team_parent(
        vec![
            task("p1", "general", json!({})),
            call(&[("p2", AGENT_MESSAGE, json!({ "agent_id": "a1", "message": "Fix the edge case." }))]),
            call(&[("p3", AGENT, json!({ "description": "Side", "prompt": "Do it", "with_conversation": true, "model": "crew" }))]),
            call(&[("p4", AGENT, json!({ "agent": "general", "description": "Odd", "prompt": "Do it", "model": "boss" }))]),
            say("ok"),
        ],
        child.clone(),
        true,
    );
    parent.run("Go", &Control::new()).await.unwrap();
    let models: Vec<String> = child.requests().iter().map(|request| request.model.clone()).collect();
    assert_eq!(models, ["crew-model", "crew-model"]);
    let results = parent.results();
    assert!(results[2].1 && results[2].0.contains("A copy of the conversation runs on your own model"), "{:?}", results[2]);
    assert!(results[3].1 && results[3].0.contains("model must be \"lead\" or \"crew\""), "{:?}", results[3]);
}

#[tokio::test]
async fn a_report_names_the_files_the_app_saw_change_and_keeps_its_footer_last() {
    let child = ScriptedProvider::new([
        call(&[
            ("w1", "write", json!({ "path": "src/a.rs", "content": "a" })),
            ("w2", "write", json!({ "path": "src/bad.rs", "content": "b" })),
            ("w3", "write", json!({ "path": "docs/notes.md", "content": "c" })),
        ]),
        say("Changed a.rs."),
    ]);
    let mut parent = team_parent(vec![task("p1", "general", json!({ "files": ["src/a.rs", "./src/b.rs"] })), say("ok")], child, true);
    parent.run("Go", &Control::new()).await.unwrap();
    let (report, _) = &parent.results()[0];
    assert!(report.contains(&format!("{}\n", super::child::CHANGED_HEADING)), "{report}");
    assert!(report.contains("- src/a.rs\n- docs/notes.md (outside its files)\n"), "{report}");
    assert!(!report.contains("bad.rs"), "a failed write changed nothing");
    assert!(report.lines().last().unwrap().starts_with("[agent a1 · general · 3 tool calls · "), "{report}");
    let info = &parent.agents.list()[0];
    assert_eq!(info.files, ["src/a.rs", "src/b.rs"]);
    assert_eq!(info.changed, ["src/a.rs", "docs/notes.md"]);
    assert!(info.spent.input + info.spent.output > 0 || info.spent == Default::default());
}

#[tokio::test]
async fn a_team_worker_is_briefed_and_the_lead_is_told_the_contract_once() {
    let child = ScriptedProvider::new([say("one"), say("two")]);
    let mut parent = team_parent(
        vec![
            call(&[("t1", crate::todo::TODO, json!({ "action": "create", "subject": "Parse the header" }))]),
            task("p1", "general", json!({ "files": ["src/retry.rs"], "background": true })),
            task("p2", "general", json!({ "files": ["src/parse.rs"] })),
            say("Both started."),
        ],
        child.clone(),
        true,
    );
    parent.run("Honour Retry-After everywhere.", &Control::new()).await.unwrap();
    let briefs: Vec<String> = child.requests().iter().map(|request| request.messages[0].text()).collect();
    let second = briefs.iter().find(|text| text.contains("src/parse.rs\nStay within them.")).expect("the second worker's briefing");
    assert!(second.contains("## Working beside you\n- a1 (general · general task): src/retry.rs"), "{second}");
    assert!(second.contains("## What the user asked the lead\nHonour Retry-After everywhere."), "{second}");
    assert!(second.contains("## The lead's todo list\n[pending] #1 Parse the header"), "{second}");
    assert!(second.ends_with("## Your task\nDo the general part."), "{second}");
    // The lead's every request carries the contract once, as a note after its instructions.
    for request in parent.provider.requests() {
        let notes = request.messages.iter().filter(|message| message.text().starts_with(TEAM_MARKER)).count();
        assert_eq!(notes, 1);
    }
    assert!(parent.provider.requests()[0].messages.iter().any(|message| message.text().contains("your crew runs on crew-model")));
}

#[tokio::test]
async fn workers_started_in_one_reply_are_told_about_each_other() {
    let child = ScriptedProvider::new([say("one"), say("two")]);
    let start = |id: &str, file: &str| (id.to_string(), AGENT.to_string(), json!({ "agent": "general", "description": format!("Own {file}"), "prompt": "Go.", "files": [file] }));
    let mut parent = team_parent(vec![calls(&[start("p1", "src/a.rs"), start("p2", "src/b.rs")]), say("ok")], child.clone(), true);
    parent.run("Change both", &Control::new()).await.unwrap();
    let briefs: Vec<String> = child.requests().iter().map(|request| request.messages[0].text()).collect();
    for (mine, theirs) in [("src/a.rs", "src/b.rs"), ("src/b.rs", "src/a.rs")] {
        let brief = briefs.iter().find(|text| text.contains(&format!("## Your files\n{mine}\n"))).unwrap();
        assert!(brief.contains(&format!("general · Own {theirs}): {theirs}")), "{brief}");
        assert_eq!(brief.matches(&format!("Own {theirs}")).count(), 1, "named once: {brief}");
    }
}

#[tokio::test]
async fn the_lead_sends_the_same_instructions_and_tools_with_team_on_or_off() {
    let mut on = team_parent(vec![say("hi")], ScriptedProvider::new([]), true);
    let mut off = team_parent(vec![say("hi")], ScriptedProvider::new([]), false);
    on.run("Go", &Control::new()).await.unwrap();
    off.run("Go", &Control::new()).await.unwrap();
    // Each parent has a folder of its own; past that, the requests must start the same.
    let instructions = |parent: &Parent| {
        let folder = parent.folder.path().display().to_string();
        parent.provider.requests()[0].system.iter().map(|part| part.text.replace(&folder, "<folder>")).collect::<Vec<_>>()
    };
    assert_eq!(instructions(&on), instructions(&off));
    assert_eq!(on.provider.requests()[0].tools, off.provider.requests()[0].tools);
}
