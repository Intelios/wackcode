//! Task suggestions: the `suggest_task` tool the agent files a follow-up with — a bug, a rough
//! edge or a missing convenience it noticed but should not work on now.
//!
//! Ported from version 2's `builtin/suggest-task.ts` with its schema, description, guidelines and
//! messages word for word.
//!
//! - The tool only records: it writes nothing to disk and changes nothing the user sees while a
//!   run works. The card under the turn's final message is drawn by the app from the call's saved
//!   arguments, so saved history and a cold reopen need nothing extra — the same discipline as
//!   version 2, whose `details` marker version 3 does not need: a successful result is the
//!   acceptance, and a refused one is an error the work log already shows.
//! - Two guards keep a model that can file tasks from filing too many: at most [`MAX_PER_RUN`]
//!   per run (a run is one kit, so a counter on this feature replaces version 2's `agent_end`
//!   reset), and a title already accepted in this chat is refused. The titles live in the
//!   session's `suggestions` state where version 2 scanned the branch, so a restart *and* a
//!   compaction both keep them.
//! - Parallel-safe reads as read-only to Plan mode, the same permission version 2 gave it.
//!   Sub-agents never get it (`agents::child::PARENT_ONLY`): a child has no transcript to draw a
//!   card in, so it reports what it noticed and the parent decides.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    agent::Feature,
    session::Session,
    tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec},
};

pub const SUGGEST_TASK: &str = "suggest_task";
/// The session state key holding the titles already filed: `{"titles": [...]}`.
pub const SUGGESTIONS_STATE: &str = "suggestions";
pub const MAX_TITLE: usize = 80;
pub const MAX_DESCRIPTION: usize = 2_000;
/// Suggestions one run may file. Further calls are refused, so a turn cannot become a backlog.
pub const MAX_PER_RUN: usize = 5;
pub const KINDS: [&str; 3] = ["bug", "polish", "improvement"];

/// A call's input, checked for shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub title: String,
    pub kind: String,
    pub description: String,
}

/// A title as both the card and the duplicate check see it: one line, one space between words.
fn clean_title(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The duplicate key: case and spacing never make a second suggestion distinct.
fn title_key(title: &str) -> String {
    clean_title(title).to_lowercase()
}

/// Checks a call's shape, porting version 2's `normalizeSuggestTaskParams` and its messages.
pub fn normalize(input: &Value) -> Result<Suggestion, String> {
    if !input.is_object() {
        return Err("Expected an object with a title, a kind and a description.".to_string());
    }
    let title = input["title"].as_str().map(clean_title).unwrap_or_default();
    if title.is_empty() {
        return Err("title must be a short non-empty string.".to_string());
    }
    if title.chars().count() > MAX_TITLE {
        return Err(format!("title must be at most {MAX_TITLE} characters."));
    }
    let kind = input["kind"].as_str().unwrap_or_default();
    if !KINDS.contains(&kind) {
        return Err(format!("kind must be one of {}.", KINDS.join(", ")));
    }
    let description = input["description"].as_str().map(str::trim).unwrap_or_default();
    if description.is_empty() {
        return Err("description must be a non-empty Markdown brief.".to_string());
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(format!(
            "description is {} characters, over the {MAX_DESCRIPTION} limit. Keep the brief to what a fresh agent needs.",
            description.chars().count()
        ));
    }
    Ok(Suggestion { title, kind: kind.to_string(), description: description.to_string() })
}

struct State {
    /// Titles already accepted, as [`title_key`] makes them.
    titles: Vec<String>,
    /// Filed this run.
    filed: usize,
}

/// The suggestions feature and its tool, sharing what this chat has already filed.
#[derive(Clone)]
pub struct Suggestions {
    state: Arc<Mutex<State>>,
}

impl Suggestions {
    pub fn new(session: &Session) -> Self {
        let titles = session
            .state(SUGGESTIONS_STATE)
            .and_then(|value| value.get("titles").cloned())
            .and_then(|titles| titles.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|title| title.as_str().map(title_key))
            .collect();
        Self { state: Arc::new(Mutex::new(State { titles, filed: 0 })) }
    }

    pub fn tool(&self) -> Arc<dyn Tool> {
        Arc::new(self.clone())
    }
}

impl Feature for Suggestions {
    fn state_to_save(&self) -> Vec<(String, Value)> {
        let state = self.state.lock().unwrap();
        if state.titles.is_empty() {
            return Vec::new();
        }
        vec![(SUGGESTIONS_STATE.to_string(), json!({ "titles": state.titles }))]
    }
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "kind", "description"],
        "properties": {
            "title": {
                "type": "string",
                "description": "A short imperative title for the follow-up task, 1-80 characters, e.g. \"Dismiss button stays disabled after a failed save\"."
            },
            "kind": {
                "type": "string",
                "enum": KINDS,
                "description": "bug: something is broken. polish: it works but looks or feels rough. improvement: a missing convenience."
            },
            "description": {
                "type": "string",
                "description": "A self-contained Markdown brief for a fresh agent with none of this conversation: what is wrong, where (file paths and lines), how you noticed it, and the fix you would make. At most 2000 characters."
            }
        }
    })
}

#[async_trait]
impl Tool for Suggestions {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: SUGGEST_TASK.into(),
            description: "File a follow-up task you noticed while working — a bug, a rough edge or missing polish outside what you were asked to do. It is recorded, not worked on: the user gets a card after your final message and decides whether to start it here or in a new chat. Write the description as a brief a fresh agent could act on without this conversation.".into(),
            input_schema: schema(),
            loading: Loading::Always,
            // Recording only: it changes nothing, so Plan mode allows it.
            parallel_safe: true,
            hint: "file a follow-up task you noticed but are not doing now".into(),
            guidelines: vec![
                "Use suggest_task for a concrete problem you noticed that is outside the task you were asked to do: a bug nearby, a rough edge, a missing bit of polish or convenience. It records the task; the user sees a card with it after your final message and decides whether to start it.".into(),
                "Never use it for work you are going to do in this task, for ideas you are not sure about, or for a list of nice-to-haves. Prefer no suggestion to a weak one.".into(),
                "Write the description as a brief a fresh agent could act on with no access to this conversation: what is wrong, where it is (file paths and line numbers), how you noticed it, and what you would change.".into(),
                "File each suggestion once; a repeated title is refused. Don't work on a suggested task now, and mention it only in passing in your final message — the card carries the details.".into(),
            ],
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let suggestion = normalize(&input)?;
        let mut state = self.state.lock().unwrap();
        let key = title_key(&suggestion.title);
        if state.titles.contains(&key) {
            return Err(format!("“{}” is already suggested in this chat. File a different task, or do this one instead.", suggestion.title));
        }
        if state.filed >= MAX_PER_RUN {
            return Err(format!(
                "This run already filed {MAX_PER_RUN} suggested tasks, which is the limit. Keep only the important ones, or finish the work you were asked to do."
            ));
        }
        state.filed += 1;
        state.titles.push(key);
        Ok(format!(
            "Suggested “{}”. It appears to the user as a card under your final message, with actions to start it later. Don't work on it now, and don't repeat its details in your reply.",
            suggestion.title
        ).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggest(title: &str) -> Value {
        json!({ "title": title, "kind": "bug", "description": "What is wrong and the fix." })
    }

    #[test]
    fn input_is_checked_for_shape() {
        assert!(normalize(&json!("nope")).is_err());
        assert!(normalize(&json!({ "kind": "bug", "description": "x" })).is_err());
        assert_eq!(normalize(&json!({ "title": "   ", "kind": "bug", "description": "x" })).unwrap_err(), "title must be a short non-empty string.");
        let long = "a".repeat(MAX_TITLE + 1);
        assert_eq!(normalize(&suggest(&long)).unwrap_err(), format!("title must be at most {MAX_TITLE} characters."));
        assert_eq!(normalize(&json!({ "title": "t", "kind": "idea", "description": "x" })).unwrap_err(), "kind must be one of bug, polish, improvement.");
        assert_eq!(normalize(&json!({ "title": "t", "kind": "bug" })).unwrap_err(), "description must be a non-empty Markdown brief.");
        let brief = "x".repeat(MAX_DESCRIPTION + 1);
        let error = normalize(&json!({ "title": "t", "kind": "bug", "description": brief })).unwrap_err();
        assert!(error.contains(&format!("over the {MAX_DESCRIPTION} limit")), "{error}");
    }

    #[test]
    fn the_title_is_cleaned() {
        let suggestion = normalize(&json!({ "title": "  Fix   the\n  bug ", "kind": "polish", "description": " d " })).unwrap();
        assert_eq!(suggestion.title, "Fix the bug");
        assert_eq!(suggestion.kind, "polish");
        assert_eq!(suggestion.description, "d");
    }

    #[tokio::test]
    async fn duplicates_and_the_run_cap_are_refused() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let suggestions = Suggestions::new(&session);
        let context = ToolContext::detached("/tmp");

        let filed = suggestions.run(suggest("One bug"), &context).await.unwrap().text;
        assert!(filed.starts_with("Suggested “One bug”"));

        // Spacing and case never make a second suggestion distinct.
        let again = suggestions.run(suggest("one   BUG"), &context).await.unwrap_err();
        assert!(again.contains("is already suggested in this chat"), "{again}");

        for index in 2..=MAX_PER_RUN {
            suggestions.run(suggest(&format!("Thing {index}")), &context).await.unwrap();
        }
        let capped = suggestions.run(suggest("One more"), &context).await.unwrap_err();
        assert!(capped.contains("which is the limit"), "{capped}");

        // The accepted titles save as state and reload into a fresh run.
        let saved = suggestions.state_to_save();
        assert_eq!(saved.len(), 1);
        session.set_state(&saved[0].0, saved[0].1.clone()).unwrap();
        let next = Suggestions::new(&session);
        assert!(next.run(suggest("ONE bug"), &context).await.unwrap_err().contains("already suggested"));
        // A fresh run counts again.
        assert!(next.run(suggest("A new one"), &context).await.is_ok());
    }

    #[tokio::test]
    async fn a_chat_with_no_suggestions_saves_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let suggestions = Suggestions::new(&session);
        assert!(suggestions.state_to_save().is_empty());
    }

    #[test]
    fn the_spec_runs_while_planning() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let spec = Suggestions::new(&session).spec();
        assert_eq!(spec.name, SUGGEST_TASK);
        assert!(spec.parallel_safe);
        assert!(!spec.ends_run && !spec.together);
    }
}
