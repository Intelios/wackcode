//! `ComputerFeature`: what the agent loop learns of computer use between tool calls.
//!
//! Invariant: a compaction keeps which apps the chat may use, so the model doesn't ask the user
//! again for an app they already allowed.

use wack_agent::Feature;

use crate::computer::ChatComputer;

pub struct ComputerFeature {
    chat: ChatComputer,
}

impl ComputerFeature {
    pub fn new(chat: ChatComputer) -> Self {
        Self { chat }
    }
}

impl Feature for ComputerFeature {
    fn summary_sections(&self) -> Vec<String> {
        let apps: Vec<String> = self
            .chat
            .allowed()
            .into_iter()
            .filter_map(|key| {
                let path = key.path?;
                let name = std::path::Path::new(&path).file_stem()?.to_string_lossy().into_owned();
                Some(format!("- {name} ({})", key.bundle_id.unwrap_or(path)))
            })
            .collect();
        if apps.is_empty() {
            return Vec::new();
        }
        vec![format!("## Computer use\nApps the user allowed this chat to use through the computer_* tools:\n{}", apps.join("\n"))]
    }
}
