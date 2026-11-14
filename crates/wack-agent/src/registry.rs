//! The tools an agent can call, and which of them each request carries.
//!
//! Deferred tools reach the model in one of two ways. A provider with its own tool search gets
//! every deferred tool marked as such and loads them itself. Any other provider gets our
//! `tool_search` tool, and the tools it finds are added to the end of the list: the request's
//! start stays byte-identical, which is what keeps the provider's prompt cache valid.

use std::sync::Arc;

use serde_json::json;
use wack_llm::ToolDef;

use crate::tool::{Loading, Tool, ToolSpec};

/// The name of our own search tool. The loop answers it itself: see `Agent::tool_search`.
pub const TOOL_SEARCH: &str = "tool_search";
const SEARCH_RESULTS: usize = 5;

pub struct Registry {
    tools: Vec<(ToolSpec, Arc<dyn Tool>)>,
}

impl Registry {
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Result<Self, String> {
        let mut entries: Vec<(ToolSpec, Arc<dyn Tool>)> = Vec::new();
        for tool in tools {
            let spec = tool.spec();
            if spec.name == TOOL_SEARCH || entries.iter().any(|(other, _)| other.name == spec.name) {
                return Err(format!("Two tools are named {}.", spec.name));
            }
            entries.push((spec, tool));
        }
        Ok(Self { tools: entries })
    }

    pub fn get(&self, name: &str) -> Option<(&ToolSpec, &Arc<dyn Tool>)> {
        self.tools.iter().find(|(spec, _)| spec.name == name).map(|(spec, tool)| (spec, tool))
    }

    pub fn specs(&self) -> impl Iterator<Item = &ToolSpec> {
        self.tools.iter().map(|(spec, _)| spec)
    }

    fn deferred(&self) -> impl Iterator<Item = &ToolSpec> {
        self.specs().filter(|spec| spec.loading == Loading::Deferred)
    }

    /// The tool list for one request. `loaded` is the session's record of deferred tools found so
    /// far, in the order they were found.
    pub fn defs(&self, loaded: &[String], native_search: bool) -> Vec<ToolDef> {
        let def = |spec: &ToolSpec, deferred: bool| ToolDef {
            name: spec.name.clone(),
            description: spec.description.clone(),
            input_schema: spec.input_schema.clone(),
            deferred,
        };
        let mut defs: Vec<ToolDef> =
            self.specs().filter(|spec| spec.loading == Loading::Always).map(|spec| def(spec, false)).collect();
        if native_search {
            defs.extend(self.deferred().map(|spec| def(spec, true)));
            return defs;
        }
        if self.deferred().next().is_some() {
            defs.push(ToolDef {
                name: TOOL_SEARCH.to_string(),
                description: "Finds more tools by what they do and loads them. The tools it returns can be called from your next step onward.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "A tool's name, or a few words for what you need to do." }
                    },
                    "required": ["query"]
                }),
                deferred: false,
            });
        }
        for name in loaded {
            if let Some(spec) = self.deferred().find(|spec| &spec.name == name) {
                defs.push(def(spec, false));
            }
        }
        defs
    }

    /// The deferred tools that best match a query, best first.
    pub fn search(&self, query: &str) -> Vec<&ToolSpec> {
        let words: Vec<String> = words(query);
        let mut scored: Vec<(usize, &ToolSpec)> = self
            .deferred()
            .filter_map(|spec| {
                let name = spec.name.to_lowercase();
                let about = format!("{} {}", spec.hint, spec.description).to_lowercase();
                let score: usize = words
                    .iter()
                    .map(|word| if name.contains(word) { 3 } else if about.contains(word) { 1 } else { 0 })
                    .sum();
                (score > 0).then_some((score, spec))
            })
            .collect();
        // A stable sort keeps registration order among equals, so results never shuffle.
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        scored.into_iter().take(SEARCH_RESULTS).map(|(_, spec)| spec).collect()
    }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() > 1)
        .map(str::to_string)
        .collect()
}
