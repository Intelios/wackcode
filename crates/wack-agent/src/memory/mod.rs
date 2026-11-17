//! Memory: small per-project notes the agent keeps for future conversations, ported from
//! version 2 with its tool descriptions and guidelines word for word.
//!
//! - Three tools, one job each: `memory_save` writes or updates a note, `memory_recall` reads
//!   notes in full, `memory_forget` moves one to the folder's `.trash`.
//! - The system prompt carries only the index ([`store::index`]), read once when the run's kit is
//!   built. A save never rewrites the prompt mid-run, so the prompt cache holds and the run sees
//!   a stable list; the next run picks the change up.
//! - The folder is the engine's to choose (under the app's data, never the workspace) and to
//!   withhold: memory switched off means no [`Memory`] at all, so no tools and no index.
//! - Not parallel-safe: two saves in one reply would race on the same numbered name. Plan mode
//!   lets these through by name, as version 2 did.

pub mod store;

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec};
use store::{Kind, MAX_BODY_CHARS, MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS};

pub const MEMORY_SAVE: &str = "memory_save";
pub const MEMORY_RECALL: &str = "memory_recall";
pub const MEMORY_FORGET: &str = "memory_forget";
pub const MEMORY_TOOLS: [&str; 3] = [MEMORY_SAVE, MEMORY_RECALL, MEMORY_FORGET];
const MAX_RECALL_NAMES: usize = 8;

/// One project's memory folder, for one run.
#[derive(Clone, Debug)]
pub struct Memory {
    root: PathBuf,
}

impl Memory {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The index section for the system prompt, as the folder stands now; `None` with no notes.
    pub fn index(&self) -> Option<String> {
        store::index(&store::scan(&self.root))
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(Save(self.root.clone())),
            Arc::new(Recall(self.root.clone())),
            Arc::new(Forget(self.root.clone())),
        ]
    }
}

struct Save(PathBuf);
struct Recall(PathBuf);
struct Forget(PathBuf);

fn one_line(value: &str, max: usize) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

fn text<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or("")
}

#[async_trait]
impl Tool for Save {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: MEMORY_SAVE.into(),
            description: "Save or update a small note for future conversations in this project. Four kinds: user (the user's role and lasting preferences), feedback (corrections and confirmed approaches), project (ongoing work, decisions, deadlines not derivable from the code or git history), reference (where information lives outside the project).".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["type", "title", "description", "body"],
                "properties": {
                    "type": { "type": "string", "enum": ["user", "feedback", "project", "reference"], "description": "user, feedback, project or reference." },
                    "title": { "type": "string", "description": "Short name for the note, shown in the memory index every session." },
                    "description": { "type": "string", "description": "One line saying when the note matters, shown next to the title in the index." },
                    "body": { "type": "string", "description": "The note itself. Keep it as small as it can be and still be useful." },
                    "name": { "type": "string", "description": "The existing note to update, as the index spells it. Omit to create a new one." }
                }
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "keep a small note for future conversations".into(),
            guidelines: vec![
                "Memory (memory_save, memory_recall, memory_forget): keep lasting preferences, explained corrections and project facts absent from code. Update existing notes; verify recalled paths and flags.".into(),
                "Save a memory only when it would help a future conversation, not every session. Never save what the codebase, AGENTS.md files or the transcript already show, or anything true only of the current task.".into(),
                "Updating beats accumulating: pass the existing note's name to rewrite it, and retire notes that no longer hold with memory_forget.".into(),
            ],
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let kind = Kind::parse(text(&input, "type")).ok_or("type must be one of user, feedback, project, reference.")?;
        let title = one_line(text(&input, "title"), MAX_TITLE_CHARS);
        let description = one_line(text(&input, "description"), MAX_DESCRIPTION_CHARS);
        let body: String = text(&input, "body").trim().chars().take(MAX_BODY_CHARS).collect();
        if title.is_empty() {
            return Err("Give the memory a title.".into());
        }
        if body.is_empty() {
            return Err("Give the memory a body.".into());
        }
        let requested = Some(text(&input, "name").trim()).filter(|name| !name.is_empty());
        if requested.is_some_and(|name| !store::is_note_name(name)) {
            return Err("Names use only lowercase letters, numbers, hyphens and underscores, like feedback_run-tests.".into());
        }
        let name = match requested {
            Some(name) => name.to_string(),
            None => store::available_name(&self.0, &store::note_name(kind, &title)),
        };
        store::write(&self.0, &name, kind, &title, &description, &body)?;
        Ok(format!("{} {name}: {title}", if requested.is_some() { "Updated" } else { "Saved" }).into())
    }
}

#[async_trait]
impl Tool for Recall {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: MEMORY_RECALL.into(),
            description: "Read saved memory notes in full. The memory index in the system prompt lists every note's name; call this with the names that look relevant to the current request.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["names"],
                "properties": {
                    "names": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": MAX_RECALL_NAMES, "description": "Note names from the memory index." }
                }
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "read a saved memory note in full".into(),
            guidelines: vec![
                "Recall a note when its index line looks relevant to the request; the title and description are only a summary of the body.".into(),
            ],
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let names: Vec<String> = input
            .get("names")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .map(|name| match name {
                        Value::String(name) => name.trim().to_string(),
                        other => other.to_string(),
                    })
                    .filter(|name| !name.is_empty())
                    .take(MAX_RECALL_NAMES)
                    .collect()
            })
            .unwrap_or_default();
        if names.is_empty() {
            return Err("Give memory_recall at least one note name.".into());
        }
        let parts: Vec<String> = names
            .iter()
            .map(|name| match store::read(&self.0, name) {
                Some(note) => {
                    let when = note.modified.map(|modified| format!("\nSaved {modified}")).unwrap_or_default();
                    format!("# {} ({}){when}\n{}", note.title, note.kind.as_str(), note.body)
                }
                None => format!("No memory named '{name}'."),
            })
            .collect();
        Ok(parts.join("\n\n").into())
    }
}

#[async_trait]
impl Tool for Forget {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: MEMORY_FORGET.into(),
            description: "Retire a saved memory note that no longer holds. It moves to the memory folder's .trash, out of the index.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "The note to retire, as the index spells it." }
                }
            }),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "retire a saved memory note".into(),
            guidelines: Vec::new(),
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let name = text(&input, "name").trim();
        if name.is_empty() {
            return Err("Give memory_forget the note's name.".into());
        }
        // A name that could not be a note file (a path, a dot file) is simply not found.
        if store::read(&self.0, name).is_none() {
            return Err(format!("No memory named '{name}'."));
        }
        store::trash(&self.0, name)?;
        Ok(format!("Forgot {name}.").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn call(memory: &Memory, name: &str, input: Value) -> Result<String, String> {
        let tool = memory.tools().into_iter().find(|tool| tool.spec().name == name).unwrap();
        tool.run(input, &ToolContext::detached("/tmp")).await.map(|output| output.text)
    }

    #[tokio::test]
    async fn save_index_recall_and_forget() {
        let folder = tempfile::tempdir().unwrap();
        let memory = Memory::new(folder.path());
        assert!(memory.index().is_none());

        let saved = call(&memory, MEMORY_SAVE, json!({
            "type": "feedback", "title": "Run   the tests\nfirst", "description": "Before review", "body": "  cargo test --workspace  "
        }))
        .await
        .unwrap();
        assert_eq!(saved, "Saved feedback_run-the-tests-first: Run the tests first");
        let index = memory.index().unwrap();
        assert!(index.contains("- [feedback] Run the tests first — Before review (name: feedback_run-the-tests-first)"));

        let recalled = call(&memory, MEMORY_RECALL, json!({ "names": ["feedback_run-the-tests-first", "user_nope", "../escape"] })).await.unwrap();
        let mut parts = recalled.split("\n\n");
        let first = parts.next().unwrap();
        assert!(first.starts_with("# Run the tests first (feedback)\nSaved "));
        assert!(first.ends_with("\ncargo test --workspace"));
        assert_eq!(parts.next(), Some("No memory named 'user_nope'."));
        assert_eq!(parts.next(), Some("No memory named '../escape'."));

        assert_eq!(call(&memory, MEMORY_FORGET, json!({ "name": "feedback_run-the-tests-first" })).await.unwrap(), "Forgot feedback_run-the-tests-first.");
        assert!(memory.index().is_none());
        assert!(folder.path().join(".trash/feedback_run-the-tests-first.md").exists());
        assert_eq!(call(&memory, MEMORY_FORGET, json!({ "name": "feedback_run-the-tests-first" })).await.unwrap_err(), "No memory named 'feedback_run-the-tests-first'.");
    }

    #[tokio::test]
    async fn saves_update_by_name_and_number_collisions() {
        let folder = tempfile::tempdir().unwrap();
        let memory = Memory::new(folder.path());
        let input = json!({ "type": "user", "title": "Tabs", "description": "", "body": "Prefers tabs" });
        assert_eq!(call(&memory, MEMORY_SAVE, input.clone()).await.unwrap(), "Saved user_tabs: Tabs");
        assert_eq!(call(&memory, MEMORY_SAVE, input).await.unwrap(), "Saved user_tabs-2: Tabs");
        let update = json!({ "type": "user", "title": "Tabs", "description": "", "body": "Width 4", "name": "user_tabs" });
        assert_eq!(call(&memory, MEMORY_SAVE, update).await.unwrap(), "Updated user_tabs: Tabs");
        assert_eq!(store::read(folder.path(), "user_tabs").unwrap().body, "Width 4");
        assert_eq!(store::scan(folder.path()).len(), 2);
    }

    #[tokio::test]
    async fn bad_input_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let memory = Memory::new(folder.path());
        let base = |extra: Value| {
            let mut input = json!({ "type": "user", "title": "T", "description": "", "body": "b" });
            input.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            input
        };
        assert!(call(&memory, MEMORY_SAVE, base(json!({ "type": "diary" }))).await.unwrap_err().starts_with("type must be one of"));
        assert_eq!(call(&memory, MEMORY_SAVE, base(json!({ "title": " " }))).await.unwrap_err(), "Give the memory a title.");
        assert_eq!(call(&memory, MEMORY_SAVE, base(json!({ "body": "" }))).await.unwrap_err(), "Give the memory a body.");
        for bad in ["../escape", "a/b", ".hidden", "Has Space", "UPPER"] {
            assert!(call(&memory, MEMORY_SAVE, base(json!({ "name": bad }))).await.unwrap_err().starts_with("Names use only"), "{bad}");
        }
        assert_eq!(call(&memory, MEMORY_RECALL, json!({ "names": [] })).await.unwrap_err(), "Give memory_recall at least one note name.");
        assert_eq!(call(&memory, MEMORY_FORGET, json!({})).await.unwrap_err(), "Give memory_forget the note's name.");
        assert!(std::fs::read_dir(folder.path()).unwrap().next().is_none(), "nothing was written");
    }
}
