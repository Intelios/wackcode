//! The host side of `@` chat references: resolves a chat id the agent asked for (`read_chat`)
//! to the metadata and session file the worker needs, and enforces who may read whom.
//!
//! The id comes from the model, so this is the guard, not a convenience:
//! - the chat must still exist;
//! - a chat may never read itself (its history is already in its own context);
//! - a Chat mode chat may only read other Chat mode chats, so project material from a Code chat
//!   never reaches a chat whose file tools are confined to a scratchpad;
//! - the target must have a saved session file that is still on disk.
//!
//! Nothing else crosses: no message text, no images, no credentials. The worker renders the
//! session itself (`worker/src/chat-history.ts`), read-only.

use crate::models::{AppData, TaskKind, TaskRecord};
use crate::storage::MetadataState;
use serde_json::{json, Value};
use std::path::Path;
use tauri::{AppHandle, Manager};

/// Answers one `chat_request` from a worker: `{ chat: "<task id>" }`.
pub async fn execute_agent_request(
    app: &AppHandle,
    task_id: &str,
    request: Value,
) -> Result<Value, String> {
    let chat = request
        .get("chat")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|chat| !chat.is_empty())
        .ok_or_else(|| "That chat reference had no chat id.".to_string())?;
    let state = app.state::<MetadataState>();
    let data = state
        .data
        .lock()
        .map_err(|_| "WackCode's chats could not be read.".to_string())?;
    resolve(&data, task_id, chat)
}

/// The rule, as a pure function of the saved state: what the worker may know about `chat_id`,
/// or the sentence the model relays. Split out so the rule is testable without an app handle.
pub fn resolve(data: &AppData, requester_id: &str, chat_id: &str) -> Result<Value, String> {
    let requester = data
        .tasks
        .iter()
        .find(|task| task.id == requester_id)
        .ok_or_else(|| "This chat isn't saved yet, so it cannot read another chat.".to_string())?;
    let target = data
        .tasks
        .iter()
        .find(|task| task.id == chat_id)
        .ok_or_else(|| "That chat no longer exists.".to_string())?;
    if target.id == requester.id {
        return Err("That is this chat; its history is already in your context.".to_string());
    }
    if requester.kind == TaskKind::Chat && target.kind == TaskKind::Code {
        return Err("Chat mode chats can only read other Chat mode chats.".to_string());
    }
    let session_file = target
        .session_file
        .as_deref()
        .filter(|path| !path.is_empty() && Path::new(path).is_file())
        .ok_or_else(|| "That chat has no saved history yet.".to_string())?;
    Ok(json!({
        "id": target.id,
        "name": target.name,
        "kind": target.kind,
        "archived": target.archived,
        "project": project_name(data, target),
        "workspace": target.workspace_path,
        "model": format!("{} · {}", target.provider_id, target.model_id),
        "lastActiveAt": target.last_activity_at,
        "sessionFile": session_file,
    }))
}

/// The chat's project name; a Chat mode chat, or one whose project was removed, has none.
fn project_name<'a>(data: &'a AppData, target: &TaskRecord) -> Option<&'a str> {
    let id = target.project_id.as_deref()?;
    data.projects
        .iter()
        .find(|project| project.id == id)
        .map(|project| project.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use crate::models::{AppData, ProjectRecord, TaskKind, TaskMode, TaskRecord, TaskStatus};
    use std::fs;

    fn task(directory: &std::path::Path, id: &str, kind: TaskKind, session: bool) -> TaskRecord {
        let session_file = session.then(|| {
            let path = directory.join(format!("{id}.jsonl"));
            fs::write(&path, "{}\n").unwrap();
            path.to_string_lossy().into_owned()
        });
        TaskRecord {
            id: id.into(),
            kind,
            project_id: None,
            name: format!("Chat {id}"),
            auto_title_eligible: false,
            auto_title_attempt_id: None,
            workspace_path: directory.to_string_lossy().into_owned(),
            worktree_path: None,
            branch: None,
            uses_worktree: false,
            provider_id: "p".into(),
            model_id: "m".into(),
            thinking_level: "off".into(),
            session_file,
            status: TaskStatus::Idle,
            mode: TaskMode::Build,
            archived: false,
            archived_at: None,
            last_error: None,
            created_at: "now".into(),
            updated_at: "now".into(),
            last_activity_at: None,
        }
    }

    fn data(tasks: Vec<TaskRecord>) -> AppData {
        AppData { tasks, ..AppData::default() }
    }

    #[test]
    fn a_code_chat_may_read_any_other_chat() {
        let directory = tempfile::tempdir().unwrap();
        let data = data(vec![
            task(directory.path(), "code", TaskKind::Code, true),
            task(directory.path(), "other", TaskKind::Code, true),
            task(directory.path(), "chat", TaskKind::Chat, true),
        ]);
        let answer = resolve(&data, "code", "other").unwrap();
        assert_eq!(answer["id"], "other");
        assert_eq!(answer["name"], "Chat other");
        assert!(answer["sessionFile"].as_str().unwrap().ends_with("other.jsonl"));
        assert!(resolve(&data, "code", "chat").is_ok());
    }

    #[test]
    fn a_chat_mode_chat_may_only_read_other_chat_mode_chats() {
        let directory = tempfile::tempdir().unwrap();
        let data = data(vec![
            task(directory.path(), "code", TaskKind::Code, true),
            task(directory.path(), "chat", TaskKind::Chat, true),
            task(directory.path(), "chat2", TaskKind::Chat, true),
        ]);
        assert_eq!(
            resolve(&data, "chat", "code").unwrap_err(),
            "Chat mode chats can only read other Chat mode chats."
        );
        assert!(resolve(&data, "chat", "chat2").is_ok());
    }

    #[test]
    fn self_missing_and_empty_chats_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let data = data(vec![
            task(directory.path(), "a", TaskKind::Code, true),
            task(directory.path(), "empty", TaskKind::Code, false),
        ]);
        assert_eq!(
            resolve(&data, "a", "a").unwrap_err(),
            "That is this chat; its history is already in your context."
        );
        assert_eq!(
            resolve(&data, "a", "gone").unwrap_err(),
            "That chat no longer exists."
        );
        assert_eq!(
            resolve(&data, "a", "empty").unwrap_err(),
            "That chat has no saved history yet."
        );
        // A session file that was removed from disk is the same as never having one.
        let path = directory.path().join("a.jsonl");
        fs::remove_file(path).unwrap();
        assert_eq!(
            resolve(&data, "a", "empty").unwrap_err(),
            "That chat has no saved history yet."
        );
    }

    #[test]
    fn the_project_name_rides_along_when_it_still_exists() {
        let directory = tempfile::tempdir().unwrap();
        let mut chat = task(directory.path(), "a", TaskKind::Code, true);
        chat.project_id = Some("p1".into());
        let project: ProjectRecord = serde_json::from_value(serde_json::json!({
            "id": "p1", "name": "WackCode", "path": directory.path().to_string_lossy(),
            "gitHasHead": false, "createdAt": "now"
        }))
        .unwrap();
        let mut data = data(vec![chat]);
        data.projects = vec![project];
        assert_eq!(resolve(&data, "a", "a").is_err(), true);
        let mut requester = task(directory.path(), "b", TaskKind::Code, true);
        requester.project_id = Some("p1".into());
        data.tasks.push(requester);
        let answer = resolve(&data, "b", "a").unwrap();
        assert_eq!(answer["project"], "WackCode");
    }
}
