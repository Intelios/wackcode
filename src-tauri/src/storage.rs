use crate::{models::{normalize_base_url, AppData, TaskStatus}, secrets::SecretStore};
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
        // Connections saved before `validate_base_url` trimmed a pasted endpoint suffix can still
        // carry `…/v1` or `…/v1/messages`; the Messages client adds `/v1/messages` itself, so the
        // request doubled the path and landed on the gateway's HTML 404 page. Repair them once.
        let rebased = normalize_provider_base_urls(&mut data);
        let changed = recovered || refreshed || rebased;
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

/// Re-canonicalizes every saved connection's base URL; returns whether any changed. The fix is
/// idempotent, so a clean `wackcode.json` writes nothing.
fn normalize_provider_base_urls(data: &mut AppData) -> bool {
    let mut changed = false;
    for provider in &mut data.providers {
        let normalized = normalize_base_url(&provider.base_url, &provider.api_format);
        if normalized != provider.base_url {
            provider.base_url = normalized;
            changed = true;
        }
    }
    changed
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
        let data: AppData = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(data.appearance.collapse_completed_work);
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{}}"#).unwrap();
        assert!(data.appearance.thinking_preview);
        assert!(!data.appearance.message_bubbles);
        assert!(data.appearance.group_exploration);
        assert!(data.appearance.collapse_completed_work);
        // The thinking timer counts whole seconds until the user picks tenths.
        assert_eq!(data.appearance.thinking_timer_precision, crate::models::ThinkingTimerPrecision::Second);
        let data: AppData = serde_json::from_str(r#"{"version":1,"appearance":{"thinkingPreview":false,"messageBubbles":true,"groupExploration":false,"collapseCompletedWork":false,"thinkingTimerPrecision":"tenth"}}"#).unwrap();
        assert!(!data.appearance.thinking_preview);
        assert!(data.appearance.message_bubbles);
        assert!(!data.appearance.group_exploration);
        assert!(!data.appearance.collapse_completed_work);
        assert_eq!(data.appearance.thinking_timer_precision, crate::models::ThinkingTimerPrecision::Tenth);
        let saved = serde_json::to_string(&data).unwrap();
        assert!(saved.contains(r#""thinkingPreview":false"#));
        assert!(saved.contains(r#""messageBubbles":true"#));
        assert!(saved.contains(r#""groupExploration":false"#));
        assert!(saved.contains(r#""collapseCompletedWork":false"#));
        assert!(saved.contains(r#""thinkingTimerPrecision":"tenth""#));
    }

    #[test]
    fn appearance_completed_work_folding_defaults_on_and_persists_without_resetting_preferences() {
        use crate::models::{AppearanceConfig, BackdropMode, GlassStyleSetting, ThinkingTimerPrecision};
        let mut expected = AppearanceConfig {
            thinking_preview: false,
            thinking_timer_precision: ThinkingTimerPrecision::Tenth,
            message_bubbles: true,
            group_exploration: false,
            accent: Some("#b69cff".into()),
            background: Some("#14111b".into()),
            backdrop: BackdropMode::Image,
            background_image: Some("kept.png".into()),
            image_dim: 50,
            image_blur: 7,
            image_zoom: 160,
            image_x: 250,
            image_y: 750,
            glass_style: GlassStyleSetting::Clear,
            glass_tint: 25,
            agent_name: Some("Nova".into()),
            ..AppearanceConfig::default()
        };
        assert!(expected.collapse_completed_work);
        // An older file can have every other preference set, but no completed-work setting.
        let mut legacy = serde_json::to_value(AppData { appearance: expected.clone(), ..AppData::default() }).unwrap();
        legacy["appearance"].as_object_mut().unwrap().remove("collapseCompletedWork");
        let data: AppData = serde_json::from_value(legacy).unwrap();
        assert_eq!(data.appearance, expected);

        let directory = tempfile::tempdir().unwrap();
        let state = MetadataState {
            data: Mutex::new(data),
            data_path: directory.path().join("wackcode.json"),
            secrets: SecretStore::load(directory.path()).unwrap(),
        };
        for collapse_completed_work in [false, true] {
            state.mutate(|data| {
                data.appearance.collapse_completed_work = collapse_completed_work;
                Ok(())
            }).unwrap();
            expected.collapse_completed_work = collapse_completed_work;
            let content = fs::read_to_string(&state.data_path).unwrap();
            let reloaded: AppData = serde_json::from_str(&content).unwrap();
            assert_eq!(reloaded.appearance, expected);
            assert_eq!(state.data.lock().unwrap().appearance, expected);
        }
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
                id: "task".into(), kind: crate::models::TaskKind::Code, project_id: Some("project".into()), name: "Running task".into(),
                auto_title_eligible: false, auto_title_attempt_id: None,
                workspace_path: "/tmp/project".into(), worktree_path: None, branch: None,
                uses_worktree: false, provider_id: "provider".into(), model_id: "model".into(),
                thinking_level: "off".into(), session_file: Some("session.jsonl".into()),
                status: TaskStatus::Running, mode: crate::models::TaskMode::Build, archived: false, archived_at: None, last_error: None,
                created_at: "now".into(), updated_at: "now".into(), last_activity_at: None,
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
        // A chat saved before Chat mode existed is a coding chat with no recorded activity.
        assert_eq!(task.kind, crate::models::TaskKind::Code);
        assert!(task.last_activity_at.is_none());
        let chat = json.replace("\"id\":\"t\"", "\"id\":\"t\",\"kind\":\"chat\"");
        let task: TaskRecord = serde_json::from_str(&chat).unwrap();
        assert_eq!(task.kind, crate::models::TaskKind::Chat);
        assert_eq!(serde_json::to_value(&task).unwrap()["kind"], "chat");
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

    #[test]
    fn startup_repairs_a_messages_connection_saved_with_an_endpoint_suffix() {
        // A connection saved before `validate_base_url` trimmed keeps its pasted `…/v1` (or a
        // whole `…/v1/messages` endpoint); loading re-canonicalizes it so the Messages client
        // stops doubling the path into the gateway's HTML 404 page.
        let connection = |base_url: &str, api_format: &str| crate::models::ProviderRecord {
            id: "p".into(), name: "P".into(), kind: crate::models::ProviderKind::Custom,
            base_url: base_url.into(), api_format: api_format.into(), models: vec![],
            created_at: "now".into(), updated_at: "now".into(),
            has_api_key: true, connected: true, enabled: true,
        };
        let mut data = AppData {
            providers: vec![
                connection("https://opencode.ai/zen/go/v1", "anthropic-messages"),
                connection("https://opencode.ai/zen/go/v1/messages", "anthropic-messages"),
                connection("https://opencode.ai/zen/go/v1", "openai-completions"),
            ],
            ..AppData::default()
        };
        assert!(normalize_provider_base_urls(&mut data));
        assert_eq!(data.providers[0].base_url, "https://opencode.ai/zen/go");
        assert_eq!(data.providers[1].base_url, "https://opencode.ai/zen/go");
        // The OpenAI formats keep their `/v1` — and a clean load reports nothing changed.
        assert_eq!(data.providers[2].base_url, "https://opencode.ai/zen/go/v1");
        assert!(!normalize_provider_base_urls(&mut data));
    }
}
