use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    #[serde(default)]
    pub thinking_level_map: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_format: String,
    #[serde(default)]
    pub models: Vec<ModelRecord>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub path: String,
    pub git_root: Option<String>,
    pub git_has_head: bool,
    #[serde(default, skip_deserializing)]
    pub branch: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Running,
    Stopping,
    Interrupted,
    Error,
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub project_id: Option<String>,
    pub name: String,
    pub workspace_path: String,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub uses_worktree: bool,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
    pub session_file: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub archived: bool,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppData {
    pub version: u32,
    #[serde(default)]
    pub providers: Vec<ProviderRecord>,
    #[serde(default)]
    pub projects: Vec<ProjectRecord>,
    #[serde(default)]
    pub tasks: Vec<TaskRecord>,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            version: DATA_VERSION,
            providers: Vec::new(),
            projects: Vec::new(),
            tasks: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub api_format: String,
    pub models: Vec<ModelRecord>,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub use_worktree: bool,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInput {
    pub task_id: String,
    pub message: String,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapPayload {
    pub data: AppData,
    pub app_data_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChangeFile {
    pub path: String,
    pub status: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub binary: bool,
    pub truncated: bool,
    pub diff: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChanges {
    pub is_git: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    pub files: Vec<GitChangeFile>,
}
