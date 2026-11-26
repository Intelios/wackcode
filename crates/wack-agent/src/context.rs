//! How full the model's context is, part by part: what the composer's context popover lists.
//!
//! The provider only reports a total. Each part here is estimated the way compaction estimates
//! (a quarter of the characters, a picture as [`compaction::IMAGE_CHARS`]), then every part is
//! scaled so the parts add up to the measured total: the shares are estimates, the sum is real.
//! The parts are read from the request the agent would send next, so they are exactly what goes:
//! the system prompt's parts by their headings (they are built by `prompt` and the kit, which set
//! those headings), the tools' definitions, and the messages by who wrote them. Tool results are
//! grouped by the tool that made them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use wack_llm::{Block, Request, Role};

pub use crate::agent::COMPACT_AT_PERCENT;
use crate::{compaction::IMAGE_CHARS, prompt::ULTRA_PROMPT};

/// One part of the context, with what it is made of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub kind: Kind,
    pub tokens: u64,
    /// Its pieces, largest first where the order is not fixed ("AGENTS.md", "bash").
    pub pieces: Vec<(String, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// WackCode's own instructions and its guide to the chat.
    Instructions,
    /// The tools' names, descriptions and arguments.
    Tools,
    /// The project's AGENTS.md or CLAUDE.md files.
    Project,
    Skills,
    Memory,
    /// What stands in for turns compacted away.
    Summary,
    Messages,
    ToolResults,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Instructions => "WackCode's instructions",
            Kind::Tools => "Tools",
            Kind::Project => "Project instructions",
            Kind::Skills => "Skills",
            Kind::Memory => "Memory",
            Kind::Summary => "Summary of earlier turns",
            Kind::Messages => "Messages",
            Kind::ToolResults => "Tool results",
        }
    }

    /// Sent with every request, whatever the conversation holds.
    pub fn always_sent(self) -> bool {
        matches!(self, Kind::Instructions | Kind::Tools | Kind::Project | Kind::Skills | Kind::Memory)
    }
}

/// Tool results past this many tools fold into "Other".
const RESULT_PIECES: usize = 5;

/// `request`'s parts, scaled to add up to `measured` tokens (left as estimated when it is 0).
pub fn breakdown(request: &Request, measured: u64) -> Vec<Part> {
    let mut parts: Vec<(Kind, Vec<(String, usize)>)> = Vec::new();
    let mut add = |kind: Kind, piece: String, chars: usize| {
        if chars == 0 {
            return;
        }
        let at = match parts.iter().position(|(existing, _)| *existing == kind) {
            Some(at) => at,
            None => {
                parts.push((kind, Vec::new()));
                parts.len() - 1
            }
        };
        let pieces = &mut parts[at].1;
        match pieces.iter_mut().find(|(name, _)| *name == piece) {
            Some((_, total)) => *total += chars,
            None => pieces.push((piece, chars)),
        }
    };

    for (index, part) in request.system.iter().enumerate() {
        let text = part.text.as_str();
        if let Some(files) = text.strip_prefix("# Project instructions") {
            for file in files.split("\n\n## ").skip(1) {
                let path = file.lines().next().unwrap_or_default();
                add(Kind::Project, short_path(path), file.len());
            }
        } else if text.starts_with("# Skills") {
            let count = text.lines().filter(|line| line.starts_with("- ")).count();
            add(Kind::Skills, plural(count, "skill"), text.len());
        } else if text.starts_with("## Memory") || text.starts_with("## Project memory") {
            let count = text.lines().filter(|line| line.starts_with("- [")).count();
            add(Kind::Memory, plural(count, "note"), text.len());
        } else if text == ULTRA_PROMPT || index > 1 {
            add(Kind::Instructions, "Ultra".into(), text.len());
        } else if index == 0 {
            add(Kind::Instructions, "How WackCode works".into(), text.len());
        } else {
            add(Kind::Instructions, "Guide to this chat".into(), text.len());
        }
    }

    let (mut builtin, mut mcp, mut searchable) = ((0, 0), (0, 0), (0, 0));
    for tool in &request.tools {
        let chars = tool.name.len() + tool.description.len() + tool.input_schema.to_string().len();
        let group = if tool.deferred {
            &mut searchable
        } else if tool.name.starts_with("mcp__") {
            &mut mcp
        } else {
            &mut builtin
        };
        group.0 += 1;
        group.1 += chars;
    }
    add(Kind::Tools, plural(builtin.0, "built-in tool"), builtin.1);
    add(Kind::Tools, plural(mcp.0, "MCP tool"), mcp.1);
    add(Kind::Tools, plural(searchable.0, "tool found by search"), searchable.1);

    // A tool result names its call, not its tool: the call comes first in the conversation.
    let mut called: HashMap<&str, &str> = HashMap::new();
    let mut results: Vec<(String, usize)> = Vec::new();
    // After a compaction the first message is the summary of what was dropped.
    let summarised = request.messages.first().is_some_and(|first| {
        first.role == Role::User && matches!(first.content.as_slice(), [Block::Text { text }] if text.starts_with(crate::compaction::SUMMARY_HEADING))
    });
    for (index, message) in request.messages.iter().enumerate() {
        for block in &message.content {
            match (block, message.role) {
                (Block::Text { text }, Role::User) if index == 0 && summarised => add(Kind::Summary, "Summary".into(), text.len()),
                (Block::Text { text }, Role::User) => add(Kind::Messages, "You".into(), text.len()),
                (Block::Text { text }, Role::Assistant) => add(Kind::Messages, "Replies".into(), text.len()),
                (Block::Thinking { text, .. }, _) => add(Kind::Messages, "Thinking".into(), text.len()),
                (Block::ToolUse { id, name, input }, _) => {
                    called.insert(id, name);
                    add(Kind::Messages, "Tool calls".into(), name.len() + input.to_string().len());
                }
                (Block::ToolResult { tool_use_id, content, images, .. }, _) => {
                    let tool = called.get(tool_use_id.as_str()).copied().unwrap_or("Other").to_string();
                    let chars = content.len() + images.len() * IMAGE_CHARS;
                    match results.iter_mut().find(|(name, _)| *name == tool) {
                        Some((_, total)) => *total += chars,
                        None => results.push((tool, chars)),
                    }
                }
                (Block::Image { .. }, _) => add(Kind::Messages, "Pictures".into(), IMAGE_CHARS),
                (Block::Opaque { value, .. }, _) => add(Kind::Messages, "Replies".into(), value.to_string().len()),
            }
        }
    }
    results.sort_by_key(|(_, chars)| std::cmp::Reverse(*chars));
    let other: usize = results.iter().skip(RESULT_PIECES).map(|(_, chars)| chars).sum();
    results.truncate(RESULT_PIECES);
    for (tool, chars) in results {
        add(Kind::ToolResults, tool, chars);
    }
    add(Kind::ToolResults, "Other".into(), other);

    let estimated: usize = parts.iter().flat_map(|(_, pieces)| pieces.iter().map(|(_, chars)| chars)).sum();
    let scale = if measured > 0 && estimated > 0 { measured as f64 / estimated as f64 } else { 0.25 };
    let order = [
        Kind::Instructions,
        Kind::Tools,
        Kind::Project,
        Kind::Skills,
        Kind::Memory,
        Kind::Summary,
        Kind::Messages,
        Kind::ToolResults,
    ];
    let mut out: Vec<Part> = parts
        .into_iter()
        .map(|(kind, pieces)| {
            let pieces: Vec<(String, u64)> = pieces.into_iter().map(|(name, chars)| (name, (chars as f64 * scale).round() as u64)).collect();
            Part { kind, tokens: pieces.iter().map(|(_, tokens)| tokens).sum(), pieces }
        })
        .filter(|part| part.tokens > 0)
        .collect();
    out.sort_by_key(|part| order.iter().position(|kind| *kind == part.kind));
    out
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

/// An instruction file as the popover names it: its folder and name.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.trim().rsplit('/').take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wack_llm::{Message, SystemPart, ToolDef};

    use super::*;

    fn request() -> Request {
        Request {
            model: "m".into(),
            system: vec![
                SystemPart { text: "core ".repeat(40) },
                SystemPart { text: "guide ".repeat(20) },
                SystemPart { text: format!("# Project instructions\n\nintro\n\n## /code/app/AGENTS.md\n\n{}", "rule ".repeat(30)) },
                SystemPart { text: "# Skills\n\n- pdf: reads PDFs\n- notes: writes notes".into() },
            ],
            messages: vec![
                Message { role: Role::User, content: vec![Block::Text { text: "fix the tests".into() }] },
                Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolUse { id: "a".into(), name: "read".into(), input: json!({"path": "src/lib.rs"}) }],
                },
                Message {
                    role: Role::User,
                    content: vec![Block::ToolResult { tool_use_id: "a".into(), content: "x".repeat(400), is_error: false, images: Vec::new() }],
                },
            ],
            tools: vec![
                ToolDef { name: "read".into(), description: "Reads a file".into(), input_schema: json!({}), deferred: false },
                ToolDef { name: "mcp__notes__find".into(), description: "Finds".into(), input_schema: json!({}), deferred: false },
            ],
            max_output_tokens: None,
            effort: None,
        }
    }

    #[test]
    fn the_parts_add_up_to_what_was_measured() {
        let parts = breakdown(&request(), 1_000);
        let total: u64 = parts.iter().map(|part| part.tokens).sum();
        assert!(total.abs_diff(1_000) <= parts.iter().map(|part| part.pieces.len() as u64).sum::<u64>(), "{total}");
        let kinds: Vec<Kind> = parts.iter().map(|part| part.kind).collect();
        assert_eq!(kinds, [Kind::Instructions, Kind::Tools, Kind::Project, Kind::Skills, Kind::Messages, Kind::ToolResults]);
    }

    #[test]
    fn each_part_names_its_pieces() {
        let parts = breakdown(&request(), 0);
        let pieces = |kind: Kind| parts.iter().find(|part| part.kind == kind).unwrap().pieces.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
        assert_eq!(pieces(Kind::Project), ["app/AGENTS.md"]);
        assert_eq!(pieces(Kind::Skills), ["2 skills"]);
        assert_eq!(pieces(Kind::Tools), ["1 built-in tool", "1 MCP tool"]);
        assert_eq!(pieces(Kind::ToolResults), ["read"]);
        assert_eq!(pieces(Kind::Messages), ["You", "Tool calls"]);

        // A customised Ultra instruction at the end of `system` still counts as "Ultra".
        let mut with_custom_ultra = request();
        with_custom_ultra.system.push(SystemPart { text: "Custom thorough instructions.".into() });
        let parts = breakdown(&with_custom_ultra, 0);
        assert_eq!(pieces_of(&parts, Kind::Instructions), ["How WackCode works", "Guide to this chat", "Ultra"]);
    }

    fn pieces_of(parts: &[Part], kind: Kind) -> Vec<String> {
        parts.iter().find(|part| part.kind == kind).unwrap().pieces.iter().map(|(name, _)| name.clone()).collect()
    }
}
