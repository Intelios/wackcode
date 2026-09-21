use crate::{models::{AppData, TaskStatus}, secrets::SecretStore};
use std::{fs, path::{Path, PathBuf}, sync::Mutex};
use tauri::{AppHandle, Manager};

pub struct MetadataState {
    pub data: Mutex<AppData>,
    pub data_path: PathBuf,
    pub secrets: SecretStore,
}

impl MetadataState {
    pub fn load(app: &AppHandle) -> Result<Self, String> {
        let directory = app.path().app_data_dir().map_err(|error| error.to_string())?;
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let data_path = directory.join("wackcode.json");
        let mut data = if data_path.exists() {
            let content = fs::read_to_string(&data_path).map_err(|error| error.to_string())?;
            serde_json::from_str::<AppData>(&content)
                .map_err(|error| format!("Could not read WackCode metadata: {error}"))?
        } else {
            AppData::default()
        };
        let changed = recover_interrupted_tasks(&mut data);
        let state = Self { data: Mutex::new(data), data_path, secrets: SecretStore::load(&directory)? };
        if changed { state.save()?; }
        Ok(state)
    }

    pub fn save(&self) -> Result<(), String> {
        let data = self.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
        save_to_path(&self.data_path, &data)
    }

    pub fn mutate<T>(&self, operation: impl FnOnce(&mut AppData) -> Result<T, String>) -> Result<T, String> {
        let result = {
            let mut data = self.data.lock().map_err(|_| "Metadata lock was poisoned".to_string())?;
            let result = operation(&mut data)?;
            save_to_path(&self.data_path, &data)?;
            result
        };
        Ok(result)
    }
}

fn recover_interrupted_tasks(data: &mut AppData) -> bool {
    let mut changed = false;
    for task in &mut data.tasks {
        if matches!(task.status, TaskStatus::Running | TaskStatus::Stopping) {
            task.status = TaskStatus::Interrupted;
            task.last_error = Some("WackCode closed while this task was running.".into());
            changed = true;
        }
    }
    changed
}

fn save_to_path(path: &Path, data: &AppData) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(data).map_err(|error| error.to_string())?;
    fs::write(&temporary, json).map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TaskRecord;

    #[test]
    fn metadata_round_trip_has_no_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wackcode.json");
        let data = AppData::default();
        save_to_path(&path, &data).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("providers"));
        assert!(!content.contains("apiKey"));
    }

    #[test]
    fn startup_marks_active_tasks_interrupted_without_replaying_them() {
        let mut data = AppData {
            tasks: vec![TaskRecord {
                id: "task".into(), project_id: Some("project".into()), name: "Running task".into(),
                workspace_path: "/tmp/project".into(), worktree_path: None, branch: None,
                uses_worktree: false, provider_id: "provider".into(), model_id: "model".into(),
                thinking_level: "off".into(), session_file: Some("session.jsonl".into()),
                status: TaskStatus::Running, archived: false, last_error: None,
                created_at: "now".into(), updated_at: "now".into(),
            }],
            ..AppData::default()
        };
        assert!(recover_interrupted_tasks(&mut data));
        assert_eq!(data.tasks[0].status, TaskStatus::Interrupted);
        assert_eq!(data.tasks[0].session_file.as_deref(), Some("session.jsonl"));
        assert!(data.tasks[0].last_error.as_deref().unwrap().contains("closed"));
        assert!(!recover_interrupted_tasks(&mut data));
    }

    #[test]
    fn task_project_id_is_optional() {
        let json = r#"{"id":"t","projectId":null,"name":"n","workspacePath":"/tmp","worktreePath":null,"branch":null,"usesWorktree":false,"providerId":"p","modelId":"m","thinkingLevel":"off","sessionFile":null,"status":"idle","archived":false,"lastError":null,"createdAt":"c","updatedAt":"u"}"#;
        let task: TaskRecord = serde_json::from_str(json).unwrap();
        assert_eq!(task.project_id, None);
        let legacy = json.replace("\"projectId\":null", "\"projectId\":\"project-1\"");
        let task: TaskRecord = serde_json::from_str(&legacy).unwrap();
        assert_eq!(task.project_id.as_deref(), Some("project-1"));
    }

    #[test]
    fn project_branch_is_runtime_only() {
        use crate::models::ProjectRecord;
        let json = r#"{"id":"p","name":"n","path":"/tmp/p","gitRoot":"/tmp/p","gitHasHead":true,"createdAt":"c"}"#;
        let project: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(project.branch, None);
        let with_branch = json.replace("\"createdAt\":\"c\"", "\"branch\":\"main\",\"createdAt\":\"c\"");
        let project: ProjectRecord = serde_json::from_str(&with_branch).unwrap();
        assert_eq!(project.branch, None);
    }
}
