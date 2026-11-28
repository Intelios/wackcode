//! Shows a run on the terminal: the reply as it streams, and a line or two per tool call.

use std::{
    collections::HashMap,
    io::{IsTerminal, Write},
    sync::{Arc, Mutex},
};

use serde_json::Value;
use wack_agent::{
    Agents, Event,
    agents::Status as AgentStatus,
    plan::{PLAN_STATE, PlanState},
    todo::{Status, TODO_STATE, TodoList},
};
use wack_llm::Usage;

use crate::asking::Asking;

const RESULT_LINES: usize = 6;

fn colour() -> bool {
    std::io::stdout().is_terminal()
}

pub fn paint(code: &str, text: &str) -> String {
    if colour() { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

#[derive(Default)]
struct State {
    /// True when the cursor is partway along a line of streamed text.
    mid_line: bool,
    thinking: bool,
    usage: Usage,
    replies: u32,
}

pub struct Printer {
    state: Mutex<State>,
    asking: Arc<Asking>,
}

impl Printer {
    pub fn new(asking: Arc<Asking>) -> Self {
        Self { state: Mutex::default(), asking }
    }

    pub fn event(&self, event: Event) {
        let mut state = self.state.lock().unwrap();
        match event {
            Event::Text(text) => {
                if state.thinking {
                    state.thinking = false;
                    Self::break_line(&mut state);
                }
                Self::stream(&mut state, &text);
            }
            Event::Thinking(text) => {
                state.thinking = true;
                Self::stream(&mut state, &dim(&text));
            }
            Event::ToolStart { .. } | Event::ToolInput { .. } => {}
            Event::ToolCall { name, input, .. } => {
                state.thinking = false;
                Self::break_line(&mut state);
                println!("{} {}", paint("1", &format!("▸ {name}")), summary(&name, &input));
            }
            Event::ToolResult { content, is_error, .. } => {
                let lines: Vec<&str> = content.lines().collect();
                for line in lines.iter().take(RESULT_LINES) {
                    let line: String = line.chars().take(160).collect();
                    println!("  {}", if is_error { paint("31", &line) } else { dim(&line) });
                }
                if lines.len() > RESULT_LINES {
                    println!("  {}", dim(&format!("… {} more lines", lines.len() - RESULT_LINES)));
                }
            }
            Event::Reply { usage } => {
                state.replies += 1;
                if let Some(usage) = usage {
                    state.usage.input += usage.input;
                    state.usage.output += usage.output;
                    state.usage.cache_read += usage.cache_read;
                    state.usage.cache_write += usage.cache_write;
                }
            }
            Event::Retrying(notice) => {
                Self::break_line(&mut state);
                println!("{}", dim(&format!("Retrying in {}s (try {}): {}", notice.wait.as_secs(), notice.attempt, notice.reason)));
            }
            Event::Thought { .. } | Event::Context { .. } => {}
            Event::Interrupted => {
                state.thinking = false;
                Self::break_line(&mut state);
                println!("{}", dim("Interrupted for your message."));
            }
            Event::Steered(text) => {
                Self::break_line(&mut state);
                println!("{}", paint("36", &format!("→ you: {text}")));
            }
            Event::Compacting => {
                Self::break_line(&mut state);
                println!("{}", dim("Compacting the conversation…"));
            }
            Event::Compacted { .. } => println!("{}", dim("Compacted.")),
            Event::Question { id, questions, wrap_up } => {
                Self::break_line(&mut state);
                self.asking.ask(id, questions, wrap_up);
            }
            Event::State { key, value } if key == PLAN_STATE => {
                if let Some(PlanState { mode, plan: Some(plan) }) = PlanState::from_value(&value) {
                    Self::break_line(&mut state);
                    println!("\n{}\n\n{plan}\n", paint("1;35", &format!("Proposed plan ({})", mode.label())));
                }
            }
            Event::State { key, value } if key == TODO_STATE => {
                if let Some(list) = TodoList::from_value(&value) {
                    Self::break_line(&mut state);
                    println!("{}", dim(&todo_line(&list)));
                }
            }
            Event::State { .. } => {}
            Event::Noted { tag, text } if tag.starts_with(wack_agent::tasks::NOTE_PREFIX) => {
                Self::break_line(&mut state);
                let mut lines = text.lines();
                println!("{}", paint("36", &format!("◆ {}", lines.next().unwrap_or_default())));
                for line in lines.filter(|line| !line.starts_with("```")).take(RESULT_LINES) {
                    println!("  {}", dim(line));
                }
            }
            // An MCP server failed or stopped: the model was told, and so is the user.
            Event::Noted { tag, text } if tag.starts_with(wack_mcp::NOTE_PREFIX) => {
                Self::break_line(&mut state);
                println!("{}", paint("33", &text));
            }
            Event::Noted { .. } => {}
            Event::Notice(text) => {
                Self::break_line(&mut state);
                println!("{}", paint("33", &text));
            }
        }
        let _ = std::io::stdout().flush();
    }

    fn stream(state: &mut State, text: &str) {
        print!("{text}");
        if let Some(last) = text.chars().last() {
            state.mid_line = last != '\n';
        }
    }

    fn break_line(state: &mut State) {
        if state.mid_line {
            println!();
            state.mid_line = false;
        }
    }

    pub fn finish(&self) {
        Self::break_line(&mut self.state.lock().unwrap());
    }

    pub fn usage_line(&self) -> String {
        let state = self.state.lock().unwrap();
        let usage = state.usage;
        format!(
            "{} replies · {} in · {} out · {} cache read · {} cache write",
            state.replies, usage.input, usage.output, usage.cache_read, usage.cache_write
        )
    }
}

/// The todo list in a line: how far along it is and what is being done now.
fn todo_line(list: &TodoList) -> String {
    let total = list.visible().count();
    let done = list.visible().filter(|task| task.status == Status::Completed).count();
    let doing = list.visible().find(|task| task.status == Status::InProgress);
    match doing {
        Some(task) => format!("todo {done}/{total} · {}", task.active_form.as_deref().unwrap_or(&task.subject)),
        None => format!("todo {done}/{total}"),
    }
}

/// The part of a tool's input worth a glance.
fn summary(name: &str, input: &Value) -> String {
    let field = |key: &str| input[key].as_str().map(str::to_string);
    let text = match name {
        "bash" | "watch" => {
            let background = if input["run_in_background"].as_bool() == Some(true) { " (in the background)" } else { "" };
            field("command").map(|command| format!("{command}{background}"))
        }
        "stop_task" => field("id"),
        "read" | "write" | "edit" | "ls" => field("path"),
        "grep" | "find" => field("pattern"),
        wack_agent::agents::AGENT => Some(format!("{} · {}", field("agent").unwrap_or_else(|| "copy".into()), field("description").unwrap_or_default())),
        wack_agent::agents::AGENT_MESSAGE => Some(format!("{} · {}", field("agent_id").unwrap_or_default(), field("message").unwrap_or_default())),
        _ => None,
    }
    .unwrap_or_else(|| if input.as_object().is_some_and(|object| object.is_empty()) { String::new() } else { input.to_string() });
    let line = text.lines().next().unwrap_or_default();
    let short: String = line.chars().take(140).collect();
    if short.len() < text.len() { format!("{short}…") } else { short }
}

/// Prints each sub-agent's start, steps and end, indented under the call that started it.
/// The run's agents in a few lines when any started: each one's tier, model, end and spend, then
/// what the crew and the lead's helpers spent in all.
pub fn team_summary(agents: &Agents) -> Option<String> {
    let list = agents.list();
    if list.is_empty() {
        return None;
    }
    let tokens = |usage: &wack_llm::Usage| usage.input + usage.output + usage.cache_read + usage.cache_write;
    let mut lines = vec!["agents:".to_string()];
    let mut totals = [0u64; 2];
    for child in &list {
        let spent = tokens(&child.spent);
        let tier = child.tier.map_or("?", |tier| tier.as_str());
        totals[usize::from(tier == "crew")] += spent;
        let status = match &child.status {
            AgentStatus::Done => "done".to_string(),
            AgentStatus::Failed { message } => format!("failed ({message})"),
            AgentStatus::Stopped { .. } => "stopped".to_string(),
            AgentStatus::Interrupted => "interrupted".to_string(),
            AgentStatus::Queued => "queued".to_string(),
            AgentStatus::Running => "running".to_string(),
        };
        let changed = if child.changed.is_empty() { String::new() } else { format!(" · changed {}", child.changed.join(", ")) };
        lines.push(format!("  {} {} [{tier} · {}] {status} · {} tool calls · {spent} tokens{changed}", child.id, child.role, child.model, child.tool_calls));
    }
    lines.push(format!("  crew tokens {} · lead-model agents' tokens {}", totals[1], totals[0]));
    Some(lines.join("\n"))
}

pub fn watch_agents(agents: &Agents) {
    let seen: Mutex<HashMap<(String, u32), (u32, bool)>> = Mutex::default();
    let listed = agents.clone();
    agents.set_observer(move |_| {
        let mut seen = seen.lock().unwrap();
        for child in listed.list() {
            let (steps, ended) = seen.entry((child.id.clone(), child.round)).or_insert_with(|| {
                let round = if child.round > 1 { format!(" (round {})", child.round) } else { String::new() };
                let tier = child.tier.map(|tier| format!(" [{} · {}]", tier.as_str(), child.model)).unwrap_or_default();
                println!("  {}", paint("36", &format!("↳ {} {} · {}{round}{tier}", child.id, child.role, child.description)));
                (0, false)
            });
            if child.tool_calls > *steps {
                *steps = child.tool_calls;
                if let Some(activity) = &child.activity {
                    println!("    {}", dim(&format!("· {} {}", activity.tool, activity.target)));
                }
            }
            if !*ended && !child.status.active() {
                *ended = true;
                let how = match &child.status {
                    AgentStatus::Done => "done".to_string(),
                    AgentStatus::Failed { message } => format!("failed: {message}"),
                    AgentStatus::Stopped { .. } => "stopped".to_string(),
                    AgentStatus::Interrupted => "interrupted".to_string(),
                    AgentStatus::Queued | AgentStatus::Running => unreachable!("it has ended"),
                };
                println!("  {}", paint("36", &format!("↳ {} {how} · {} tool calls", child.id, child.tool_calls)));
            }
        }
        let _ = std::io::stdout().flush();
    });
}
