//! Running one child: its tools, its rules, its model, and turning how it ended into the report
//! its parent reads.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use wack_llm::{Block, Message, Provider, Role, SystemPart};

use super::{AgentDef, Agents, Child, MAX_REPORT_CHARS, Status, StoppedBy, prompt::CHILD_PROMPT};
use crate::{
    agent::{Agent, Control, Event, Feature, Outcome, Profile, Settings, Verdict},
    ask::ASK_USER_QUESTION,
    memory::MEMORY_TOOLS,
    plan::{PLAN_MODE_COMPLETE, PLAN_MODE_ENTER, policy},
    registry::Registry,
    session::Session,
    suggest::SUGGEST_TASK,
    tasks::{STOP_TASK, Tasks, TasksFeature, WAIT_FOR_TASKS, WATCH},
    todo::TODO,
    tool::{Tool, ToolSpec},
    usage::{ForSubagent, Recorder},
};

/// Tools only the chat itself has: spawning (so children never nest), talking to the user, the
/// chat's plan and todo list and task cards, and the project's memory.
const PARENT_ONLY: [&str; 7] = [super::AGENT, super::AGENT_MESSAGE, ASK_USER_QUESTION, TODO, PLAN_MODE_ENTER, PLAN_MODE_COMPLETE, SUGGEST_TASK];
/// Families only the chat itself has: computer use (`wack_computer::tools::PREFIX`), whose apps
/// the user allowed for the conversation they are having, not for work they never see.
const PARENT_ONLY_PREFIXES: [&str; 1] = ["computer_"];

fn parent_only(name: &str) -> bool {
    PARENT_ONLY.contains(&name) || MEMORY_TOOLS.contains(&name) || PARENT_ONLY_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}
/// The shell family. A role that makes no edits keeps it: helpers must be able to grep, build and
/// run tests.
const SHELL: [&str; 4] = ["bash", WATCH, STOP_TASK, WAIT_FOR_TASKS];

/// A model a child can run on, with what records its usage.
#[derive(Clone)]
pub struct ChildModel {
    pub provider: Arc<dyn Provider>,
    pub settings: Settings,
    pub recorder: Option<Arc<dyn Recorder>>,
}

/// The tools a role gets out of its parent's: those its file allows, less the parent's own, and
/// for a role that makes no edits, nothing that changes things except the shell.
pub fn child_tools(def: &AgentDef, pool: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    let edits = def.edits();
    pool.iter()
        .filter(|tool| {
            let spec = tool.spec();
            let name = spec.name.as_str();
            !parent_only(name)
                && def.allows(name)
                && (edits || spec.parallel_safe || SHELL.contains(&name))
        })
        .cloned()
        .collect()
}

/// How a child's session begins.
pub(crate) enum Start {
    /// A new session, with the role's prompt and tools; the brief is its first message.
    Fresh { brief: String, tools: Vec<Arc<dyn Tool>> },
    /// The child's own session, as its last round left it; `message` starts the next. A copy
    /// of the conversation keeps sending its parent's instructions (`system`).
    Continue { message: String, tools: Vec<Arc<dyn Tool>>, system: Option<Vec<SystemPart>> },
    /// A copy of the parent's conversation, sent with exactly the parent's instructions, tools
    /// and settings so the prompt cache holds. The copy ends with the parent's open calls
    /// answered and the directive.
    Copy { system: Vec<SystemPart>, messages: Vec<Message>, loaded: Vec<String>, registry: Arc<Registry>, settings: Settings, call_id: String, directive: String },
}

/// What a child is started with.
pub(crate) struct Launch {
    pub def: AgentDef,
    pub start: Start,
    pub model: ChildModel,
    pub cwd: PathBuf,
    /// The skills index, as its parent's prompt has it.
    pub skills: Option<String>,
    /// The parent was planning, read-only, when it started this child.
    pub planning: bool,
    /// The ledger's name for the child: its chat and its id.
    pub ledger_id: String,
}

/// Starts `child`'s run on its own task once one of the chat's places is free. Its end is
/// recorded on `child`, whatever happens.
pub(crate) fn start(child: Arc<Child>, agents: Agents, launch: Launch, control: Control) {
    tokio::spawn(async move {
        let Some(_slot) = agents.slot(&control).await else {
            child.finish(Status::Stopped { by: StoppedBy::User }, Err("It was stopped before it started.".to_string()));
            return;
        };
        child.step(|live| live.status = Status::Running);
        let (status, report) = match run(&child, launch, &control).await {
            Ok(ended) => ended,
            Err(message) => (Status::Failed { message: message.clone() }, Err(format!("The agent could not start: {message}"))),
        };
        child.finish(status, report);
    });
}

async fn run(child: &Arc<Child>, launch: Launch, control: &Control) -> Result<(Status, Result<String, String>), String> {
    let Launch { def, start, model, cwd, skills, planning, ledger_id } = launch;
    let limited = Arc::new(AtomicBool::new(false));
    let copy = matches!(start, Start::Copy { .. });
    let rules = Rules {
        planning,
        copy,
        cwd: cwd.clone(),
        max_turns: def.max_turns,
        replies: AtomicU32::new(0),
        control: control.clone(),
        limited: limited.clone(),
    };
    let tasks = Tasks::scratch();
    let profile = Profile { name: def.name.clone(), prompt: format!("{CHILD_PROMPT}\n\n# Your role\n\n{}", def.prompt), chat: false };
    // Ultra and commit attribution are the main chat's own settings: a fresh child role never runs
    // with either, except a conversation copy, which sends exactly what its parent does.
    let settings = Settings { ultra: false, commit_attribution: false, ..model.settings.clone() };
    let (mut agent, prompt) = match start {
        Start::Fresh { brief, tools } => {
            let session = Session::create(&child.session, &cwd)?;
            let mut agent = Agent::new(model.provider.clone(), Arc::new(Registry::new(tools)?), session, &profile, settings);
            if let Some(skills) = skills {
                agent = agent.with_system_part(skills);
            }
            (agent, Some(brief))
        }
        Start::Continue { message, tools, system: Some(system) } => {
            let session = Session::open(&child.session)?;
            let agent = Agent::new(model.provider.clone(), Arc::new(Registry::new(tools)?), session, &Profile::code(), model.settings.clone()).with_system(system);
            (agent, Some(message))
        }
        Start::Continue { message, tools, system: None } => {
            let session = Session::open(&child.session)?;
            let mut agent = Agent::new(model.provider.clone(), Arc::new(Registry::new(tools)?), session, &profile, settings);
            if let Some(skills) = skills {
                agent = agent.with_system_part(skills);
            }
            (agent, Some(message))
        }
        Start::Copy { system, messages, loaded, registry, settings, call_id, directive } => {
            let mut session = Session::create(&child.session, &cwd)?;
            for name in &loaded {
                session.load_tool(name)?;
            }
            let opening = messages.last().cloned();
            for message in messages {
                session.push(message, None)?;
            }
            session.push(answer_open_calls(opening.as_ref(), &call_id, &directive), None)?;
            let agent = Agent::new(model.provider.clone(), registry, session, &Profile::code(), settings).with_system(system);
            (agent, None)
        }
    };
    let tasks_feature = TasksFeature::new(agent.session(), tasks.clone());
    // Every start, a copied conversation's too: its tool calls are this child's.
    // The chat's pictures, not a folder of the child's own: `read_image` reopens what either saw.
    let images = child.session.parent().map(|folder| crate::images::Images::at(folder.join("images"))).unwrap_or_default();
    agent = agent.with_caller(child.id.clone()).with_tasks(tasks).with_images(images).with_feature(Box::new(tasks_feature)).with_feature(Box::new(rules));
    if let Some(recorder) = model.recorder {
        agent = agent.with_recorder(Arc::new(ForSubagent { inner: recorder, id: ledger_id }));
    }
    let observer = child.clone();
    let sink = move |event: Event| observe(&observer, event);
    let outcome = match prompt {
        Some(prompt) => agent.run(&prompt, control, &sink).await,
        None => agent.carry_on(control, &sink).await,
    };
    let text = last_reply(agent.session());
    Ok(ending(child, outcome, text, limited.load(Ordering::SeqCst), def.max_turns))
}

/// The message that turns a copy of the parent's conversation into the child's: an answer to
/// every call in the parent's last reply (each needs one before anything else is sent), then the
/// directive.
fn answer_open_calls(reply: Option<&Message>, call_id: &str, directive: &str) -> Message {
    let mut content: Vec<Block> = reply
        .into_iter()
        .flat_map(|reply| reply.tool_uses())
        .map(|(id, _, _)| Block::ToolResult {
            tool_use_id: id.to_string(),
            content: if id == call_id {
                COPY_STARTED.to_string()
            } else {
                "This call is your parent's, not yours: it runs separately, and you won't see its result.".to_string()
            },
            is_error: false,
            images: Vec::new(),
        })
        .collect();
    content.push(Block::Text { text: format!("{COPY_DIRECTIVE}\n\n{directive}") });
    Message { role: Role::User, content }
}

/// The started call's answer, in a copy of the conversation.
const COPY_STARTED: &str = "This call started you. You are a sub-agent working from a copy of the conversation above, not the agent that made the call: that agent carries on separately.";
pub const COPY_DIRECTIVE: &str = "You are a sub-agent started from a copy of this conversation. Carry out the task below on your own, then end with a report for the agent that started you: it sees only your final message. Don't start other agents, don't ask the user anything, and don't take on work beyond the task.\n\nYour task:";

/// Keeps the child's counters and latest activity up to date for whoever watches.
fn observe(child: &Child, event: Event) {
    match event {
        Event::ToolCall { name, input, .. } => child.step(|live| {
            live.tool_calls += 1;
            live.activity = Some(super::Activity { target: target(&input), tool: name });
        }),
        Event::Reply { usage: Some(usage) } => child.step(|live| live.context_tokens = usage.input + usage.cache_read + usage.cache_write + usage.output),
        _ => {}
    }
}

/// What a call is pointed at, for the line that says what a child is doing: what it looks for
/// before where it looks.
fn target(input: &Value) -> String {
    let text = ["pattern", "query", "url", "command", "path", "description"].iter().find_map(|key| input[key].as_str()).unwrap_or_default();
    let line = text.lines().next().unwrap_or_default().trim();
    let mut short: String = line.chars().take(120).collect();
    if short.len() < line.len() {
        short.push('…');
    }
    short
}

/// The text of the child's last reply, which is its report.
fn last_reply(session: &Session) -> String {
    session.messages().iter().rev().find(|message| message.role == Role::Assistant).map(|message| message.text()).unwrap_or_default().trim().to_string()
}

/// The child's status, and what its parent reads: the report with a footer line, or why there is
/// none. An `Err` only when nothing came back.
fn ending(child: &Child, outcome: Result<Outcome, String>, text: String, limited: bool, max_turns: u32) -> (Status, Result<String, String>) {
    let (status, why) = match outcome {
        Ok(Outcome::Done) => (Status::Done, None),
        Ok(Outcome::CutOff) => (Status::Done, Some("Its last reply reached the model's length limit before it finished.".to_string())),
        Ok(Outcome::Refused) => (Status::Failed { message: "The model declined to continue.".into() }, Some("The model declined to continue.".to_string())),
        Ok(Outcome::Stopped) if limited => (
            Status::Stopped { by: StoppedBy::Limit },
            Some(format!("It was stopped after using all {max_turns} of its steps.")),
        ),
        Ok(Outcome::Stopped) => (Status::Stopped { by: StoppedBy::User }, Some("It was stopped before it finished.".to_string())),
        Err(message) => (Status::Failed { message: message.clone() }, Some(format!("It failed: {message}"))),
    };
    let info = child.info();
    let footer = format!(
        "[agent {} · {} · {} · {} · {}]",
        child.id,
        child.role,
        plural(info.tool_calls as u64, "tool call"),
        tokens(info.context_tokens),
        duration(child.elapsed())
    );
    let text = cap(&text);
    let report = match (why, text.is_empty()) {
        (None, false) => Ok(format!("{text}\n\n{footer}")),
        (None, true) => Ok(format!("The agent finished without writing a report.\n\n{footer}")),
        (Some(why), false) => Ok(format!("{why} What it wrote last:\n\n{text}\n\n{footer}")),
        (Some(why), true) => Err(format!("{why} It wrote no report.\n\n{footer}")),
    };
    (status, report)
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_REPORT_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_REPORT_CHARS).collect();
    format!("{cut}\n\n[The rest of the report was cut: it was longer than {MAX_REPORT_CHARS} characters.]")
}

fn plural(count: u64, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

fn tokens(count: u64) -> String {
    match count {
        0..1_000 => format!("{count} tokens"),
        _ => format!("{}k tokens", (count + 500) / 1_000),
    }
}

pub(crate) fn duration(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        _ => format!("{}m {}s", seconds / 60, seconds % 60),
    }
}

/// A child's own rules: its step limit, Plan mode's read-only gate when its parent was planning,
/// and, for a copy of the conversation (which has all its parent's tools, so the request matches),
/// a refusal of the tools only the parent may use.
struct Rules {
    planning: bool,
    copy: bool,
    cwd: PathBuf,
    max_turns: u32,
    replies: AtomicU32,
    control: Control,
    limited: Arc<AtomicBool>,
}

/// Replies past the limit given to write the report before the child is stopped.
const GRACE_REPLIES: u32 = 2;

impl Feature for Rules {
    fn on_tool_call(&self, tool: &ToolSpec, input: &Value) -> Verdict {
        if self.replies.load(Ordering::SeqCst) >= self.max_turns {
            return Verdict::Deny(format!(
                "You have used all {} of your steps. Call no more tools: write your report now from what you have, and say what you didn't get to.",
                self.max_turns
            ));
        }
        let name = tool.name.as_str();
        if self.copy && parent_only(name) {
            return Verdict::Deny(format!(
                "As a sub-agent you can't use {name}: it belongs to the agent that started you. Do the work with your other tools, and put anything for the user in your report."
            ));
        }
        if !self.planning {
            return Verdict::Allow;
        }
        if name == "bash" || name == WATCH {
            let command = input["command"].as_str().unwrap_or_default();
            return match policy::blocked_segment(command, Some(&self.cwd)) {
                None => Verdict::Allow,
                Some(blocked) => Verdict::Deny(format!(
                    "The chat is in Plan mode, which only allows read-only shell commands (inspection, status, tests).\nBlocked command: {blocked}"
                )),
            };
        }
        if tool.parallel_safe || [STOP_TASK, WAIT_FOR_TASKS].contains(&name) {
            return Verdict::Allow;
        }
        Verdict::Deny(format!("The chat is in Plan mode, which blocks the mutating tool '{name}'."))
    }

    fn after_turn(&self, _reply: &wack_llm::Message) {
        let replies = self.replies.fetch_add(1, Ordering::SeqCst) + 1;
        if replies >= self.max_turns + GRACE_REPLIES {
            self.limited.store(true, Ordering::SeqCst);
            self.control.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::definition::builtins;

    #[test]
    fn a_role_gets_its_own_tools_and_never_the_parents() {
        let pool: Vec<Arc<dyn Tool>> = vec![
            Arc::new(Named("read", true)),
            Arc::new(Named("write", false)),
            Arc::new(Named("bash", false)),
            Arc::new(Named(super::super::AGENT, false)),
            Arc::new(Named(ASK_USER_QUESTION, true)),
            Arc::new(Named("memory_save", false)),
            Arc::new(Named("mcp__gh__list", true)),
            Arc::new(Named("mcp__gh__merge", false)),
            Arc::new(Named("computer_snapshot", true)),
            Arc::new(Named("computer_open", false)),
        ];
        let names = |def: &AgentDef| child_tools(def, &pool).iter().map(|tool| tool.spec().name).collect::<Vec<_>>();
        let builtins = builtins();
        assert_eq!(names(&builtins[0]), ["read", "bash", "mcp__gh__list"], "explore: no edits, the shell kept, read-only servers only");
        assert_eq!(names(&builtins[2]), ["read", "write", "bash", "mcp__gh__list", "mcp__gh__merge"], "general: everything but the parent's own");
    }

    #[test]
    fn a_copy_never_uses_the_computer() {
        let rules = Rules {
            planning: false,
            copy: true,
            cwd: PathBuf::from("/tmp"),
            max_turns: 10,
            replies: AtomicU32::new(0),
            control: Control::new(),
            limited: Arc::new(AtomicBool::new(false)),
        };
        for (name, safe) in [("computer_snapshot", true), ("computer_open", false)] {
            let verdict = rules.on_tool_call(&Named(name, safe).spec(), &Value::Null);
            assert!(matches!(verdict, Verdict::Deny(reason) if reason.contains("belongs to the agent that started you")), "{name}");
        }
        assert!(matches!(rules.on_tool_call(&Named("read", true).spec(), &Value::Null), Verdict::Allow));
    }

    struct Named(&'static str, bool);

    #[async_trait::async_trait]
    impl Tool for Named {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.0.to_string(),
                description: String::new(),
                input_schema: serde_json::json!({ "type": "object" }),
                loading: crate::tool::Loading::Always,
                parallel_safe: self.1,
                hint: String::new(),
                guidelines: Vec::new(),
                ends_run: false,
                together: false,
            }
        }

        async fn run(&self, _input: Value, _context: &crate::tool::ToolContext<'_>) -> Result<crate::tool::ToolOutput, String> {
            Ok(String::new().into())
        }
    }

    #[test]
    fn short_forms_read_well() {
        assert_eq!(tokens(940), "940 tokens");
        assert_eq!(tokens(38_400), "38k tokens");
        assert_eq!(duration(Duration::from_secs(72)), "1m 12s");
        assert_eq!(plural(1, "tool call"), "1 tool call");
    }
}
