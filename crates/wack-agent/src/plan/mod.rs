//! Plan and Ultra Plan: read-only modes in which the agent explores, asks, and submits a plan
//! for the user to review. Ported from version 2 (forked there from `@narumitw/pi-plan-mode`
//! v0.58.3, MIT). Ultra Plan is the same mode under a different contract: an exhaustive
//! interview, one question at a time, instead of a few questions.
//!
//! How it holds together:
//! - **The mode is the caller's.** The app keeps each chat's mode and passes it to every run;
//!   the session only records what the model has been told. The agent can enter a planning mode
//!   itself (`plan_mode_enter`), never leave one: only the user returns a chat to Build.
//! - **The rules arrive as a contract**, a note in the conversation, sent whenever the mode in
//!   force differs from the last contract the model can still see (`Plan::notes`). The system
//!   prompt and tool list are the same in every mode, so switching never breaks the prompt cache.
//! - **The prompt explains the rules; `on_tool_call` enforces them.** While planning, a tool runs
//!   only if it is one of planning's own, changes nothing, or is a shell command the policy
//!   recognises as reading. Lifting the restrictions (`unrestricted`) skips only this check: the
//!   interview, the submission and the review stay.
//! - **A submitted plan waits for the user** under the `plan` state key. Switching modes keeps it;
//!   the user's next message, in any mode, consumes it: feedback needs a new submission, and a
//!   Build turn starts the work.

pub mod policy;
pub mod prompt;
pub mod proposed;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wack_llm::{Message, Request, Role};

use crate::{
    agent::{Feature, Verdict},
    ask::ASK_USER_QUESTION,
    session::{Note, Session},
    todo::TODO,
    tool::{Loading, Tool, ToolContext, ToolSpec},
};

pub const PLAN_MODE_ENTER: &str = "plan_mode_enter";
pub const PLAN_MODE_COMPLETE: &str = "plan_mode_complete";
/// The session state key holding [`PlanState`].
pub const PLAN_STATE: &str = "plan";
pub const MAX_PLAN_CHARS: usize = 50_000;
const NOTE_PREFIX: &str = "contract:";
const ONLY_WHILE_PLANNING: &str = "plan_mode_complete is only available while Plan mode is active.";

/// What a chat is doing: working, or planning in one of two ways.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Build,
    Plan,
    #[serde(rename = "ultraplan")]
    UltraPlan,
}

impl AgentMode {
    /// Plan and Ultra Plan both count as planning.
    pub fn planning(self) -> bool {
        self != Self::Build
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::UltraPlan => "ultraplan",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "build" => Self::Build,
            "plan" => Self::Plan,
            "ultraplan" => Self::UltraPlan,
            _ => return None,
        })
    }

    /// The name the user sees.
    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "Build",
            Self::Plan => "Plan",
            Self::UltraPlan => "Ultra Plan",
        }
    }
}

/// The rules a contract note carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contract {
    Plan,
    UltraPlan,
    /// Planning is over: the earlier restrictions no longer apply.
    Normal,
}

impl Contract {
    fn tag(self) -> &'static str {
        match self {
            Self::Plan => "contract:plan",
            Self::UltraPlan => "contract:ultraplan",
            Self::Normal => "contract:normal",
        }
    }
}

/// What the session saves under [`PLAN_STATE`], and what the app reads to draw the mode and the
/// plan awaiting review.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanState {
    pub mode: AgentMode,
    /// A submitted plan the user has not acted on yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

impl PlanState {
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Checks a submitted plan. The error is a phrase the model is shown.
pub fn normalize_plan(input: &Value) -> Result<String, String> {
    let Some(plan) = input.get("plan").and_then(Value::as_str) else {
        return Err("plan must be a string".to_string());
    };
    let plan = plan.trim();
    if plan.is_empty() {
        return Err("plan must not be empty".to_string());
    }
    if plan.chars().count() > MAX_PLAN_CHARS {
        return Err(format!("plan must not exceed {MAX_PLAN_CHARS} characters"));
    }
    Ok(plan.to_string())
}

/// Reads `plan_mode_enter`'s input: Plan unless Ultra Plan is asked for.
fn normalize_entry(input: &Value) -> Result<AgentMode, String> {
    if !input.is_object() {
        return Err("Provide an object with an optional mode: plan or ultraplan.".to_string());
    }
    match input.get("mode") {
        None | Some(Value::Null) => Ok(AgentMode::Plan),
        Some(Value::String(mode)) if mode == "plan" => Ok(AgentMode::Plan),
        Some(Value::String(mode)) if mode == "ultraplan" => Ok(AgentMode::UltraPlan),
        Some(_) => Err("Planning mode must be plan or ultraplan; only the user can switch back to Build.".to_string()),
    }
}

/// The plan feature and its two tools, sharing one state.
#[derive(Clone)]
pub struct Plan {
    state: Arc<Mutex<PlanState>>,
    /// What the session held when the run began, so a chat that never plans saves nothing.
    saved: bool,
    unrestricted: bool,
    cwd: PathBuf,
}

impl Plan {
    /// `mode` is the chat's mode as the app has it; `None` keeps the mode the session last saved.
    pub fn new(session: &Session, mode: Option<AgentMode>, unrestricted: bool) -> Self {
        let saved = session.state(PLAN_STATE).and_then(PlanState::from_value);
        let mut state = saved.clone().unwrap_or_default();
        if let Some(mode) = mode {
            state.mode = mode;
        }
        Self { state: Arc::new(Mutex::new(state)), saved: saved.is_some(), unrestricted, cwd: session.cwd().to_path_buf() }
    }

    pub fn state(&self) -> PlanState {
        self.state.lock().unwrap().clone()
    }

    pub fn mode(&self) -> AgentMode {
        self.state.lock().unwrap().mode
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![Arc::new(Enter(self.clone())), Arc::new(Complete(self.clone()))]
    }

    fn expected_contract(&self, session: &Session) -> Option<Contract> {
        match self.mode() {
            AgentMode::Plan => Some(Contract::Plan),
            AgentMode::UltraPlan => Some(Contract::UltraPlan),
            // A chat that never planned needs no word that planning is over.
            AgentMode::Build => session.ever_noted(NOTE_PREFIX).then_some(Contract::Normal),
        }
    }
}

impl Feature for Plan {
    fn on_run_start(&self) {
        self.state.lock().unwrap().plan = None;
    }

    fn notes(&self, session: &Session) -> Vec<Note> {
        match self.expected_contract(session) {
            Some(contract) if session.latest_note(NOTE_PREFIX) != Some(contract.tag()) => {
                vec![Note { tag: contract.tag().to_string(), text: prompt::contract(contract) }]
            }
            _ => Vec::new(),
        }
    }

    fn before_request(&self, request: &mut Request) {
        if !(self.unrestricted && self.mode().planning()) {
            return;
        }
        // After the contract it supersedes, and before the user's request, which stays last.
        let contracts = [Contract::Plan, Contract::UltraPlan, Contract::Normal].map(prompt::contract);
        let after = request
            .messages
            .iter()
            .rposition(|message| message.role == Role::User && contracts.contains(&message.text()))
            .map_or(0, |index| index + 1);
        request.messages.insert(after, Message::user(prompt::UNRESTRICTED_ACCESS));
    }

    fn on_tool_call(&self, tool: &ToolSpec, input: &Value) -> Verdict {
        let name = tool.name.as_str();
        if !self.mode().planning() {
            return if name == PLAN_MODE_COMPLETE { Verdict::Deny(ONLY_WHILE_PLANNING.to_string()) } else { Verdict::Allow };
        }
        // Planning's own tools always pass. `todo` changes only its own list, never the project.
        if self.unrestricted || [ASK_USER_QUESTION, PLAN_MODE_ENTER, PLAN_MODE_COMPLETE, TODO].contains(&name) {
            return Verdict::Allow;
        }
        if name == "bash" {
            let command = input["command"].as_str().unwrap_or_default();
            return match policy::blocked_segment(command, Some(&self.cwd)) {
                None => Verdict::Allow,
                Some(blocked) => Verdict::Deny(format!(
                    "Plan mode only allows read-only shell commands (inspection, status, tests).\nBlocked command: {blocked}"
                )),
            };
        }
        if tool.parallel_safe {
            return Verdict::Allow;
        }
        Verdict::Deny(format!("Plan mode blocks the mutating tool '{name}'."))
    }

    fn on_run_end(&self, reply: &str) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        if !state.mode.planning() {
            return None;
        }
        match proposed::parse(reply) {
            proposed::Proposed::Valid(plan) => {
                state.plan = Some(plan);
                None
            }
            other => proposed::invalid_message(&other),
        }
    }

    fn state_to_save(&self) -> Vec<(String, Value)> {
        let state = self.state();
        if !self.saved && state == PlanState::default() {
            return Vec::new();
        }
        vec![(PLAN_STATE.to_string(), state.to_value())]
    }
}

struct Enter(Plan);

#[async_trait]
impl Tool for Enter {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: PLAN_MODE_ENTER.into(),
            description: "Enter Plan mode and continue planning in this run under the selected mode contract, then submit a decision-ready plan with plan_mode_complete. Use your judgment when implementation decisions need working through; respect explicit requests to skip planning or stay in Build. Default to regular Plan; use ultraplan for an exhaustive interview. Do not use this tool for straightforward work, ordinary roadmaps, checklists or plan-file work. Call this tool alone. Only the user can return to Build.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["plan", "ultraplan"],
                        "description": "Planning variant: plan (the default), or ultraplan for an exhaustive interview."
                    }
                }
            }),
            loading: Loading::Always,
            // Runs alone, so the calls after it in the same reply are judged under the new mode.
            parallel_safe: false,
            hint: "switch this chat into Plan or Ultra Plan when the work needs decisions settled first".into(),
            guidelines: vec!["Plan when useful.".into()],
            ends_run: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<String, String> {
        let mode = normalize_entry(&input)?;
        let unchanged = {
            let mut state = self.0.state.lock().unwrap();
            let unchanged = state.mode == mode;
            state.mode = mode;
            unchanged
        };
        Ok(format!(
            "{} mode is {} active. Continue planning in this run under the current mode contract and execution policy. Submit the complete plan with plan_mode_complete when it is decision-ready; the user reviews it before implementation.",
            mode.label(),
            if unchanged { "already" } else { "now" }
        ))
    }
}

struct Complete(Plan);

#[async_trait]
impl Tool for Complete {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: PLAN_MODE_COMPLETE.into(),
            description: "Submit a decision-ready plan only while Plan mode is active, and call it alone as the final action. Never call for ordinary planning requests, roadmaps, checklists, or plan-file work.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["plan"],
                "properties": {
                    "plan": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": MAX_PLAN_CHARS,
                        "description": "The complete decision-ready implementation plan in Markdown."
                    }
                }
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "submit the finished plan for the user's review, while Plan mode is on".into(),
            guidelines: Vec::new(),
            ends_run: true,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<String, String> {
        let mut state = self.0.state.lock().unwrap();
        if !state.mode.planning() {
            return Err(ONLY_WHILE_PLANNING.to_string());
        }
        let plan = normalize_plan(&input)?;
        state.plan = Some(plan.clone());
        Ok(format!("**Proposed Plan**\n\n{plan}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (tempfile::TempDir, Session) {
        let folder = tempfile::tempdir().unwrap();
        let session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        (folder, session)
    }

    fn spec(name: &str, parallel_safe: bool) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: String::new(),
            input_schema: json!({}),
            loading: Loading::Always,
            parallel_safe,
            hint: String::new(),
            guidelines: Vec::new(),
            ends_run: false,
        }
    }

    fn verdict(plan: &Plan, name: &str, parallel_safe: bool, input: Value) -> Option<String> {
        match plan.on_tool_call(&spec(name, parallel_safe), &input) {
            Verdict::Allow => None,
            Verdict::Deny(reason) => Some(reason),
        }
    }

    #[test]
    fn planning_allows_reading_and_its_own_tools_and_nothing_else() {
        let (_folder, session) = session();
        let plan = Plan::new(&session, Some(AgentMode::Plan), false);
        for (name, safe) in [("read", true), ("grep", true), (ASK_USER_QUESTION, false), (TODO, false), (PLAN_MODE_COMPLETE, false)] {
            assert_eq!(verdict(&plan, name, safe, json!({})), None, "{name}");
        }
        assert_eq!(verdict(&plan, "edit", false, json!({})).unwrap(), "Plan mode blocks the mutating tool 'edit'.");
        // A tool nobody vouched for is refused, whatever it is.
        assert!(verdict(&plan, "deploy", false, json!({})).is_some());
        assert_eq!(verdict(&plan, "bash", false, json!({ "command": "git status" })), None);
        let refused = verdict(&plan, "bash", false, json!({ "command": "ls && rm -rf x" })).unwrap();
        assert_eq!(refused, "Plan mode only allows read-only shell commands (inspection, status, tests).\nBlocked command: rm -rf x");

        // Lifting the restrictions lifts only this check.
        let free = Plan::new(&session, Some(AgentMode::UltraPlan), true);
        assert_eq!(verdict(&free, "edit", false, json!({})), None);

        let build = Plan::new(&session, Some(AgentMode::Build), false);
        assert_eq!(verdict(&build, "edit", false, json!({})), None);
        assert_eq!(verdict(&build, PLAN_MODE_COMPLETE, false, json!({})).unwrap(), ONLY_WHILE_PLANNING);
    }

    #[test]
    fn a_contract_is_owed_only_when_the_model_has_not_seen_the_current_one() {
        let (_folder, mut session) = session();
        let build = Plan::new(&session, Some(AgentMode::Build), false);
        assert!(build.notes(&session).is_empty(), "a chat that never planned is told nothing");

        let plan = Plan::new(&session, Some(AgentMode::Plan), false);
        let notes = plan.notes(&session);
        assert_eq!(notes[0].tag, "contract:plan");
        assert_eq!(notes[0].text, prompt::contract(Contract::Plan));
        session.push_note(notes[0].clone()).unwrap();
        assert!(plan.notes(&session).is_empty());

        // Plan to Ultra Plan, then back to Build.
        let ultra = Plan::new(&session, Some(AgentMode::UltraPlan), false);
        let notes = ultra.notes(&session);
        assert_eq!(notes[0].tag, "contract:ultraplan");
        session.push_note(notes[0].clone()).unwrap();
        let build = Plan::new(&session, Some(AgentMode::Build), false);
        let notes = build.notes(&session);
        assert_eq!(notes[0].text, prompt::contract(Contract::Normal));
        session.push_note(notes[0].clone()).unwrap();
        assert!(build.notes(&session).is_empty());
    }

    #[tokio::test]
    async fn a_plan_is_submitted_kept_across_a_mode_switch_and_consumed_by_the_next_message() {
        let (_folder, mut session) = session();
        let plan = Plan::new(&session, Some(AgentMode::Plan), false);
        let context = ToolContext::detached("/tmp");
        let tools = plan.tools();
        let complete = &tools[1];

        assert_eq!(complete.run(json!({ "plan": "  " }), &context).await.unwrap_err(), "plan must not be empty");
        assert_eq!(complete.run(json!({}), &context).await.unwrap_err(), "plan must be a string");
        let result = complete.run(json!({ "plan": " # Fix it\n\n1. Do it " }), &context).await.unwrap();
        assert_eq!(result, "**Proposed Plan**\n\n# Fix it\n\n1. Do it");
        for (key, value) in plan.state_to_save() {
            session.set_state(&key, value).unwrap();
        }

        // The user switches to Build: the proposal is still there to approve.
        let build = Plan::new(&session, Some(AgentMode::Build), false);
        assert_eq!(build.state(), PlanState { mode: AgentMode::Build, plan: Some("# Fix it\n\n1. Do it".into()) });
        // Their next message consumes it.
        build.on_run_start();
        assert_eq!(build.state().plan, None);
        assert_eq!(build.state_to_save()[0].1, json!({ "mode": "build" }));

        let untouched = Plan::new(&Session::create(&_folder.path().join("other.jsonl"), _folder.path()).unwrap(), None, false);
        assert!(untouched.state_to_save().is_empty(), "a chat that never plans saves nothing");
    }

    #[tokio::test]
    async fn the_agent_can_enter_planning_but_not_leave_it() {
        let (_folder, session) = session();
        let plan = Plan::new(&session, Some(AgentMode::Build), false);
        let context = ToolContext::detached("/tmp");
        let tools = plan.tools();
        let enter = &tools[0];
        assert!(enter.run(json!({}), &context).await.unwrap().starts_with("Plan mode is now active."));
        assert_eq!(plan.mode(), AgentMode::Plan);
        assert!(enter.run(json!({ "mode": "ultraplan" }), &context).await.unwrap().starts_with("Ultra Plan mode is now active."));
        assert!(enter.run(json!({ "mode": "ultraplan" }), &context).await.unwrap().starts_with("Ultra Plan mode is already active."));
        let refused = enter.run(json!({ "mode": "build" }), &context).await.unwrap_err();
        assert_eq!(refused, "Planning mode must be plan or ultraplan; only the user can switch back to Build.");
        assert_eq!(plan.mode(), AgentMode::UltraPlan);
    }

    #[test]
    fn a_prose_plan_is_accepted_and_a_broken_one_reported() {
        let (_folder, session) = session();
        let plan = Plan::new(&session, Some(AgentMode::Plan), false);
        assert_eq!(plan.on_run_end("<proposed_plan>\nDo it\n</proposed_plan>"), None);
        assert_eq!(plan.state().plan.as_deref(), Some("Do it"));
        assert!(plan.on_run_end("<proposed_plan>\nnever closed").unwrap().contains("closing tag"));
        let build = Plan::new(&session, Some(AgentMode::Build), false);
        assert_eq!(build.on_run_end("<proposed_plan>\nDo it\n</proposed_plan>"), None);
        assert_eq!(build.state().plan, None);
    }

    #[test]
    fn lifted_restrictions_are_explained_after_the_contract_and_never_last() {
        let (_folder, session) = session();
        let plan = Plan::new(&session, Some(AgentMode::Plan), true);
        let mut request = Request {
            model: "m".into(),
            system: Vec::new(),
            messages: vec![Message::user("earlier"), Message::user(prompt::contract(Contract::Plan)), Message::user("Plan the fix")],
            tools: Vec::new(),
            max_output_tokens: None,
            effort: None,
        };
        plan.before_request(&mut request);
        let texts: Vec<String> = request.messages.iter().map(Message::text).collect();
        assert_eq!(texts[2], prompt::UNRESTRICTED_ACCESS);
        assert_eq!(texts[3], "Plan the fix");

        let restricted = Plan::new(&session, Some(AgentMode::Plan), false);
        let before = request.clone();
        restricted.before_request(&mut request);
        assert_eq!(request, before);
    }
}
