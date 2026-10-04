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
        let recovered = recover_interrupted_tasks(&mut data);
        // Built-in sub-agent definitions ship with the app; refresh them on every start.
        let refreshed = crate::subagents::normalize(&mut data.subagents);
        let changed = recovered || refreshed;
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
            // Access permissions must remain at their saved value on failure. Other state,
            // such as a worker crash interruption, intentionally survives a failed save.
            let previous_policy = data.execution_policy.clone();
            let result = operation(&mut data)
                .and_then(|result| save_to_path(&self.data_path, &data).map(|_| result));
            if result.is_err() { data.execution_policy = previous_policy; }
            result?
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
    use crate::models::{FavoriteModelRef, TaskRecord};

    #[test]
    fn metadata_round_trip_has_no_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wackcode.json");
        let mut data = AppData::default();
        // Sub-agent settings name the connection an agent uses, never its credential.
        data.subagents.enabled = true;
        crate::subagents::normalize(&mut data.subagents);
        data.subagents.agents[0].model = Some(crate::models::SubagentModel {
            provider_id: "custom-1".into(), model_id: "small".into(), thinking_level: "off".into(),
        });
        save_to_path(&path, &data).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("providers"));
        assert!(content.contains("subagents"));
        assert!(!content.contains("apiKey"));
        assert!(!content.contains("authPath"));
    }

    #[test]
    fn execution_policy_persists_and_failed_saves_leave_effective_settings_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let state = MetadataState {
            data: Mutex::new(AppData::default()),
            data_path: directory.path().join("wackcode.json"),
            secrets: SecretStore::load(directory.path()).unwrap(),
        };
        let enabled = crate::models::ExecutionPolicyConfig {
            unrestricted_planning: true, unrestricted_subagents: true,
        };
        state.mutate(|data| { data.execution_policy = enabled.clone(); Ok(()) }).unwrap();
        let reloaded: AppData = serde_json::from_str(&fs::read_to_string(&state.data_path).unwrap()).unwrap();
        assert_eq!(reloaded.execution_policy, enabled);

        // A directory in place of the temporary file makes the next write fail deterministically.
        fs::create_dir(state.data_path.with_extension("json.tmp")).unwrap();
        assert!(state.mutate(|data| {
            data.execution_policy = crate::models::ExecutionPolicyConfig::default(); Ok(())
        }).is_err());
        assert_eq!(state.data.lock().unwrap().execution_policy, enabled);
        let disk: AppData = serde_json::from_str(&fs::read_to_string(&state.data_path).unwrap()).unwrap();
        assert_eq!(disk.execution_policy, enabled);

        fs::remove_dir(state.data_path.with_extension("json.tmp")).unwrap();
        state.mutate(|data| { data.execution_policy = crate::models::ExecutionPolicyConfig::default(); Ok(()) }).unwrap();
        fs::create_dir(state.data_path.with_extension("json.tmp")).unwrap();
        assert!(state.mutate(|data| { data.execution_policy = enabled.clone(); Ok(()) }).is_err());
        assert_eq!(state.data.lock().unwrap().execution_policy, crate::models::ExecutionPolicyConfig::default());
    }

    #[test]
    fn metadata_written_before_tool_settings_existed_still_loads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wackcode.json");
        fs::write(&path, r#"{"version":1,"providers":[],"projects":[],"tasks":[]}"#).unwrap();
        let data: AppData = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(data.tool_config.disabled.is_empty());
        assert!(data.tool_catalog.is_empty());
        assert!(data.favorite_models.is_empty());
        assert!(data.appearance.thinking_preview);
    }

    #[test]
    fn favorite_model_references_survive_relaunch_even_without_available_providers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wackcode.json");
        let data = AppData {
            favorite_models: vec![
                FavoriteModelRef { provider_id: "p".into(), model_id: "same".into() },
                FavoriteModelRef { provider_id: "q".into(), model_id: "same".into() },
            ],
            ..AppData::default()
        };
        save_to_path(&path, &data).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("favoriteModels"));
        let reloaded: AppData = serde_json::from_str(&content).unwrap();
        assert_eq!(reloaded.favorite_models, data.favorite_models);
    }

    #[test]
    fn appearance_settings_default_on_and_round_trip() {
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{}}"#).unwrap();
        assert!(data.appearance.thinking_preview);
        assert!(!data.appearance.message_bubbles);
        assert!(data.appearance.group_exploration);
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{"thinkingPreview":false,"messageBubbles":true,"groupExploration":false}}"#).unwrap();
        assert!(!data.appearance.thinking_preview);
        assert!(data.appearance.message_bubbles);
        assert!(!data.appearance.group_exploration);
        let saved = serde_json::to_string(&data).unwrap();
        assert!(saved.contains(r#""thinkingPreview":false"#));
        assert!(saved.contains(r#""messageBubbles":true"#));
        assert!(saved.contains(r#""groupExploration":false"#));
    }

    #[test]
    fn appearance_files_from_before_theming_load_with_the_default_look() {
        use crate::models::{AppearanceConfig, BackdropMode};
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{"thinkingPreview":false}}"#).unwrap();
        assert_eq!(data.appearance, AppearanceConfig { thinking_preview: false, ..AppearanceConfig::default() });
        assert_eq!(data.appearance.backdrop, BackdropMode::Solid);
        assert_eq!((data.appearance.image_dim, data.appearance.image_blur, data.appearance.glass_tint), (65, 12, 40));
        // Files from before image cropping show the picture centred and uncropped.
        assert_eq!((data.appearance.image_zoom, data.appearance.image_x, data.appearance.image_y), (100, 500, 500));
        // Unpicked colours stay absent, so a future default still reaches this user.
        let saved = serde_json::to_string(&data.appearance).unwrap();
        assert!(!saved.contains("accent") && !saved.contains("backgroundImage"), "{saved}");
    }

    #[test]
    fn agent_name_is_absent_until_the_user_picks_one() {
        // Files written before the setting existed load with the default persona.
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{"agentName":null}}"#).unwrap();
        assert_eq!(data.appearance.agent_name, None);
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{}}"#).unwrap();
        assert_eq!(data.appearance.agent_name, None);
        assert!(!serde_json::to_string(&data.appearance).unwrap().contains("agentName"));
        // A picked name round-trips.
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{"agentName":"Nova"}}"#).unwrap();
        assert_eq!(data.appearance.agent_name.as_deref(), Some("Nova"));
        assert!(serde_json::to_string(&data.appearance).unwrap().contains(r#""agentName":"Nova""#));
    }

    #[test]
    fn window_geometry_is_absent_until_recorded() {
        let data: AppData = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(data.window.is_none());
        assert!(!serde_json::to_string(&data).unwrap().contains("\"window\""));
        let data: AppData = serde_json::from_str(
            r#"{"version":1,"window":{"width":1200.0,"height":800.0,"x":40,"y":60}}"#,
        )
        .unwrap();
        let window = data.window.unwrap();
        assert_eq!((window.width, window.height), (1200.0, 800.0));
        assert_eq!((window.x, window.y), (Some(40), Some(60)));
    }

    #[test]
    fn prompt_overrides_default_empty_and_round_trip() {
        // Old files predate the field entirely; cleared fields never appear in the file.
        let data: AppData = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(data.prompts, crate::models::PromptConfig::default());
        let data: AppData = serde_json::from_str(
            r#"{"version":1,"prompts":{"planPrompt":"My plan rules."}}"#,
        ).unwrap();
        assert_eq!(data.prompts.plan_prompt.as_deref(), Some("My plan rules."));
        assert_eq!(data.prompts.system_prompt, None);
        let written = serde_json::to_string(&data).unwrap();
        assert!(written.contains(r#""planPrompt":"My plan rules.""#));
        assert!(!written.contains("systemPrompt"));
    }

    #[test]
    fn startup_marks_active_tasks_interrupted_without_replaying_them() {
        let mut data = AppData {
            tasks: vec![TaskRecord {
                id: "task".into(), project_id: Some("project".into()), name: "Running task".into(),
                auto_title_eligible: false, auto_title_attempt_id: None,
                workspace_path: "/tmp/project".into(), worktree_path: None, branch: None,
                uses_worktree: false, provider_id: "provider".into(), model_id: "model".into(),
                thinking_level: "off".into(), session_file: Some("session.jsonl".into()),
                status: TaskStatus::Running, mode: crate::models::TaskMode::Build, archived: false, archived_at: None, last_error: None,
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
        assert!(!task.auto_title_eligible);
        assert!(task.auto_title_attempt_id.is_none());
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
