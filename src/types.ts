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

export interface ProviderRecord {
  id: string;
  name: string;
  baseUrl: string;
  apiFormat: ApiFormat;
  models: ModelRecord[];
  createdAt: string;
  updatedAt: string;
  hasApiKey: boolean;
}

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

export interface AppData {
  version: number;
  providers: ProviderRecord[];
  projects: ProjectRecord[];
  tasks: TaskRecord[];
  toolConfig: ToolConfig;
  toolCatalog: ToolCatalogEntry[];
  packages: PackageRecord[];
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

export interface NormalizedMessage {
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  timestamp?: number;
  blocks: NormalizedBlock[];
  stopReason?: string;
  errorMessage?: string;
}

export interface SessionSnapshot {
  sessionId: string;
  sessionFile?: string;
  messages: NormalizedMessage[];
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
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; state: TaskStatus }
  | { type: "activity"; taskId: string; event: string; detail?: Record<string, unknown> }
  | { type: "worker_error"; taskId?: string; message: string }
  | { type: "response"; taskId?: string; id: string; success: boolean; error?: string }
  | ({ type: "extension_ui_request" } & ExtensionUIRequest)
  | ({ type: "extension_notice"; taskId: string } & ExtensionNotice)
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] }
  | ({ type: "plan_state"; taskId: string } & PlanState)
  | ({ type: "todo_state"; taskId: string } & TodoState);

export interface TaskRuntime {
  snapshot?: SessionSnapshot;
  partial?: NormalizedMessage;
  activity?: string;
  /** Accumulated text from in-flight tools, keyed by Pi's tool call id. */
  liveToolText?: Record<string, string>;
  error?: string;
  /** Extension output and load failures. Informational only — never blocks a chat. */
  notices?: ExtensionNotice[];
  /** Latest Plan mode state from the worker; falls back to `TaskRecord.mode` before `ready`. */
  planState?: PlanState;
  /** Latest todo list from the worker's built-in todo extension. */
  todoState?: TodoState;
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
