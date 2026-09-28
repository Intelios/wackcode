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
    /// The MCP server's id when `kind` is "mcp".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
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

/// How WackCode reaches an MCP server.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
    Sse,
}

/// One tool an MCP server offers, as its last "Test connection" found it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The server marks it read-only (`readOnlyHint`), so Plan mode lets it through.
    #[serde(default)]
    pub read_only: bool,
}

/// One MCP server from Settings › MCP servers. Header and environment variable values are not
/// here: only their names. The values (often tokens) live in `secrets.json` (`mcp::secret_key`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRecord {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub transport: McpTransport,
    #[serde(default = "default_mcp_timeout")]
    pub timeout_ms: u64,
    /// stdio: the program and its arguments.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// http and sse: the endpoint.
    #[serde(default)]
    pub url: String,
    /// Names of the headers sent to an http or sse server.
    #[serde(default)]
    pub headers: Vec<String>,
    /// Names of the environment variables a stdio server gets.
    #[serde(default)]
    pub env: Vec<String>,
    /// The server's own tool names the user switched off.
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    /// The server's tools as of the last successful "Test connection".
    #[serde(default)]
    pub tools: Vec<McpToolInfo>,
}

fn default_mcp_timeout() -> u64 {
    crate::mcp::DEFAULT_TIMEOUT_MS
}

/// Settings › Skills. The user's own skills are files in `~/.agents/skills`; this records only
/// what the user switched: which other folders load, and which skills are off.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillsConfig {
    /// Other tools' folders the user switched on, and folders the user added.
    #[serde(default)]
    pub folders: Vec<SkillFolderRecord>,
    /// Skill files switched off, spelled exactly as a scan reports them.
    #[serde(default)]
    pub disabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFolderRecord {
    /// A known folder's id (`claude`, `codex`, `pi`, `opencode`) or `custom:<uuid>`.
    pub id: String,
    /// Added folders only: the absolute path the user picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillFolderKind {
    /// `~/.agents/skills`: always loads, and the only folder WackCode writes to.
    Library,
    /// Another tool's user-level folder, off until switched on.
    Tool,
    /// A folder the user added.
    Custom,
}

/// Settings' list of every skill folder and every trusted package's skills. Runtime only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsOverview {
    pub library_path: String,
    pub folders: Vec<SkillFolderView>,
    pub packages: Vec<SkillPackageView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFolderView {
    pub id: String,
    pub label: String,
    pub path: String,
    /// `path` with the home folder written as `~`.
    pub display_path: String,
    pub kind: SkillFolderKind,
    pub exists: bool,
    pub enabled: bool,
    pub skills: Vec<SkillEntry>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageView {
    pub source: String,
    pub label: String,
    pub skills: Vec<SkillEntry>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    /// `disable-model-invocation: true`: only `/skill:name` uses it.
    pub manual: bool,
    /// The SKILL.md's optional `argument-hint`, shown next to the name in Settings and the picker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// The skill's own switch (for a package skill, its resource's).
    pub enabled: bool,
    /// Inside `~/.agents/skills`, so WackCode may edit and delete it.
    pub editable: bool,
    /// Switched on, but a skill of the same name wins: that skill's folder or package.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<String>,
    /// Package skills: the resource the skill's switch toggles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDiagnostic {
    /// `warning` or `error`, as Pi reports it.
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What the skill editor needs beyond the list: the instructions and the folder's other files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDocument {
    pub body: String,
    /// Paths relative to the skill's folder, `SKILL.md` itself left out.
    pub files: Vec<String>,
    pub files_truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSkillInput {
    /// The skill's file when editing; absent to create one in `~/.agents/skills`.
    #[serde(default)]
    pub path: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub manual: bool,
    /// Optional `argument-hint` frontmatter; empty writes no key.
    #[serde(default)]
    pub argument_hint: String,
    #[serde(default)]
    pub body: String,
}

/// The result of a change in Settings › Skills: the fresh list, and a note when part of the
/// change was skipped (an import that found a name already taken, say).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsChange {
    pub overview: SkillsOverview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSkillPackagesInput {
    #[serde(default)]
    pub query: Option<String>,
    /// `downloads`, `recent` or `name`.
    #[serde(default)]
    pub sort: Option<String>,
    /// 1-based.
    #[serde(default)]
    pub page: Option<u32>,
}

/// One page of Settings › Skills › Browse.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSearchPage {
    pub results: Vec<PackageSearchResult>,
    /// `pidev`, or `npm` when pi.dev could not be read and keyword search stood in.
    pub source: String,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerRecord>,
}

/// A header or environment variable as the editor sends it. A blank value keeps the saved one.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSecretInput {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMcpServerInput {
    pub id: Option<String>,
    pub name: String,
    pub transport: McpTransport,
    pub timeout_ms: u64,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<McpSecretInput>,
    #[serde(default)]
    pub env: Vec<McpSecretInput>,
}

/// What "Test connection" found, with the server as saved afterwards.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub server: McpServerRecord,
}

/// Tools the user has switched off. A denylist, so a tool contributed by a newly
/// installed package is on by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Cosmetic preferences (Settings → Appearance). Nothing here reaches a worker: the renderer
/// themes itself (`src/theme.ts`) and `glass.rs` applies the backdrop to the native window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    /// One-line gist of the reasoning beside a live "Thinking…" row.
    #[serde(default = "default_true")]
    pub thinking_preview: bool,
    /// Assistant prose in a bubble like the user's; the user's bubble always shows.
    #[serde(default)]
    pub message_bubbles: bool,
    /// Fold runs of read-only tool calls into one "Explored" row in the transcript.
    #[serde(default = "default_true")]
    pub group_exploration: bool,
    /// `#rrggbb`; `None` is WackCode green, so a future default reaches users who never picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// `#rrggbb`; `None` is the default dark background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default)]
    pub backdrop: BackdropMode,
    /// A file name inside `<app data>/backgrounds/`, set only by `choose_background_image`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// How much the background colour covers the image behind a chat, 0–90 %.
    #[serde(default = "default_image_dim")]
    pub image_dim: u8,
    /// Blur of the image behind a chat, 0–40 px.
    #[serde(default = "default_image_blur")]
    pub image_blur: u8,
    #[serde(default)]
    pub glass_style: GlassStyleSetting,
    /// How much the background colour tints the glass, 0–90 %.
    #[serde(default = "default_glass_tint")]
    pub glass_tint: u8,
    /// What the app calls the agent in its own copy ("Nova is working…"); `None` is WackCode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
}

/// What sits behind the app's panels. The modes are exclusive: glass shows the desktop, which
/// an image would cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackdropMode {
    #[default]
    Solid,
    Image,
    Glass,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassStyleSetting {
    #[default]
    Frosted,
    Clear,
}

fn default_image_dim() -> u8 {
    65
}

fn default_image_blur() -> u8 {
    12
}

fn default_glass_tint() -> u8 {
    40
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            thinking_preview: true,
            message_bubbles: false,
            group_exploration: true,
            accent: None,
            background: None,
            backdrop: BackdropMode::Solid,
            background_image: None,
            image_dim: default_image_dim(),
            image_blur: default_image_blur(),
            glass_style: GlassStyleSetting::Frosted,
            glass_tint: default_glass_tint(),
            agent_name: None,
        }
    }
}

/// User-customized built-in prompt texts (Settings → Prompts). An absent field means the
/// shipped default is in force. Plain text in `wackcode.json`: none of it is a credential.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptConfig {
    /// Replaces Pi's default system-prompt persona; the assembled sections still follow it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Replaces the Plan-mode contract body (the marker line stays the app's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_prompt: Option<String>,
    /// Replaces the Ultra Plan contract body (the marker line stays the app's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ultra_plan_prompt: Option<String>,
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

/// One child of one `subagent` call, as the side panel watches it: the call's tool call id and
/// the child's position in it. Mirrors `SubagentTarget` in the worker's protocol.ts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentWatchTarget {
    pub tool_call_id: String,
    pub index: u32,
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
    /// Children one call runs at the same time, including those that can edit files.
    #[serde(default = "default_subagent_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub agents: Vec<SubagentRecord>,
}

pub const DEFAULT_SUBAGENT_CONCURRENCY: u32 = 4;

/// The computer-use built-in (`computer_use/`). Off by default. Which apps a chat may use is
/// never stored: grants live in memory per chat until the app quits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Bundle ids the user never wants the agent to use, on top of the built-in block list.
    #[serde(default)]
    pub never_allow: Vec<String>,
}

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

/// One extra model request on a new chat's opening prompt. Never enabled by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoTitleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
}

/// Project memory (Settings › Memory, see `memory.rs`). On by default; each project can be
/// switched off on its own. The notes themselves are files under `<app data>/memory/`, never
/// records here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConfig {
    #[serde(default = "memory_enabled_default")]
    pub enabled: bool,
    /// Memory keys (the hash suffix of each `<app data>/memory/<name>-<key>` directory) the
    /// user switched off, on top of the global switch.
    #[serde(default)]
    pub disabled_projects: Vec<String>,
}

fn memory_enabled_default() -> bool { true }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self { enabled: true, disabled_projects: Vec::new() }
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
    /// Legacy chats and forks deserialize as ineligible. Consumed before the first prompt.
    #[serde(default)]
    pub auto_title_eligible: bool,
    #[serde(default)]
    pub auto_title_attempt_id: Option<String>,
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
    /// When the chat was archived; the Archived view orders by it. Cleared on unarchive,
    /// and absent for chats archived before it existed (the UI falls back to `updated_at`).
    #[serde(default)]
    pub archived_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The main window's last normal-mode geometry in logical points, recorded from resize and
/// move events and restored on launch (`window_state.rs`). Absent until the first event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub width: f64,
    pub height: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
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
    pub diff_comments: std::collections::HashMap<String, Vec<DiffComment>>,
    #[serde(default)]
    pub tool_config: ToolConfig,
    #[serde(default)]
    pub tool_catalog: Vec<ToolCatalogEntry>,
    #[serde(default)]
    pub packages: Vec<PackageRecord>,
    #[serde(default)]
    pub subagents: SubagentConfig,
    #[serde(default)]
    pub auto_title: AutoTitleConfig,
    #[serde(default = "record_usage_default")]
    pub record_usage: bool,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub prompts: PromptConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub commands: CommandsConfig,
    #[serde(default)]
    pub computer_use: ComputerUseConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowState>,
}

fn record_usage_default() -> bool { true }

impl Default for AppData {
    fn default() -> Self {
        Self {
            version: DATA_VERSION,
            providers: Vec::new(),
            projects: Vec::new(),
            tasks: Vec::new(),
            diff_comments: std::collections::HashMap::new(),
            tool_config: ToolConfig::default(),
            tool_catalog: Vec::new(),
            packages: Vec::new(),
            subagents: SubagentConfig::default(),
            auto_title: AutoTitleConfig::default(),
            record_usage: true,
            appearance: AppearanceConfig::default(),
            prompts: PromptConfig::default(),
            mcp: McpConfig::default(),
            skills: SkillsConfig::default(),
            commands: CommandsConfig::default(),
            computer_use: ComputerUseConfig::default(),
            memory: MemoryConfig::default(),
            window: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffComment {
    pub id: String,
    pub path: String,
    pub layer: String,
    pub side: String,
    pub line: usize,
    pub excerpt: String,
    pub revision: String,
    pub text: String,
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

/// Queue a message on a chat's running prompt: "steer" delivers it at the run's next boundary,
/// "follow_up" holds it until the run finishes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueMessageInput {
    pub task_id: String,
    pub behavior: String,
    pub message: String,
    /// Images attached to this message, forwarded verbatim to the worker.
    #[serde(default)]
    pub images: Vec<ImageContent>,
    /// Sent raw, without command, skill or template expansion ("Send as message").
    #[serde(default)]
    pub literal: bool,
}

/// The texts Pi's pending-message queues held when they were cleared, for the composer to take back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedMessages {
    pub steering: Vec<String>,
    #[serde(default)]
    pub follow_up: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// Its `commandKey` — stable across sessions and the same key Settings' switches use.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    pub source_label: String,
    /// What the model can call: a prompt template's `argument-hint` or a skill's, shown next to the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandSource { App, Extension, Prompt, Custom, Skill }

/// Settings › Commands. Which `commandKey`s are switched off; the user's commands themselves
/// are files in `<app data>/commands` (see `slash_commands.rs`), not records here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandsConfig {
    /// `app:` / `extension:` / `prompt:` / `custom:` / `skill:` keys the user switched off.
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Where one listed command comes from, for Settings' grouping. `App` rows are the desktop's own.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandKind { App, Extension, Prompt, Custom }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandEntry {
    /// Its `commandKey` — the same id a chat's `SlashCommand.id` reports, so they always agree.
    pub key: String,
    /// What `/` offers: the clash-resolved name, or the command's own name while switched off.
    pub name: String,
    /// The name before a clash renamed it to `<source>:<name>`, when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub kind: SlashCommandKind,
    pub enabled: bool,
    /// The user's own command file may be edited and deleted from Settings.
    pub editable: bool,
    /// The template file for prompt/custom, the extension file for extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandGroupKind { Custom, Package }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandGroup {
    /// `custom` for the user's own commands, else the package's `source`.
    pub id: String,
    pub label: String,
    pub kind: SlashCommandGroupKind,
    pub entries: Vec<SlashCommandEntry>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

/// Everything Settings › Commands lists, from a fresh scan. Runtime only. The "WackCode" group
/// is the app's own five commands, which the renderer adds itself (`command-utils.ts`): the
/// scan never sees them because the picker offers them before the worker catalog is consulted.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandsOverview {
    /// `<app data>/commands`, where the user's own command files live.
    pub custom_path: String,
    /// The switched-off keys as saved, for the renderer's own "WackCode" rows.
    pub disabled: Vec<String>,
    pub groups: Vec<SlashCommandGroup>,
}

/// A change's result: the fresh list, the config as saved, and a note when part was skipped.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandsChange {
    pub overview: SlashCommandsOverview,
    pub config: CommandsConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSlashCommandInput {
    /// The command file being rewritten; absent creates a new one.
    pub path: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub argument_hint: String,
    pub body: String,
}

/// What the command editor needs beyond the list: the template's body.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandDocument {
    pub body: String,
}

/// One project's memory directory as Settings lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProject {
    /// The hash suffix of the directory name; what the per-project switch stores.
    pub key: String,
    /// The project's name when a known project maps to this directory, else the folder name.
    pub name: String,
    /// The repository or folder the memory belongs to, from the directory's `origin.json`.
    pub path: String,
    /// `<app data>/memory/<name>-<key>`.
    pub dir: String,
    pub entries: Vec<MemoryEntry>,
    /// The global switch is on and this project hasn't been switched off.
    pub enabled: bool,
}

/// One note file, parsed for the list. Files without a title stay invisible (the worker's
/// index skips them too); the trash folder never appears.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    /// Filename without `.md` — the id `memory_recall` takes.
    pub name: String,
    pub file_path: String,
    /// user / feedback / project / reference.
    pub kind: String,
    pub title: String,
    pub description: String,
    /// ISO 8601, when the note was last written.
    pub modified: Option<String>,
}

/// Everything Settings › Memory lists, from a fresh scan.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoriesOverview {
    pub enabled: bool,
    pub projects: Vec<MemoryProject>,
}

/// A change's result: the fresh list and the config as saved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoriesChange {
    pub overview: MemoriesOverview,
    pub config: MemoryConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMemoryInput {
    /// The note file being rewritten; absent creates a new one.
    pub path: Option<String>,
    /// The project's memory directory, where a new note is created.
    pub dir: String,
    pub name: String,
    /// user / feedback / project / reference.
    pub memory_type: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub body: String,
}

/// What the note editor needs beyond the list: the note's body.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDocument {
    pub body: String,
}

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
    /// pi.dev results: the resource types its catalogue lists (`extension`, `skill`, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// pi.dev results: downloads in the last month.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
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
    /// Settings › Skills: switch on only the package's skills.
    #[serde(default)]
    pub skills_only: bool,
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
    /// Liquid Glass needs `NSGlassEffectView` (macOS 26+). Runtime-only, never stored.
    pub glass_supported: bool,
    /// Computer use needs `SCScreenshotManager` (macOS 14+). Runtime-only, never stored.
    pub computer_use_supported: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChangeFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub binary: bool,
    pub hunkable: bool,
    pub truncated: bool,
    pub diff: String,
    pub sections: Vec<GitDiffSection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffSection {
    pub layer: String,
    pub revision: String,
    pub diff: String,
    pub hunks: Vec<GitDiffHunk>,
    pub truncated: bool,
    /// Line counts over the full diff, so a truncated preview still reports exact totals.
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffHunk {
    pub id: usize,
    pub header: String,
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<GitDiffLine>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffLine {
    pub kind: String,
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFiles {
    pub files: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChanges {
    pub is_git: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    pub files: Vec<GitChangeFile>,
    /// Guard hash over every changed file's path, status, and section revisions.
    /// Commit and bulk actions compare against this so a moved working tree is rejected.
    pub changes_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPublishInfo {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPrInfo {
    pub repo: String,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body: String,
    pub existing_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGeneratedMessage {
    pub message: String,
    pub revision: String,
}

/// One chunk on a terminal's output channel (`terminal.rs`). `output` carries UTF-8 terminal
/// bytes ready to write into xterm; `exit` ends the session; `busy` reflects whether the shell
/// has a foreground job, so the header button can hint at work the user isn't watching.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalFrame {
    #[serde(rename_all = "camelCase")]
    Output { data: String },
    #[serde(rename_all = "camelCase")]
    Exit { code: Option<i32>, signal: Option<String> },
    #[serde(rename_all = "camelCase")]
    Busy { busy: bool },
}

/// How a terminal's shell ended; `signal` names the signal when it didn't exit normally.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExit {
    pub code: i32,
    pub signal: Option<String>,
}

/// What `open_terminal` returns when the panel attaches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    /// Identifies this shell; a restart's late events carry the old id and are ignored.
    pub session_id: String,
    pub shell: String,
    pub cwd: String,
    /// True when this call spawned the shell; false when it attached to a live one.
    pub fresh: bool,
    pub exit: Option<TerminalExit>,
    pub busy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTerminalInput {
    pub task_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTerminalInput {
    pub task_id: String,
    /// xterm `onData` output: UTF-8 text including escape sequences for special keys.
    pub data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeTerminalInput {
    pub task_id: String,
    pub cols: u16,
    pub rows: u16,
}
