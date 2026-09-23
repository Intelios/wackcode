export type ApiFormat = "openai-completions" | "openai-responses";
export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type TaskStatus = "idle" | "running" | "stopping" | "interrupted" | "error";

/** The agent's working mode. "plan" is the read-only, plan-first mode. */
export type TaskMode = "build" | "plan";

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
}

/** Where a tool came from, so Settings can group and attribute it. */
export interface ToolSource {
  /** "wackcode" tools ship inside the app itself and can't be switched off. */
  kind: "builtin" | "package" | "wackcode";
  packageId?: string;
  path?: string;
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

/** When the chat's agent should reach for sub-agents. Only changes the tool's guidance. */
export type SubagentTrigger = "on_request" | "auto";

/** Pi's own tools a sub-agent can be given. Mirrors `CHILD_TOOLS` in subagents.rs. */
export const SUBAGENT_TOOLS = ["read", "grep", "find", "ls", "bash", "edit", "write"] as const;
export type SubagentTool = (typeof SUBAGENT_TOOLS)[number];
/** What a read-only sub-agent may have. */
export const READ_ONLY_SUBAGENT_TOOLS: readonly string[] = ["read", "grep", "find", "ls", "bash"];
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

export type SubagentStatus = "queued" | "running" | "done" | "failed" | "aborted";

/** One sub-agent in a `subagent` call, as the card renders it. Mirrors worker/src/protocol.ts. */
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

export interface AppData {
  version: number;
  providers: ProviderRecord[];
  projects: ProjectRecord[];
  tasks: TaskRecord[];
  toolConfig: ToolConfig;
  toolCatalog: ToolCatalogEntry[];
  packages: PackageRecord[];
  subagents: SubagentConfig;
}

export interface BootstrapPayload {
  data: AppData;
  appDataPath: string;
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

export interface SessionSnapshot {
  /** Bumped by every full snapshot and delta; lets the renderer chain deltas to a snapshot. */
  rev: number;
  sessionId: string;
  sessionFile?: string;
  messages: NormalizedMessage[];
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
}

/**
 * An incremental snapshot: applies on top of the snapshot or delta carrying `rev - 1`. Removed
 * ids go first, then each upsert replaces its message by id or appends when the id is new.
 * Model, thinking level and tool fields only ride full snapshots.
 */
export interface SnapshotDelta {
  rev: number;
  /** New or changed messages, keyed by id. */
  upserts: NormalizedMessage[];
  /** Ids that left the transcript, applied before the upserts. */
  removed: string[];
  /** Present only when the field changed since the last emission; absent means unchanged. */
  runTimings?: RunTiming[];
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
}

/** A question an extension asked, mirrored from the worker protocol. */
export type ExtensionUIRequest = { taskId: string; requestId: string } & (
  | { method: "select"; title: string; options: string[] }
  | { method: "confirm"; title: string; message: string }
  | { method: "input"; title: string; placeholder?: string }
  | { method: "editor"; title: string; prefill?: string }
  | { method: "questions"; title: string; questions: AskQuestion[] }
);

export interface ExtensionNotice {
  message: string;
  level: "info" | "warning" | "error";
}

export type WorkerEvent =
  | { type: "ready" | "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot_delta"; taskId: string; delta: SnapshotDelta }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; startedAt?: number; state: TaskStatus }
  | { type: "activity"; taskId: string; event: string; detail?: Record<string, unknown> }
  | { type: "worker_error"; taskId?: string; message: string }
  | { type: "response"; taskId?: string; id: string; success: boolean; error?: string }
  | ({ type: "extension_ui_request" } & ExtensionUIRequest)
  | ({ type: "extension_notice"; taskId: string } & ExtensionNotice)
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] }
  | ({ type: "plan_state"; taskId: string } & PlanState)
  | ({ type: "todo_state"; taskId: string } & TodoState)
  /** From the host, once per chat per session: why file checkpoints are off for it. */
  | { type: "checkpoint_unavailable"; taskId: string; message: string };

export interface TaskRuntime {
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
  /** The most recent file restore, offered for undo until dismissed. */
  lastRestore?: { count: number; undo: CheckpointRef };
}

export interface GitChangeFile {
  path: string;
  status: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  binary: boolean;
  truncated: boolean;
  diff: string;
}

export interface GitChanges {
  isGit: boolean;
  root: string | null;
  branch: string | null;
  files: GitChangeFile[];
}

export interface SaveProviderInput {
  id?: string;
  name: string;
  baseUrl: string;
  apiFormat: ApiFormat;
  models: ModelRecord[];
  apiKey?: string;
}
