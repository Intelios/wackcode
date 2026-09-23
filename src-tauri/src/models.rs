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

/// Cosmetic, renderer-only preferences (Settings → Appearance). Nothing here reaches a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    /// One-line gist of the reasoning beside a live "Thinking…" row.
    #[serde(default = "default_true")]
    pub thinking_preview: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { thinking_preview: true }
    }
}

/// When the chat's agent should reach for sub-agents. Only changes the tool's guidance text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentTrigger {
    /// Only when the user asks for sub-agents or names one: cost stays predictable.
    #[default]
    OnRequest,
    /// Whenever the agent judges delegation useful.
    Auto,
}

/// A sub-agent's own model. Without one, an agent runs on the chat's model and thinking level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentModel {
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
}

/// One agent the `subagent` tool can launch. Built-in records are refreshed from the shipped
/// definitions on every load (see `subagents.rs`); only `enabled` and `model` are the user's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRecord {
    pub id: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    /// No edit/write, bash limited to inspection, and allowed in Plan mode.
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub model: Option<SubagentModel>,
}

/// The sub-agents built-in extension. Off by default: every child is extra model usage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub trigger: SubagentTrigger,
    /// Children one call runs at the same time; those that can edit files still take turns.
    #[serde(default = "default_subagent_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub agents: Vec<SubagentRecord>,
}

pub const DEFAULT_SUBAGENT_CONCURRENCY: u32 = 4;

fn default_subagent_concurrency() -> u32 {
    DEFAULT_SUBAGENT_CONCURRENCY
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger: SubagentTrigger::OnRequest,
            max_concurrency: DEFAULT_SUBAGENT_CONCURRENCY,
            agents: Vec::new(),
        }
    }
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

/// The agent's working mode. `Plan` is the read-only, plan-first mode and `UltraPlan`
/// (`"ultraplan"`) the same mode with an exhaustive interview; the worker's built-in plan-mode
/// extension enforces both. The record mirrors the worker's `plan_state` events and is the
/// durable hint the UI uses before a worker reports in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    Build,
    Plan,
    UltraPlan,
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
    #[serde(default)]
    pub subagents: SubagentConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
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
            subagents: SubagentConfig::default(),
            appearance: AppearanceConfig::default(),
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
    #[serde(default)]
    pub literal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    pub source_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandSource { App, Extension, Prompt, Skill }

/// A workspace checkpoint: a tree in the chat's shadow repository (see `checkpoints.rs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointRef {
    pub id: String,
    /// The workspace repository's HEAD when the snapshot was taken, if it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// One file restoring a checkpoint would change. `status` is what the restore does to it:
/// "revert", "delete", or "recreate".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointChange {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restored: Vec<String>,
    /// Paths left alone because they hold something the checkpoint store never had (ignored or
    /// oversized files) or because a folder with other content is in the way.
    pub skipped: Vec<String>,
    /// The files just before the restore, itself a checkpoint.
    pub undo: CheckpointRef,
}

/// A file restore requested alongside a conversation change.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSelection {
    pub checkpoint_id: String,
    /// Only these files; all changed files when absent.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreCheckpointInput {
    pub task_id: String,
    pub checkpoint_id: String,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResendInput {
    pub task_id: String,
    /// The user message to send again as a new version.
    pub entry_id: String,
    /// New text for an edit; absent for a retry.
    #[serde(default)]
    pub message: Option<String>,
    /// Indexes of the original message's images to leave out.
    #[serde(default)]
    pub remove_images: Vec<usize>,
    #[serde(default)]
    pub restore: Option<RestoreSelection>,
    #[serde(default)]
    pub started_at: Option<u64>,
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateTaskInput {
    pub task_id: String,
    pub entry_id: String,
    /// "before" (rewind to just above a user message) or "latest" (the newest entry beneath).
    pub target: String,
    /// "rewind", "switch", or "undo".
    pub kind: String,
    #[serde(default)]
    pub restore: Option<RestoreSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    pub leaf_id: Option<String>,
    #[serde(default)]
    pub editor_text: Option<String>,
    /// The files the branch now shown was left with.
    #[serde(default)]
    pub files: Option<CheckpointRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateTaskResult {
    pub navigate: NavigateResult,
    pub restore: Option<RestoreResult>,
    /// Set when the conversation moved but the requested file restore failed.
    pub restore_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkTaskInput {
    pub task_id: String,
    /// The last entry of the turn to fork at; the chat's current end when absent.
    #[serde(default)]
    pub entry_id: Option<String>,
    /// The files as they were after that turn. Without it the fork copies the files as they are now.
    #[serde(default)]
    pub checkpoint: Option<CheckpointRef>,
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
    /// "Write the plan now" on an Ultra Plan questionnaire.
    #[serde(default)]
    pub wrap_up: Option<bool>,
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
