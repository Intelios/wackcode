//! The todo list: a task list the agent keeps current while it works, shown above the composer.
//!
//! Ported from version 2 (from `@juicesharp/rpiv-todo` v2.11.0, MIT). The description and
//! guidelines are version 2's, word for word: they encode why the tool exists, keeping the agent
//! honest about multi-step progress.
//!
//! - The list lives in the session under the `todo` state key, saved after every call that
//!   changed it, so it survives a restart and is untouched by compaction.
//! - A compaction would otherwise leave the model without its list (it was only ever in tool
//!   results), so the list is written into every summary in code (`summary_sections`).
//! - Several `todo` calls in one reply run in the model's order: the tool is not parallel-safe.

mod format;
mod reducer;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use format::{clean, list_line};
pub use reducer::{Action, Op};

use crate::{
    agent::Feature,
    session::Session,
    tool::{Loading, Tool, ToolContext, ToolOutput, ToolSpec},
};

pub const TODO: &str = "todo";
/// The session state key holding the [`TodoList`].
pub const TODO_STATE: &str = "todo";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    InProgress,
    Completed,
    /// A tombstone: the task stays in the list, hidden.
    Deleted,
}

impl Status {
    pub const NAMES: [&str; 4] = ["pending", "in_progress", "completed", "deleted"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "deleted" => Self::Deleted,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: u64,
    /// Short and imperative.
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Present-continuous, shown while the task is in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    pub status: Status,
    /// Tasks that must be done before this one can start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoList {
    pub tasks: Vec<Task>,
    pub next_id: u64,
}

impl Default for TodoList {
    fn default() -> Self {
        Self { tasks: Vec::new(), next_id: 1 }
    }
}

impl TodoList {
    /// Reads a saved list, dropping any task that is not well formed so a damaged line can
    /// never break the panel.
    pub fn from_value(value: &Value) -> Option<Self> {
        let tasks = value.get("tasks")?.as_array()?;
        let next_id = value.get("nextId")?.as_u64()?;
        Some(Self { tasks: tasks.iter().filter_map(|task| serde_json::from_value(task.clone()).ok()).collect(), next_id })
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn task(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    /// The tasks the user sees: everything but tombstones.
    pub fn visible(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(|task| task.status != Status::Deleted)
    }
}

/// A call's input, checked for shape. What the values mean is the reducer's business.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub status: Option<Status>,
    pub blocked_by: Option<Vec<u64>>,
    pub add_blocked_by: Option<Vec<u64>>,
    pub remove_blocked_by: Option<Vec<u64>>,
    pub id: Option<u64>,
    pub include_deleted: Option<bool>,
}

/// Checks a call's shape. The error is a phrase the model is shown.
pub fn normalize(input: &Value) -> Result<(Action, Input), String> {
    if !input.is_object() {
        return Err("parameters must be an object".to_string());
    }
    let Some(action) = input["action"].as_str().and_then(Action::parse) else {
        return Err(format!("action must be one of {}", Action::NAMES.join(", ")));
    };
    let present = |field: &str| input.get(field).filter(|value| !value.is_null());
    let text = |field: &str| match present(field) {
        None => Ok(None),
        Some(value) => value.as_str().map(|text| Some(text.to_string())).ok_or(format!("{field} must be a string")),
    };
    let id_list = |field: &str| match present(field) {
        None => Ok(None),
        Some(value) => value
            .as_array()
            .and_then(|list| list.iter().map(Value::as_u64).collect::<Option<Vec<u64>>>())
            .map(Some)
            .ok_or(format!("{field} must be an array of task ids")),
    };
    let status = match present("status") {
        None => None,
        Some(value) => {
            Some(value.as_str().and_then(Status::parse).ok_or(format!("status must be one of {}", Status::NAMES.join(", ")))?)
        }
    };
    let id = match present("id") {
        None => None,
        Some(value) => Some(value.as_u64().ok_or("id must be a number")?),
    };
    let include_deleted = match present("includeDeleted") {
        None => None,
        Some(value) => Some(value.as_bool().ok_or("includeDeleted must be a boolean")?),
    };
    Ok((
        action,
        Input {
            subject: text("subject")?,
            description: text("description")?,
            active_form: text("activeForm")?,
            status,
            blocked_by: id_list("blockedBy")?,
            add_blocked_by: id_list("addBlockedBy")?,
            remove_blocked_by: id_list("removeBlockedBy")?,
            id,
            include_deleted,
        },
    ))
}

const DESCRIPTION: &str = "Manage a task list for tracking multi-step progress. Actions: create (new task), update (change status/fields/dependencies), list (all tasks, optionally filtered by status), get (single task details), delete (tombstone), clear (reset all). Use this to plan and track multi-step work like research, design, and implementation.

Task status is a 4-state machine: pending → in_progress → completed, plus deleted as a tombstone. Pass activeForm (present-continuous label, e.g. 'researching existing tool') when marking in_progress.

To change a task's status, call update with the task id and the target status, e.g. {\"action\":\"update\",\"id\":3,\"status\":\"completed\"} or {\"action\":\"update\",\"id\":3,\"status\":\"in_progress\",\"activeForm\":\"writing tests\"}. status is the field that changes the task; an update without a mutable field (status or another) is rejected.

Use blockedBy to express dependencies (A is blocked by B). On create, pass blockedBy as the initial set. On update, use addBlockedBy / removeBlockedBy (additive merge — do not resend the full array). Cycles are rejected.

list hides tombstoned (deleted) tasks by default; pass includeDeleted:true to see them. Pass status to filter by a single status.

Subject must be short and imperative (e.g. 'Research existing tool'); description is for long-form detail.";

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["action"],
        "properties": {
            "action": {
                "type": "string",
                "enum": Action::NAMES,
                "description": "What to do: create (new task), update (change status, fields, or dependencies), list (all tasks, optionally filtered by status), get (single task with dependencies), delete (tombstone a task), clear (reset the whole list)."
            },
            "subject": { "type": "string", "description": "Task subject line (required for create)" },
            "description": { "type": "string", "description": "Long-form task description" },
            "activeForm": { "type": "string", "description": "Present-continuous label shown while status is in_progress (e.g. 'writing tests')" },
            "status": {
                "type": "string",
                "enum": Status::NAMES,
                "description": "Set this task's status (update), or filter returned tasks by status (list). One of pending, in_progress, completed, deleted."
            },
            "blockedBy": { "type": "array", "items": { "type": "number" }, "description": "Initial blockedBy ids (create only)" },
            "addBlockedBy": { "type": "array", "items": { "type": "number" }, "description": "Task ids to add to blockedBy (update only, additive merge)" },
            "removeBlockedBy": { "type": "array", "items": { "type": "number" }, "description": "Task ids to remove from blockedBy (update only)" },
            "id": { "type": "number", "description": "Task id (required for update, get, delete)" },
            "includeDeleted": { "type": "boolean", "description": "If true, list also returns deleted (tombstoned) tasks. Default: false." }
        }
    })
}

/// The todo feature and its tool, sharing one list.
#[derive(Clone)]
pub struct Todo {
    list: Arc<Mutex<TodoList>>,
    /// Whether the session already held a list, so a chat that never uses one saves nothing.
    saved: bool,
}

impl Todo {
    pub fn new(session: &Session) -> Self {
        let saved = session.state(TODO_STATE).and_then(TodoList::from_value);
        Self { saved: saved.is_some(), list: Arc::new(Mutex::new(saved.unwrap_or_default())) }
    }

    pub fn list(&self) -> TodoList {
        self.list.lock().unwrap().clone()
    }

    pub fn tool(&self) -> Arc<dyn Tool> {
        Arc::new(self.clone())
    }
}

impl Feature for Todo {
    fn state_to_save(&self) -> Vec<(String, Value)> {
        let list = self.list();
        if !self.saved && list == TodoList::default() {
            return Vec::new();
        }
        vec![(TODO_STATE.to_string(), list.to_value())]
    }

    fn summary_sections(&self) -> Vec<String> {
        let list = self.list();
        let lines: Vec<String> = list.visible().map(list_line).collect();
        if lines.is_empty() {
            return Vec::new();
        }
        vec![format!(
            "## Todo list\nYour task list as it stands, kept by the app. Keep working through it and keep it current with the todo tool.\n{}",
            lines.join("\n")
        )]
    }
}

#[async_trait]
impl Tool for Todo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TODO.into(),
            description: DESCRIPTION.into(),
            input_schema: schema(),
            loading: Loading::Always,
            parallel_safe: false,
            hint: "Manage a task list to track multi-step progress".into(),
            guidelines: vec![
                "Use `todo` for complex work with 3+ steps, when the user gives you a list of tasks, or immediately after receiving new instructions to capture requirements. Skip it for single trivial tasks and purely conversational requests.".into(),
                "When starting a task from the todo list, mark it in_progress BEFORE beginning work. Mark it completed IMMEDIATELY when done — never batch completions. Exactly one task in_progress at a time.".into(),
                "Never mark a task completed if tests are failing, the implementation is partial, or you hit unresolved errors — keep it in_progress and create a new task for the blocker instead.".into(),
                "You can emit several `todo` calls in one response — create all known tasks at once instead of one per turn. A task created in the same batch can't be referenced by id (e.g. as a blockedBy target) until the next turn.".into(),
            ],
            ends_run: false,
            together: false,
        }
    }

    async fn run(&self, input: Value, _: &ToolContext<'_>) -> Result<ToolOutput, String> {
        let (action, input) = normalize(&input)?;
        let mut list = self.list.lock().unwrap();
        let op = reducer::apply(&mut list, action, &input);
        Ok(format::content(&op, &list).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_checked_for_shape() {
        let (action, input) = normalize(&json!({ "action": "update", "id": 3, "status": "completed", "addBlockedBy": [1] })).unwrap();
        assert_eq!(action, Action::Update);
        assert_eq!(input, Input { id: Some(3), status: Some(Status::Completed), add_blocked_by: Some(vec![1]), ..Input::default() });
        assert!(normalize(&Value::Null).is_err());
        assert!(normalize(&json!({ "action": "explode" })).is_err());
        assert_eq!(normalize(&json!({ "action": "create", "subject": 5 })).unwrap_err(), "subject must be a string");
        assert_eq!(normalize(&json!({ "action": "create", "blockedBy": ["1"] })).unwrap_err(), "blockedBy must be an array of task ids");
        assert!(normalize(&json!({ "action": "list", "status": "sideways" })).is_err());
    }

    #[tokio::test]
    async fn the_list_is_saved_restored_and_carried_into_summaries() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = Session::create(&folder.path().join("chat.jsonl"), folder.path()).unwrap();
        let todo = Todo::new(&session);
        assert!(todo.state_to_save().is_empty(), "a chat with no list saves nothing");
        assert!(todo.summary_sections().is_empty());

        let context = ToolContext::detached("/tmp");
        let created = todo.run(json!({ "action": "create", "subject": "Write the parser" }), &context).await.unwrap().text;
        assert_eq!(created, "Created #1: Write the parser (pending)");
        // A rule broken is reported in the result, not as a failed call.
        assert_eq!(todo.run(json!({ "action": "get", "id": 9 }), &context).await.unwrap().text, "Error: #9 not found");
        assert!(todo.run(json!({ "action": "explode" }), &context).await.is_err());
        todo.run(json!({ "action": "update", "id": 1, "status": "in_progress", "activeForm": "writing the parser" }), &context).await.unwrap();

        for (key, value) in todo.state_to_save() {
            session.set_state(&key, value).unwrap();
        }
        let restored = Todo::new(&session);
        assert_eq!(restored.list(), todo.list());
        assert_eq!(restored.list().next_id, 2);
        let section = &restored.summary_sections()[0];
        assert!(section.starts_with("## Todo list\n"));
        assert!(section.ends_with("[in_progress] #1 Write the parser (writing the parser)"));

        // A cleared list is saved too: the panel must empty.
        restored.run(json!({ "action": "clear" }), &context).await.unwrap();
        assert_eq!(restored.state_to_save()[0].1, json!({ "tasks": [], "nextId": 1 }));
    }

    #[test]
    fn a_damaged_task_is_dropped_not_fatal() {
        let value = json!({ "tasks": [{ "id": 1, "subject": "Fine", "status": "pending" }, { "id": "x" }, null], "nextId": 2 });
        let list = TodoList::from_value(&value).unwrap();
        assert_eq!(list.tasks.len(), 1);
        assert_eq!(list.tasks[0].subject, "Fine");
        assert_eq!(TodoList::from_value(&json!({ "tasks": "corrupt" })), None);
    }
}
