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
    /// Accepts image input. Maps to Pi's `input: ["text", "image"]`; off means Pi swaps any image
    /// for its own "image omitted" placeholder before the request leaves the worker.
    #[serde(default)]
    pub vision: bool,
}

/// Local Pi catalogue metadata offered in Settings; never persisted with a connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinModelSuggestion {
    pub source_provider: String,
    pub source_api: String,
    pub id: String,
    pub name: String,
    pub context_window: u64,
    pub max_tokens: u64,
    pub reasoning: bool,
    pub thinking_levels: Vec<String>,
    pub thinking_level_map: BTreeMap<String, Option<String>>,
    pub vision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_format: String,
    #[serde(default)]
    pub models: Vec<ModelRecord>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    #[default]
    Custom,
    Subscription,
}

/// One resource file a package contributes, with whether the user's filters currently load it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageResourceRecord {
    /// Absolute path on disk. This is what a task worker is handed.
    pub path: String,
    /// Path relative to the package root, which is what the user sees.
    pub name: String,
    pub enabled: bool,
}

/// An installed Pi package. `trusted_at` records that the user accepted the install warning;
/// a package without it is never loaded into a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageRecord {
    pub source: String,
    pub display_name: String,
    pub kind: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub installed_path: Option<String>,
    #[serde(default)]
    pub extensions: Vec<PackageResourceRecord>,
    #[serde(default)]
    pub skills: Vec<PackageResourceRecord>,
    #[serde(default)]
    pub prompts: Vec<PackageResourceRecord>,
    #[serde(default)]
    pub themes: Vec<PackageResourceRecord>,
    #[serde(default)]
    pub errors: Vec<String>,
    pub trusted_at: String,
    pub installed_at: String,
}

impl PackageRecord {
    /// Enabled resource paths of one kind. Only these are handed to a task worker.
    pub fn enabled_paths(&self, kind: &str) -> Vec<String> {
        let resources = match kind {
            "extensions" => &self.extensions,
            "skills" => &self.skills,
            "prompts" => &self.prompts,
            "themes" => &self.themes,
            _ => return Vec::new(),
        };
        resources.iter().filter(|resource| resource.enabled).map(|resource| resource.path.clone()).collect()
    }
}

/// Where a tool came from, mirrored from the worker's snapshot so Settings can group them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolSource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One tool the agent can be offered. Cached from the last worker snapshot so the Tools
/// panel has something to render before any chat is open.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: ToolSource,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Tools the user has switched off. A denylist, so a tool contributed by a newly
/// installed package is on by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    #[serde(default)]
    pub disabled: Vec<String>,
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

/// The agent's working mode. `Plan` is the read-only, plan-first mode; the worker's built-in
/// plan-mode extension enforces it. The record mirrors the worker's `plan_state` events and is
/// the durable hint the UI uses before a worker reports in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    Build,
    Plan,
}

impl Default for TaskMode {
    fn default() -> Self {
        Self::Build
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
    pub mode: TaskMode,
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
    #[serde(default)]
    pub tool_config: ToolConfig,
    #[serde(default)]
    pub tool_catalog: Vec<ToolCatalogEntry>,
    #[serde(default)]
    pub packages: Vec<PackageRecord>,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            version: DATA_VERSION,
            providers: Vec::new(),
            projects: Vec::new(),
            tasks: Vec::new(),
            tool_config: ToolConfig::default(),
            tool_catalog: Vec::new(),
            packages: Vec::new(),
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
    #[serde(default)]
    pub started_at: Option<u64>,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
    /// The mode the composer was in when this prompt was sent; applied before the run.
    #[serde(default)]
    pub mode: Option<TaskMode>,
    /// Images attached to this prompt, in Pi's own `ImageContent` shape. Forwarded verbatim.
    #[serde(default)]
    pub images: Vec<ImageContent>,
}

/// One image attached to a prompt. Mirrors Pi's `ImageContent` (`{ type: "image", data, mimeType }`),
/// the shape Pi's RPC `prompt.images` takes, so it travels unchanged from React to the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub kind: String,
    /// Base64 without a `data:` prefix.
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTaskModeInput {
    pub task_id: String,
    pub mode: TaskMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlanInput {
    pub task_id: String,
    /// The full plan text; the worker owns it, the desktop passes it back verbatim.
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPackagesInput {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub from: Option<u32>,
}

/// One row in the Browse tab. Everything here comes from the public npm registry.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSearchResult {
    pub name: String,
    pub version: String,
    pub description: String,
    pub publisher: String,
    pub npm_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub published_at: String,
    /// Which resource kinds the package declares, so the user knows what installing adds.
    pub declares: Vec<String>,
}

/// One answered question from a `questions` dialog. Mirrors `QuestionAnswer` in the worker
/// protocol; Rust only forwards it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    pub custom: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUiResponseInput {
    pub task_id: String,
    pub request_id: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub confirmed: Option<bool>,
    #[serde(default)]
    pub cancelled: Option<bool>,
    #[serde(default)]
    pub answers: Option<Vec<QuestionAnswer>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPackageInput {
    pub source: String,
    /// The user accepted the install warning. Without it the command refuses.
    #[serde(default)]
    pub trusted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPackageResourcesInput {
    pub source: String,
    /// Omit a kind to load all of it; an empty list loads none.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub prompts: Option<Vec<String>>,
    #[serde(default)]
    pub themes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetToolConfigInput {
    #[serde(default)]
    pub disabled: Vec<String>,
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
