export type ApiFormat = "openai-completions" | "openai-responses";
export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type TaskStatus = "idle" | "running" | "stopping" | "interrupted" | "error";

/**
 * The agent's working mode. "plan" is the read-only, plan-first mode; "ultraplan" is the same
 * read-only mode with an exhaustive, one-question-at-a-time interview before the plan.
 */
export type TaskMode = "build" | "plan" | "ultraplan";

/** Plan mode state published by the worker's built-in plan-mode extension. */
export interface PlanState {
  mode: TaskMode;
  /** "ready" once the agent has submitted a complete plan; cleared by revision or leaving Plan mode. */
  phase: "planning" | "ready";
  /** The completed plan awaiting approval (phase === "ready"). */
  plan?: string;
}

/** One task in the built-in todo extension's task list. */
export type TodoStatus = "pending" | "in_progress" | "completed" | "deleted";

export interface TodoTask {
  id: number;
  subject: string;
  description?: string;
  /** Present-continuous label shown while status is in_progress. */
  activeForm?: string;
  status: TodoStatus;
  /** Ids of tasks that must complete before this one can start. */
  blockedBy?: number[];
}

/** Todo list state published by the worker's built-in todo extension. */
export interface TodoState {
  /** Carries tombstones too; hiding deleted tasks is the renderer's concern. */
  tasks: TodoTask[];
}

/**
 * Goal-loop state published by the worker's built-in goal extension: `/goal <objective>` runs
 * rounds of work and verifies each one until the verifier passes or a guard stops it.
 */
export interface GoalState {
  objective: string;
  phase: "active" | "verifying" | "paused" | "complete" | "stopped";
  /** Verified rounds so far. */
  iteration: number;
  maxIterations: number;
  /** Consecutive rounds with no detected progress. */
  noProgress: number;
  lastReason?: string;
  lastNextAction?: string;
  /** Paused/stopped explanation, e.g. "Stopped by user." */
  note?: string;
}

/** One option in an ask_user_question question. */
export interface AskQuestionOption {
  label: string;
  description: string;
}

/** A structured question the ask_user_question tool puts to the user. */
export interface AskQuestion {
  id: string;
  header: string;
  question: string;
  multiSelect?: boolean;
  options: AskQuestionOption[];
}

/** One answered question, returned to the tool that asked it. */
export interface QuestionAnswer {
  questionId: string;
  /** Labels of the chosen preset options (empty when the user wrote a custom answer). */
  selected: string[];
  custom?: string;
}

export interface ModelRecord {
  id: string;
  name: string;
  contextWindow: number | null;
  maxTokens: number | null;
  reasoning: boolean;
  thinkingLevels: ThinkingLevel[];
  thinkingLevelMap: Partial<Record<ThinkingLevel, string | null>>;
  /** Accepts image input (Pi's `input: ["text", "image"]`). Off until the user confirms it. */
  vision: boolean;
}

/** Settings-only metadata from the Pi version bundled with WackCode. */
export interface BuiltinModelSuggestion extends ModelRecord {
  sourceProvider: string;
  sourceApi: string;
  contextWindow: number;
  maxTokens: number;
}

/**
 * An image attached to a prompt, in Pi's own `ImageContent` shape. It travels unchanged through
 * Rust and the worker into `session.prompt`, so it must match worker/src/protocol.ts and models.rs.
 */
export interface ImageContent {
  type: "image";
  /** Base64 without a `data:` prefix. */
  data: string;
  mimeType: string;
}

export interface SlashCommand {
  id: string;
  name: string;
  description?: string;
  /** "custom" is a file in the user's own commands folder (Settings › Commands). */
  source: "app" | "extension" | "prompt" | "custom" | "skill";
  sourceLabel: string;
  /** What the model can call with arguments: a template's or skill's `argument-hint`, shown next to its name. `<arg>` reads required, `[arg]` optional. */
  argumentHint?: string;
}

/**
 * Settings › Commands, as saved. Which `commandKey`s (`app:` / `extension:` / `prompt:` /
 * `custom:` / `skill:`) are switched off. Mirrors models.rs.
 */
export interface CommandsConfig {
  disabled: string[];
}

/** Where one listed command comes from; "app" rows are the desktop's own (added client-side). */
export type SlashCommandKind = "app" | "extension" | "prompt" | "custom";

export interface SlashCommandEntry {
  /** Its `commandKey` — the same id a chat's `SlashCommand.id` reports. */
  key: string;
  /** What `/` offers: the clash-resolved name, or the command's own name while switched off. */
  name: string;
  /** The name before a clash renamed it to `<source>:<name>`, when it was. */
  rawName?: string;
  description: string;
  argumentHint?: string;
  kind: SlashCommandKind;
  enabled: boolean;
  /** The user's own command file may be edited and deleted from Settings. */
  editable: boolean;
  /** The template file for prompt/custom, the extension file for extension. */
  filePath?: string;
}

export interface SlashCommandGroup {
  /** "custom" for the user's own commands, else the package's `source`. */
  id: string;
  label: string;
  kind: "custom" | "package";
  entries: SlashCommandEntry[];
  diagnostics: SkillDiagnostic[];
}

/** Everything Settings › Commands lists, from a fresh scan. Runtime only. */
export interface SlashCommandsOverview {
  /** `<app data>/commands`, where the user's own command files live. */
  customPath: string;
  /** The switched-off keys as saved, for the renderer's own "WackCode" rows. */
  disabled: string[];
  groups: SlashCommandGroup[];
}

/** A change's result: the fresh list, the config as saved, and a note when part was skipped. */
export interface SlashCommandsChange {
  overview: SlashCommandsOverview;
  config: CommandsConfig;
  note?: string;
}

export interface SlashCommandDocument {
  body: string;
}

export interface SaveSlashCommandInput {
  /** The command file being rewritten; absent creates a new one. */
  path?: string;
  name: string;
  description?: string;
  argumentHint?: string;
  body: string;
}

interface ProviderBase {
  id: string;
  name: string;
  models: ModelRecord[];
  createdAt: string;
  updatedAt: string;
}

export interface CustomProviderRecord extends ProviderBase {
  kind: "custom";
  baseUrl: string;
  apiFormat: ApiFormat;
  hasApiKey: boolean;
  connected: boolean;
}

export interface SubscriptionProviderRecord extends ProviderBase {
  kind: "subscription";
  baseUrl: string;
  apiFormat: string;
  hasApiKey: false;
  connected: boolean;
}

export type ProviderRecord = CustomProviderRecord | SubscriptionProviderRecord;

export interface SubscriptionProviderInfo {
  id: string;
  name: string;
  guidance: string;
}

export type SubscriptionLoginEvent = {
  loginId: string;
  providerId: string;
} & (
  | { type: "prompt"; promptId: string; prompt: { type: "text" | "secret" | "select" | "manual_code"; message: string; placeholder?: string; options?: { id: string; label: string; description?: string }[] } }
  | { type: "auth_url"; url: string; instructions?: string }
  | { type: "device_code"; userCode: string; verificationUri: string; expiresInSeconds?: number }
  | { type: "info" | "progress"; message: string }
  | { type: "complete"; provider: SubscriptionProviderRecord }
  | { type: "error"; message: string }
  | { type: "cancelled" }
);

export interface ProjectRecord {
  id: string;
  name: string;
  path: string;
  gitRoot: string | null;
  gitHasHead: boolean;
  branch: string | null;
  createdAt: string;
}

export interface TaskRecord {
  id: string;
  projectId: string | null;
  name: string;
  autoTitleEligible: boolean;
  autoTitleAttemptId: string | null;
  workspacePath: string;
  worktreePath: string | null;
  branch: string | null;
  usesWorktree: boolean;
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  sessionFile: string | null;
  status: TaskStatus;
  /** Mirrors the worker's `plan_state`; the durable hint the UI uses before a worker reports in. */
  mode: TaskMode;
  archived: boolean;
  lastError: string | null;
  createdAt: string;
  updatedAt: string;
}

/** One resource file a package contributes. */
export interface PackageResourceRecord {
  /** Absolute path on disk. */
  path: string;
  /** Path relative to the package root, which is what the user sees. */
  name: string;
  enabled: boolean;
}

export type PackageResourceKind = "extensions" | "skills" | "prompts" | "themes";

export const PACKAGE_RESOURCE_KINDS: PackageResourceKind[] = ["extensions", "skills", "prompts", "themes"];

/** An installed Pi package. Without `trustedAt` nothing it contains is ever loaded. */
export interface PackageRecord {
  source: string;
  displayName: string;
  kind: "npm" | "git" | "local";
  version?: string;
  installedPath?: string;
  extensions: PackageResourceRecord[];
  skills: PackageResourceRecord[];
  prompts: PackageResourceRecord[];
  themes: PackageResourceRecord[];
  errors: string[];
  trustedAt: string;
  installedAt: string;
}

/** One npm registry search hit in the Browse tab. */
export interface PackageSearchResult {
  name: string;
  version: string;
  description: string;
  publisher: string;
  npmUrl: string;
  repository?: string;
  publishedAt: string;
  /** Resource kinds the package's `pi` manifest declares. Empty until details are fetched. */
  declares: string[];
  /** pi.dev results: the resource types its catalogue lists ("extension", "skill", …). */
  types?: string[];
  /** pi.dev results: downloads in the last month. */
  downloads?: number;
}

/** One page of Settings › Skills › Browse. */
export interface SkillSearchPage {
  results: PackageSearchResult[];
  /** "npm" when pi.dev could not be read and npm's keyword search stood in. */
  source: "pidev" | "npm";
  hasMore: boolean;
}

export type SkillSearchSort = "downloads" | "recent" | "name";

/** Where a tool came from, so Settings can group and attribute it. */
export interface ToolSource {
  /** "wackcode" tools ship inside the app itself and can't be switched off. "mcp" tools come
   *  from the user's MCP servers; their switches are in Settings › MCP servers. */
  kind: "builtin" | "package" | "wackcode" | "mcp";
  packageId?: string;
  path?: string;
  /** The MCP server's id when kind is "mcp". */
  serverId?: string;
}

export interface ToolCatalogEntry {
  name: string;
  description: string;
  source: ToolSource;
  /** False when a required external binary is missing; the tool is not offered to the model. */
  available: boolean;
  unavailableReason?: string;
}

/** Tools the user switched off. A denylist, so a newly added tool is on by default. */
export interface ToolConfig {
  disabled: string[];
}

/** How WackCode reaches an MCP server. Mirrors `McpTransport` in models.rs. */
export type McpTransport = "stdio" | "http" | "sse";

/** Mirrors `mcp.rs`: the default, and the range the host accepts. */
export const MCP_DEFAULT_TIMEOUT_MS = 120_000;
export const MCP_MIN_TIMEOUT_MS = 1_000;
export const MCP_MAX_TIMEOUT_MS = 3_600_000;

export interface McpToolInfo {
  name: string;
  description: string;
  /** The server marks it read-only, so Plan mode lets it through. */
  readOnly: boolean;
}

/**
 * One MCP server (Settings › MCP servers). Header and environment variable values never reach
 * the renderer: only their names, and the values stay in `secrets.json`.
 */
export interface McpServerRecord {
  id: string;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  timeoutMs: number;
  command: string;
  args: string[];
  url: string;
  headers: string[];
  env: string[];
  /** The server's own tool names the user switched off. */
  disabledTools: string[];
  /** The server's tools as of the last successful "Test connection". */
  tools: McpToolInfo[];
}

export interface McpConfig {
  servers: McpServerRecord[];
}

/**
 * Settings › Skills, as saved. The user's own skills are files in `~/.agents/skills`; this keeps
 * only which other folders load and which skills are switched off. Mirrors models.rs.
 */
export interface SkillsConfig {
  folders: SkillFolderRecord[];
  disabled: string[];
}

export interface SkillFolderRecord {
  /** A known folder's id (`claude`, `codex`, `pi`, `opencode`) or `custom:<uuid>`. */
  id: string;
  path?: string;
  enabled: boolean;
}

/** "library" is `~/.agents/skills`: always on, and the only folder WackCode writes to. */
export type SkillFolderKind = "library" | "tool" | "custom";

export interface SkillEntry {
  name: string;
  description: string;
  filePath: string;
  baseDir: string;
  /** `disable-model-invocation: true`: only `/skill:name` uses it. */
  manual: boolean;
  /** The SKILL.md's optional `argument-hint`, shown next to the name here and in the picker. */
  argumentHint?: string;
  enabled: boolean;
  /** In `~/.agents/skills`, so it can be edited and deleted here. */
  editable: boolean;
  /** Switched on, but a skill of the same name wins: that skill's folder or package. */
  shadowedBy?: string;
  /** Package skills: the resource its switch toggles. */
  resourceName?: string;
}

export interface SkillDiagnostic {
  kind: "warning" | "error";
  message: string;
  path?: string;
}

export interface SkillFolderView {
  id: string;
  label: string;
  path: string;
  displayPath: string;
  kind: SkillFolderKind;
  exists: boolean;
  enabled: boolean;
  skills: SkillEntry[];
  diagnostics: SkillDiagnostic[];
}

export interface SkillPackageView {
  source: string;
  label: string;
  skills: SkillEntry[];
  diagnostics: SkillDiagnostic[];
}

/** Every skill folder and every trusted package's skills, from a fresh scan. Runtime only. */
export interface SkillsOverview {
  libraryPath: string;
  folders: SkillFolderView[];
  packages: SkillPackageView[];
}

/** A change's result: the fresh list, and a note when part of it was skipped. */
export interface SkillsChange {
  overview: SkillsOverview;
  note?: string;
}

export interface SkillDocument {
  body: string;
  /** Relative to the skill's folder, `SKILL.md` left out. */
  files: string[];
  filesTruncated: boolean;
}

export interface SaveSkillInput {
  /** The skill's file when editing; absent to create one. */
  path?: string;
  name: string;
  description: string;
  manual: boolean;
  /** Optional `argument-hint` frontmatter; empty writes no key. */
  argumentHint?: string;
  body: string;
}

/** A header or environment variable as the editor sends it. A blank value keeps the saved one. */
export interface McpSecretInput {
  name: string;
  value?: string;
}

export interface SaveMcpServerInput {
  id?: string;
  name: string;
  transport: McpTransport;
  timeoutMs: number;
  command: string;
  args: string[];
  url: string;
  headers: McpSecretInput[];
  env: McpSecretInput[];
}

export interface McpTestResult {
  ok: boolean;
  error?: string;
  /** The server as saved after the test (with the tools it listed). */
  server: McpServerRecord;
}

/**
 * Cosmetic preferences (Settings → Appearance). Mirrors `AppearanceConfig` in models.rs. The
 * renderer themes itself from these (`theme.ts`); Rust applies the backdrop to the window.
 */
export interface AppearanceConfig {
  /** One-line gist of the reasoning beside a live "Thinking…" row. */
  thinkingPreview: boolean;
  /** Assistant prose in a bubble like the user's; the user's bubble always shows. */
  messageBubbles: boolean;
  /** Fold runs of read-only tool calls into one "Explored" row in the transcript. */
  groupExploration: boolean;
  /** `#rrggbb`; absent is WackCode green. */
  accent?: string | null;
  /** `#rrggbb` as displayed (already darkened for readability); absent is the default. */
  background?: string | null;
  backdrop: BackdropMode;
  /** File name in `<app data>/backgrounds/`; only the image commands change it. */
  backgroundImage?: string | null;
  /** How much the background colour covers the image behind a chat, 0–90 %. */
  imageDim: number;
  /** Blur of the image behind a chat, 0–40 px. */
  imageBlur: number;
  glassStyle: GlassStyle;
  /** How much the background colour tints the glass, 0–90 %. */
  glassTint: number;
  /** What the app calls the agent in its own copy; absent is `WackCode`. */
  agentName?: string | null;
}

/** What sits behind the app's panels. Exclusive: glass shows the desktop an image would cover. */
export type BackdropMode = "solid" | "image" | "glass";
export type GlassStyle = "frosted" | "clear";

/**
 * User-customized built-in prompt texts (Settings → Prompts). Mirrors `PromptConfig` in
 * models.rs and `PromptOverrides` in worker/src/protocol.ts. An absent field means the
 * shipped default is in force.
 */
export interface PromptConfig {
  /** Replaces Pi's default system-prompt persona; the assembled sections still follow it. */
  systemPrompt?: string | null;
  /** Replaces the Plan-mode contract body (the marker line stays the app's own). */
  planPrompt?: string | null;
  /** Replaces the Ultra Plan contract body (the marker line stays the app's own). */
  ultraPlanPrompt?: string | null;
}

/** When the chat's agent should reach for sub-agents. Only changes the tool's guidance. */
export type SubagentTrigger = "on_request" | "auto";

/** Tools a sub-agent can be given: Pi's own, plus the built-in web_fetch. Mirrors `CHILD_TOOLS` in subagents.rs. */
export const SUBAGENT_TOOLS = ["read", "grep", "find", "ls", "bash", "edit", "write", "web_fetch"] as const;
export type SubagentTool = (typeof SUBAGENT_TOOLS)[number];
/** What a read-only sub-agent may have. */
export const READ_ONLY_SUBAGENT_TOOLS: readonly string[] = ["read", "grep", "find", "ls", "bash", "web_fetch"];
export const MAX_SUBAGENT_CONCURRENCY = 8;

/** A sub-agent's own model; without one it runs on the chat's model. */
export interface SubagentModel {
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
}

/** One agent the `subagent` tool can launch. Built-ins: only `enabled` and `model` are editable. */
export interface SubagentRecord {
  id: string;
  builtin: boolean;
  enabled: boolean;
  name: string;
  description: string;
  prompt: string;
  tools: string[];
  /** No edit/write, bash limited to inspection, and allowed in Plan mode. */
  readOnly: boolean;
  model: SubagentModel | null;
}

/** The sub-agents built-in extension. Off by default: every child is extra model usage. */
export interface SubagentConfig {
  enabled: boolean;
  trigger: SubagentTrigger;
  maxConcurrency: number;
  agents: SubagentRecord[];
}

export interface AutoTitleConfig {
  enabled: boolean;
  providerId: string | null;
  modelId: string | null;
}

export type SubagentStatus = "queued" | "running" | "done" | "failed" | "aborted";

/**
 * One sub-agent in a `subagent` call, as its chip and panel render it. Mirrors
 * worker/src/protocol.ts, less the saved transcript: the worker strips that from snapshots and
 * streams it to the side panel instead (`watch_subagent`).
 */
export interface SubagentResult {
  agent: string;
  task: string;
  readOnly: boolean;
  model?: string;
  status: SubagentStatus;
  activity: { tool: string; subject: string }[];
  output?: string;
  outputTruncated?: boolean;
  error?: string;
  usage: { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number; turns: number };
  startedAt?: number;
  endedAt?: number;
}

/** The `subagent` tool's result details, live and after a reload. */
export interface SubagentDetails {
  v: 1;
  mode: "single" | "parallel";
  results: SubagentResult[];
}

/** One child of one `subagent` call: its tool call and position. Mirrors `SubagentWatchTarget` in models.rs. */
export interface SubagentTarget {
  toolCallId: string;
  index: number;
}

/**
 * One frame of the watched child's transcript. Mirrors worker/src/protocol.ts: a `reset` frame
 * carries the whole transcript in `upserts`; the rest chain by `rev` and merge like snapshot deltas.
 */
export interface SubagentStreamFrame extends SubagentTarget {
  rev: number;
  reset?: true;
  upserts: NormalizedMessage[];
  removed: string[];
  partial: NormalizedMessage | null;
  live: boolean;
  truncated?: boolean;
  missing?: boolean;
}

/** The side panel's copy of the watched child's transcript (`TaskRuntime.subagentView`). */
export interface SubagentView extends SubagentTarget {
  /** The last frame applied; -1 until the first (reset) frame arrives. */
  rev: number;
  messages: NormalizedMessage[];
  partial?: NormalizedMessage;
  /** The child is still running. */
  live: boolean;
  /** Older tool output was trimmed when the transcript was saved. */
  truncated: boolean;
  /** No transcript: not started yet, or a call from before transcripts were kept. */
  missing: boolean;
  /** Waiting for the first frame. */
  loading: boolean;
  /** A frame went missing or the worker restarted: watch again for a fresh reset. */
  resync?: boolean;
  error?: string;
}

/** The main window's last normal-mode geometry in logical points. Mirrors `WindowState` in models.rs. */
export interface WindowState {
  width: number;
  height: number;
  x?: number | null;
  y?: number | null;
}

export interface AppData {
  version: number;
  providers: ProviderRecord[];
  projects: ProjectRecord[];
  tasks: TaskRecord[];
  diffComments: Record<string, DiffComment[]>;
  toolConfig: ToolConfig;
  toolCatalog: ToolCatalogEntry[];
  packages: PackageRecord[];
  subagents: SubagentConfig;
  autoTitle: AutoTitleConfig;
  appearance: AppearanceConfig;
  prompts: PromptConfig;
  mcp: McpConfig;
  skills?: SkillsConfig;
  commands?: CommandsConfig;
  window?: WindowState | null;
}

export interface BootstrapPayload {
  data: AppData;
  appDataPath: string;
  /** Liquid Glass needs macOS 26+ (`NSGlassEffectView`). */
  glassSupported: boolean;
}

export interface NormalizedBlock {
  type: "text" | "thinking" | "tool-call" | "tool-result" | "image";
  text?: string;
  /** Image blocks: the original's type. Snapshots never carry the full image. */
  mimeType?: string;
  /** Image blocks: stable for the life of the worker. */
  imageId?: string;
  /** Image blocks: a small `data:` URL preview, absent until the worker has generated it. */
  thumbnail?: string;
  /** Thinking blocks: how long the model reasoned. Absent while it still is, and when never clocked. */
  durationMs?: number;
  toolName?: string;
  toolCallId?: string;
  arguments?: unknown;
  isError?: boolean;
  details?: unknown;
}

/**
 * A workspace checkpoint: a tree in the chat's private shadow repository, taken before each
 * prompt and when a branch of the conversation is left.
 */
export interface CheckpointRef {
  id: string;
  /** The workspace repository's HEAD when the snapshot was taken, if it had one. */
  head?: string;
}

/** The other versions of a user message: edits and retries sent from the same point. */
export interface MessageVersions {
  /** Zero-based position among the versions, oldest first. */
  index: number;
  total: number;
  previous?: string;
  next?: string;
  /** The same for every version of the message. */
  group: string;
}

/** Where a turn (one user message and everything answering it) ends, for retry and fork. */
export interface TurnInfo {
  userEntryId: string;
  endEntryId: string;
  /** The files as they were after this turn, when a later checkpoint recorded them. */
  after?: CheckpointRef;
}

/** How a model-visible user prompt was produced by a slash command. */
export interface CommandPresentation {
  /** Stable catalog identity, such as `app:goal` or `prompt:/path/to/file.md`. */
  id: string;
  /** Resolved name the composer showed, without the leading slash. */
  name: string;
  /** Exactly what followed the command name before expansion. */
  arguments: string;
  kind: "command" | "goal-continuation" | "goal-resume";
  /** Goal continuations and resumes: the working round this prompt starts. */
  round?: number;
  /** Goal continuations and resumes: the verifier's next action shown in the compact row. */
  nextAction?: string;
}

export interface NormalizedMessage {
  /** The Pi session entry id when known; otherwise a positional id. */
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  timestamp?: number;
  blocks: NormalizedBlock[];
  stopReason?: string;
  errorMessage?: string;
  /** Pi session entry id. Briefly absent while Pi saves a message that has just finished. */
  entryId?: string;
  versions?: MessageVersions;
  /** User messages: the files just before the message was sent. */
  checkpoint?: CheckpointRef;
  /** User messages generated by a command; the blocks remain the exact model-visible prompt. */
  commandPresentation?: CommandPresentation;
  /** The last assistant message of each turn. */
  turn?: TurnInfo;
}

/** One file restoring a checkpoint would change, and what the restore does to it. */
export interface CheckpointChange {
  path: string;
  status: "revert" | "delete" | "recreate";
}

export interface RestoreResult {
  restored: string[];
  /** Left alone: ignored or oversized files, or a folder with other content in the way. */
  skipped: string[];
  /** The files just before the restore, itself a checkpoint. */
  undo: CheckpointRef;
}

export interface NavigateResult {
  leafId: string | null;
  /** The text of the user message a rewind removed. */
  editorText?: string;
  /** The files the branch now shown was left with. */
  files?: CheckpointRef;
}

export interface NavigateTaskResult {
  navigate: NavigateResult;
  restore?: RestoreResult | null;
  /** The conversation moved, but the requested file restore failed. */
  restoreError?: string | null;
}

/** A completed prompt duration attached to the user message that started it. */
export interface RunTiming {
  userMessageId: string;
  durationMs: number;
}

export interface ModelRef {
  providerId: string;
  modelId: string;
}

/** A durable model change, inserted at `at` in the normalized transcript. */
export interface ModelSwitch {
  /** Pi's model_change entry id, stable across snapshots and reloads. */
  id: string;
  /** Zero-based insertion point: messages before this index render above the divider. */
  at: number;
  from: ModelRef;
  to: ModelRef;
}

export interface SessionSnapshot {
  /** Bumped by every full snapshot and delta; lets the renderer chain deltas to a snapshot. */
  rev: number;
  sessionId: string;
  sessionFile?: string;
  messages: NormalizedMessage[];
  modelSwitches: ModelSwitch[];
  runTimings: RunTiming[];
  activeRun?: { runId: string; startedAt: number };
  /** Where the conversation currently ends in the session tree. */
  tree?: {
    leafId: string | null;
    /** Set right after a rewind: the entry "Undo rewind" returns to. */
    undo?: string;
  };
  stats: {
    tokens: { input: number; output: number; cacheRead: number; cacheWrite: number; total: number };
    cost: number;
    contextUsage?: { tokens: number | null; contextWindow: number; percent: number | null };
    contextBreakdown?: {
      entries: { id: "system" | "user" | "assistant" | "tool"; tokens: number }[];
      cacheHitRate: number | null;
    };
  };
  thinkingLevel: ThinkingLevel;
  availableThinkingLevels: ThinkingLevel[];
  model?: { provider: string; id: string; name?: string };
  tools: ToolCatalogEntry[];
  activeTools: string[];
  planState?: PlanState;
  todoState?: TodoState;
  goalState?: GoalState;
}

/**
 * An incremental snapshot: applies on top of the snapshot or delta carrying `rev - 1`. Removed
 * ids go first, then each upsert replaces its message by id or appends when the id is new.
 * Current model, thinking level and tool fields only ride full snapshots.
 */
export interface SnapshotDelta {
  rev: number;
  /** New or changed messages, keyed by id. */
  upserts: NormalizedMessage[];
  /** Ids that left the transcript, applied before the upserts. */
  removed: string[];
  /** Present only when the field changed since the last emission; absent means unchanged. */
  runTimings?: RunTiming[];
  /** Complete replacement when the active branch's model switches change. */
  modelSwitches?: ModelSwitch[];
  /** null clears the active run; absent leaves it unchanged. */
  activeRun?: { runId: string; startedAt: number } | null;
  tree: {
    leafId: string | null;
    undo?: string;
  };
  stats: SessionSnapshot["stats"];
  sessionFile?: string;
  planState?: PlanState;
  todoState?: TodoState;
  /** null clears the goal; absent leaves it unchanged. */
  goalState?: GoalState | null;
}

/** A question an extension asked, mirrored from the worker protocol. */
export type ExtensionUIRequest = { taskId: string; requestId: string } & (
  | { method: "select"; title: string; options: string[] }
  | { method: "confirm"; title: string; message: string }
  | { method: "input"; title: string; placeholder?: string }
  | { method: "editor"; title: string; prefill?: string }
  | {
      method: "questions";
      title: string;
      questions: AskQuestion[];
      /** Ultra Plan: offer "Write the plan now", answered with `wrapUp` instead of answers. */
      offerWrapUp?: true;
    }
);

export interface ExtensionNotice {
  message: string;
  level: "info" | "warning" | "error";
}

/** Compact runtime state for one chat's native, ephemeral browser session. */
export interface BrowserState {
  taskId: string;
  exists: boolean;
  url: string;
  title: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  error?: string;
  agentActive: boolean;
  userControl: boolean;
  popup: boolean;
}

export type WorkerEvent =
  | { type: "title_changed"; taskId: string; name: string }
  | { type: "ready" | "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot_delta"; taskId: string; delta: SnapshotDelta }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; startedAt?: number; state: TaskStatus }
  | { type: "queue_state"; taskId: string; steering: string[]; followUp: string[] }
  | { type: "activity"; taskId: string; event: string; detail?: Record<string, unknown> }
  | { type: "worker_error"; taskId?: string; message: string }
  | { type: "browser_state"; taskId: string; browser: BrowserState; reveal: boolean }
  | { type: "response"; taskId?: string; id: string; success: boolean; error?: string }
  | ({ type: "extension_ui_request" } & ExtensionUIRequest)
  | ({ type: "extension_notice"; taskId: string } & ExtensionNotice)
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] }
  | ({ type: "plan_state"; taskId: string } & PlanState)
  | ({ type: "todo_state"; taskId: string } & TodoState)
  | { type: "goal_state"; taskId: string; goal: GoalState | null }
  /** The watched sub-agent's transcript (`watch_subagent`). */
  | ({ type: "subagent_stream"; taskId: string } & SubagentStreamFrame)
  /** From the host, once per chat per session: why file checkpoints are off for it. */
  | { type: "checkpoint_unavailable"; taskId: string; message: string };

export interface TaskRuntime {
  slashCommands?: SlashCommand[];
  slashCommandsLoading?: boolean;
  slashCommandsError?: string;
  snapshot?: SessionSnapshot;
  partial?: NormalizedMessage;
  /** Active prompt clock. The worker event supplies the run id after accepting the prompt. */
  activeRun?: { runId?: string; startedAt: number };
  activity?: string;
  /** Accumulated text from in-flight tools, keyed by Pi's tool call id. */
  liveToolText?: Record<string, string>;
  /** Structured progress from in-flight tools that report it (sub-agents), keyed like liveToolText. */
  liveToolDetails?: Record<string, unknown>;
  error?: string;
  /** Extension output and load failures. Informational only — never blocks a chat. */
  notices?: ExtensionNotice[];
  /** Latest Plan mode state from the worker; falls back to `TaskRecord.mode` before `ready`. */
  planState?: PlanState;
  /** Latest todo list from the worker's built-in todo extension. */
  todoState?: TodoState;
  /** Latest goal-loop state from the worker's built-in goal extension. */
  goalState?: GoalState;
  /** The transcript of the sub-agent shown in the side panel, while one is. */
  subagentView?: SubagentView;
  /** Messages queued on the running prompt: steering delivers at the next boundary, followUp after the run. */
  queued?: { steer: string[]; followUp: string[] };
  /** The most recent file restore, offered for undo until dismissed. */
  lastRestore?: { count: number; undo: CheckpointRef };
}

export interface GitChangeFile {
  path: string;
  oldPath: string | null;
  status: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  binary: boolean;
  hunkable: boolean;
  truncated: boolean;
  diff: string;
  sections: GitDiffSection[];
}

export interface GitDiffSection {
  layer: "staged" | "working";
  revision: string;
  diff: string;
  hunks: GitDiffHunk[];
  truncated: boolean;
  /** Exact line counts over the full diff, even when `diff` is a truncated preview. */
  additions: number;
  deletions: number;
}

export interface GitDiffLine {
  kind: string;
  text: string;
  oldLine: number | null;
  newLine: number | null;
}

export interface GitDiffHunk {
  id: number;
  header: string;
  oldStart: number;
  newStart: number;
  lines: GitDiffLine[];
}

export interface DiffComment {
  id: string;
  path: string;
  layer: "staged" | "working";
  side: "old" | "new";
  line: number;
  excerpt: string;
  revision: string;
  text: string;
}

/** Files `@` mentions can pick from, relative to the workspace. */
export interface WorkspaceFiles {
  files: string[];
  truncated: boolean;
}

export interface GitChanges {
  isGit: boolean;
  root: string | null;
  branch: string | null;
  files: GitChangeFile[];
  /** Guard hash over every changed file's path, status and section revisions. */
  changesRevision: string;
}

export interface GitPublishInfo {
  branch: string | null;
  upstream: string | null;
  remotes: string[];
}

export interface GitPrInfo {
  repo: string;
  base: string;
  head: string;
  title: string;
  body: string;
  existingUrl: string | null;
}

export interface GitGeneratedMessage {
  message: string;
  revision: string;
}

export interface SaveProviderInput {
  id?: string;
  name: string;
  baseUrl: string;
  apiFormat: ApiFormat;
  models: ModelRecord[];
  apiKey?: string;
}
