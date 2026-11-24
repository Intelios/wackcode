//! The Chat area's kit, the counterpart of [`crate::Kit`] for a chat with no project: the
//! question tool, the Chat area's shared memory and the picture tools. There is no Plan mode,
//! todo list, task suggestions, skills, shell tasks or sub-agents: a Chat chat is always in Build
//! and its run never offers them (version 2's Chat mode allowlist, `chat-mode/policy.ts`).
//!
//! [`ChatGuard`] is the allowlist's second line. The engine builds a Chat run's registry only
//! from tools on the list (`wack-core/src/run.rs`), and the guard refuses any call to a tool off
//! it, so a tool added to the engine later is shut out until it is put on the list on purpose.

use std::{path::PathBuf, sync::Arc};

use serde_json::Value;

use crate::{
    agent::{Agent, Feature, Verdict},
    ask::{ASK_USER_QUESTION, AskUserQuestion},
    images::tools::{LIST_IMAGES, READ_IMAGE},
    memory::{MEMORY_TOOLS, Memory},
    registry::TOOL_SEARCH,
    tool::{Tool, ToolSpec},
};

/// The file tools a Chat chat keeps, all confined to its scratchpad.
pub const CHAT_FILE_TOOLS: [&str; 6] = ["read", "write", "edit", "ls", "grep", "find"];

/// Every other tool a Chat chat may have, each still subject to its own switch in Settings.
/// `browser_preview` is left out: it starts a project's Run command, and there is no project.
const CHAT_TOOLS: [&str; 28] = [
    ASK_USER_QUESTION,
    LIST_IMAGES,
    READ_IMAGE,
    TOOL_SEARCH,
    MEMORY_TOOLS[0],
    MEMORY_TOOLS[1],
    MEMORY_TOOLS[2],
    "web_fetch",
    "web_search",
    "read_chat",
    "canvas",
    "canvas_guide",
    "canvas_look",
    "browser_batch",
    "browser_tabs",
    "browser_navigate",
    "browser_snapshot",
    "browser_screenshot",
    "browser_act",
    "browser_fill",
    "browser_wait",
    "browser_text",
    "browser_console",
    "browser_network",
    "browser_viewport",
    "browser_dialog",
    "browser_eval",
    // Its files must be in the scratchpad: it refuses any outside the working folder.
    "browser_upload",
];

/// Whether a Chat chat may have a tool of this name: one on the list, or one of the user's MCP
/// servers' (`mcp__<server>__<tool>`). Unknown names stay out.
pub fn chat_tool_allowed(name: &str) -> bool {
    CHAT_FILE_TOOLS.contains(&name) || CHAT_TOOLS.contains(&name) || name.starts_with("mcp__")
}

pub struct ChatKit {
    /// The Chat area's memory, and its index as it stood when the run started.
    memory: Option<(Memory, Option<String>)>,
}

impl ChatKit {
    /// `memory` is the Chat area's memory folder, or `None` when memory is off for it.
    pub fn new(memory: Option<PathBuf>) -> Self {
        let memory = memory.map(|root| {
            let memory = Memory::chat(root);
            let index = memory.index();
            (memory, index)
        });
        Self { memory }
    }

    /// The tools this kit adds to a Chat chat's own, in the order the model is shown them.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        // There is no Ultra Plan interview here, so a question never belongs to one.
        let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(AskUserQuestion::new(Arc::new(|| false)))];
        if let Some((memory, _)) = &self.memory {
            tools.extend(memory.tools());
        }
        tools.extend(crate::images::tools::tools());
        tools
    }

    /// Hooks the kit into an agent built with [`crate::Profile::chat`].
    pub fn attach(&self, mut agent: Agent) -> Agent {
        if let Some((_, Some(index))) = &self.memory {
            agent = agent.with_system_part(index.clone());
        }
        agent.with_feature(Box::new(ChatGuard))
    }
}

/// Refuses any tool a Chat chat may not have, whatever put it in the registry.
pub struct ChatGuard;

impl Feature for ChatGuard {
    fn on_tool_call(&self, tool: &ToolSpec, _: &Value) -> Verdict {
        if chat_tool_allowed(&tool.name) {
            Verdict::Allow
        } else {
            Verdict::Deny(format!("'{}' is not available in Chat mode.", tool.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_file_tools_and_drops_the_shell() {
        for name in CHAT_FILE_TOOLS {
            assert!(chat_tool_allowed(name), "{name}");
        }
        assert!(!chat_tool_allowed("bash"));
    }

    #[test]
    fn keeps_questions_web_memory_canvas_and_the_browser_and_nothing_else() {
        for name in ["ask_user_question", "web_fetch", "web_search", "memory_save", "canvas", "browser_navigate", "read_chat", "tool_search"] {
            assert!(chat_tool_allowed(name), "{name}");
        }
        for name in [
            "stop_task",
            "wait_for_tasks",
            "watch",
            "todo",
            "plan_mode_enter",
            "plan_mode_complete",
            "agent",
            "agent_message",
            "suggest_task",
            "browser_preview",
            "anything_else",
        ] {
            assert!(!chat_tool_allowed(name), "{name}");
        }
    }

    #[test]
    fn keeps_the_users_mcp_tools() {
        assert!(chat_tool_allowed("mcp__notes__search"));
        assert!(!chat_tool_allowed("mcp_notes"));
    }

    #[test]
    fn the_kit_offers_only_allowed_tools() {
        let folder = tempfile::tempdir().unwrap();
        for kit in [ChatKit::new(Some(folder.path().to_path_buf())), ChatKit::new(None)] {
            for tool in kit.tools() {
                assert!(chat_tool_allowed(&tool.spec().name), "{}", tool.spec().name);
            }
        }
        let names: Vec<String> = ChatKit::new(None).tools().iter().map(|tool| tool.spec().name).collect();
        assert!(!names.iter().any(|name| name.starts_with("memory_")), "memory off means no memory tools");
    }
}
