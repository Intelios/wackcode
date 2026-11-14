//! Shows a run on the terminal: the reply as it streams, and a line or two per tool call.

use std::{
    io::{IsTerminal, Write},
    sync::Mutex,
};

use serde_json::Value;
use wack_agent::Event;
use wack_llm::Usage;

const RESULT_LINES: usize = 6;

fn colour() -> bool {
    std::io::stdout().is_terminal()
}

fn paint(code: &str, text: &str) -> String {
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

#[derive(Default)]
pub struct Printer {
    state: Mutex<State>,
}

impl Printer {
    pub fn new() -> Self {
        Self::default()
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
            Event::Steered(text) => {
                Self::break_line(&mut state);
                println!("{}", paint("36", &format!("→ you: {text}")));
            }
            Event::Compacting => {
                Self::break_line(&mut state);
                println!("{}", dim("Compacting the conversation…"));
            }
            Event::Compacted { .. } => println!("{}", dim("Compacted.")),
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

/// The part of a tool's input worth a glance.
fn summary(name: &str, input: &Value) -> String {
    let field = |key: &str| input[key].as_str().map(str::to_string);
    let text = match name {
        "bash" => field("command"),
        "read" | "write" | "edit" | "ls" => field("path"),
        "grep" | "find" => field("pattern"),
        _ => None,
    }
    .unwrap_or_else(|| if input.as_object().is_some_and(|object| object.is_empty()) { String::new() } else { input.to_string() });
    let line = text.lines().next().unwrap_or_default();
    let short: String = line.chars().take(140).collect();
    if short.len() < text.len() { format!("{short}…") } else { short }
}
