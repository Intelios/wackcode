export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];
export type ApiFormat = "openai-completions" | "openai-responses";

/** The agent's working mode. "plan" is the read-only, plan-first mode. */
export type TaskMode = "build" | "plan";

/** Plan mode state published by the built-in plan-mode extension. */
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
  /** Short imperative subject line. */
  subject: string;
  /** Long-form detail. */
  description?: string;
  /** Present-continuous label shown while status is in_progress. */
  activeForm?: string;
  status: TodoStatus;
  /** Ids of tasks that must complete before this one can start. */
  blockedBy?: number[];
}

/** Todo list state published by the built-in todo extension. */
export interface TodoState {
  /** Non-deleted filtering is the renderer's concern; the list carries tombstones too. */
  tasks: TodoTask[];
}

/** One option in an ask_user_question question. */
export interface AskQuestionOption {
  /** Short choice label (1-5 words). */
  label: string;
  /** One short sentence explaining the impact/tradeoff of this choice. */
  description: string;
}

/** A structured question the ask_user_question tool puts to the user. */
export interface AskQuestion {
  /** Stable snake_case identifier for mapping answers. */
  id: string;
  /** Short tab label (12 or fewer characters). */
  header: string;
  /** Single-sentence question shown to the user. */
  question: string;
  /** Allow selecting several options instead of exactly one. */
  multiSelect?: boolean;
  options: AskQuestionOption[];
}

/** One answered question, returned to the tool that asked it. */
export interface QuestionAnswer {
  /** Matches AskQuestion.id. */
  questionId: string;
  /** Labels of the chosen preset options (empty when the user wrote a custom answer). */
  selected: string[];
  /** Free-text answer when the user chose "Other". */
  custom?: string;
}

/**
 * An image attached to a prompt. Mirrors pi-ai's `ImageContent` exactly — the shape Pi's own RPC
 * `prompt.images` takes and the one it stores in session files — so it crosses every layer unchanged.
 */
export interface ImageContent {
  type: "image";
  /** Base64 without a `data:` prefix. */
  data: string;
  mimeType: string;
}

export interface WorkerModel {
  id: string;
  name: string;
  contextWindow: number;
  maxTokens: number;
  reasoning: boolean;
  thinkingLevels: ThinkingLevel[];
  thinkingLevelMap: Partial<Record<ThinkingLevel, string | null>>;
  /** Accepts image input; becomes Pi's `input: ["text", "image"]`. */
  vision?: boolean;
}

export interface WorkerProvider {
  id: string;
  name: string;
  kind: "custom" | "subscription";
  baseUrl: string;
  api: ApiFormat;
  models: WorkerModel[];
}

export interface InitCommand {
  id: string;
  type: "init";
  taskId: string;
  cwd: string;
  agentDir: string;
  sessionDir: string;
  sessionFile?: string;
  provider: WorkerProvider;
  modelId: string;
  apiKey?: string;
  authPath?: string;
  thinkingLevel: ThinkingLevel;
  /**
   * Tools the user has switched off, built-in or extension-contributed.
   * A denylist, so a tool a newly installed package adds is on by default.
   */
  disabledTools?: string[];
  /**
   * The mode the task record last had. The plan-mode extension reconciles this with whatever
   * the restored session says; the record wins on a mismatch because it carries the user's
   * most recent explicit choice (e.g. toggled while no worker was running).
   */
  mode?: TaskMode;
  /**
   * Absolute paths of the resources this session may load, already resolved and filtered by
   * the host. Auto-discovery stays off, so these are the only resources that can execute:
   * nothing from a project's own `.pi/` is ever loaded.
   */
  resources?: WorkerResources;
  /**
   * Fork: when the task has no session file of its own yet, build it from this chat's session
   * as the path from the root to `entryId` (default: that session's leaf), like Pi's `/fork`.
   */
  forkFrom?: { sessionFile: string; entryId?: string };
}

export interface WorkerResources {
  extensions: string[];
  skills: string[];
  prompts: string[];
  themes: string[];
}

/**
 * A workspace checkpoint: a tree in the chat's private shadow repository, taken by the host
 * before a prompt or when a branch is left. Recorded in the session so it follows the tree.
 */
export interface CheckpointRef {
  /** Tree id in the shadow repository. */
  id: string;
  /** The workspace repository's HEAD commit when the snapshot was taken, if it had one. */
  head?: string;
}

/** Why the session tree moved. "rewind" is the only kind "Undo rewind" is offered for. */
export type NavigationKind = "rewind" | "switch" | "undo";

/** The `navigate` command's result, carried on its `response`. */
export interface NavigateResult {
  leafId: string | null;
  /** The text of the user message a rewind removed, for the composer. */
  editorText?: string;
  /** The files the branch now shown was left with, when a checkpoint recorded them. */
  files?: CheckpointRef;
}

export type WorkerCommand =
  | InitCommand
  | {
      id: string;
      type: "prompt";
      runId: string;
      startedAt?: number;
      message: string;
      mode?: TaskMode;
      images?: ImageContent[];
      /** Recorded just above the user message; `null` records that no snapshot was possible. */
      checkpoint?: CheckpointRef | null;
    }
  | {
      id: string;
      type: "resend";
      runId: string;
      startedAt?: number;
      /** The user message to send again as a new version. */
      entryId: string;
      /** Replacement text (edit); the original text when absent (retry). */
      message?: string;
      /** Indexes of the original message's images to leave out. */
      removeImages?: number[];
      checkpoint?: CheckpointRef | null;
      /** The files as the current branch is left. */
      leave?: CheckpointRef | null;
    }
  | {
      id: string;
      type: "navigate";
      /** "before": a user message, the conversation then ends just above it (rewind).
       *  "latest": any entry, the conversation then ends at the newest entry beneath it. */
      entryId: string;
      target: "before" | "latest";
      kind: NavigationKind;
      leave?: CheckpointRef | null;
    }
  | { id: string; type: "abort" }
  | { id: string; type: "snapshot" }
  | { id: string; type: "set_model"; modelId: string }
  | { id: string; type: "set_thinking"; level: ThinkingLevel }
  | { id: string; type: "set_mode"; mode: TaskMode }
  | { id: string; type: "set_tools"; disabledTools: string[] }
  | {
      id: string;
      type: "extension_ui_response";
      requestId: string;
      value?: string;
      confirmed?: boolean;
      cancelled?: true;
      answers?: QuestionAnswer[];
    }
  | { id: string; type: "shutdown" };

export interface NormalizedBlock {
  type: "text" | "thinking" | "tool-call" | "tool-result" | "image";
  text?: string;
  /** Image blocks: the original's type. The full image never leaves the worker in a snapshot. */
  mimeType?: string;
  /** Image blocks: stable for the life of the worker, so the UI can key and memoize on it. */
  imageId?: string;
  /** Image blocks: a small `data:` URL preview, absent until it has been generated. */
  thumbnail?: string;
  toolName?: string;
  toolCallId?: string;
  arguments?: unknown;
  isError?: boolean;
  details?: unknown;
}

/** The other versions of a user message: edits and retries sent from the same point. */
export interface MessageVersions {
  /** Zero-based position among the versions, oldest first. */
  index: number;
  total: number;
  previous?: string;
  next?: string;
  /** Stable across versions of the same message, so the UI can keep one element for all of them. */
  group: string;
}

/** Where a turn (one user message and everything answering it) ends, for retry and fork. */
export interface TurnInfo {
  userEntryId: string;
  /** The last entry of the turn: where a fork of this turn ends. */
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
  /** User messages with other versions. */
  versions?: MessageVersions;
  /** User messages: the files as they were just before this message was sent. */
  checkpoint?: CheckpointRef;
  /** The last assistant message of each turn. */
  turn?: TurnInfo;
}

/** A completed prompt duration attached to the user message that started it. */
export interface RunTiming {
  userMessageId: string;
  durationMs: number;
}

/** Where a tool came from, so the UI can group and attribute it. */
export interface ToolSource {
  /** "wackcode" tools ship inside the app itself (built-in extensions) and can't be switched off. */
  kind: "builtin" | "package" | "wackcode";
  /** Package source string (e.g. "npm:pi-web-access") when kind is "package". */
  packageId?: string;
  /** Absolute path of the file that registered the tool. */
  path?: string;
}

export interface ToolCatalogEntry {
  name: string;
  description: string;
  source: ToolSource;
  /** False when a required external binary is missing; the tool is not offered to the model. */
  available: boolean;
  /** Why the tool is unavailable, shown in Settings. */
  unavailableReason?: string;
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
  tree: {
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

/** An extension asking the user something. Mirrors Pi's own RPC dialog surface. */
export type ExtensionUIRequest =
  | { method: "select"; title: string; options: string[] }
  | { method: "confirm"; title: string; message: string }
  | { method: "input"; title: string; placeholder?: string }
  | { method: "editor"; title: string; prefill?: string }
  | { method: "questions"; title: string; questions: AskQuestion[] };

export type WorkerOutput =
  | { type: "response"; taskId?: string; id: string; success: true; result?: unknown }
  | { type: "response"; taskId?: string; id: string; success: false; error: string }
  | { type: "ready"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot_delta"; taskId: string; delta: SnapshotDelta }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; startedAt?: number; state: "running" | "idle" | "stopping" | "interrupted" }
  | { type: "activity"; taskId: string; event: string; detail?: unknown }
  | { type: "worker_error"; taskId?: string; message: string }
  | ({ type: "extension_ui_request"; taskId: string; requestId: string } & ExtensionUIRequest)
  | { type: "extension_notice"; taskId: string; message: string; level: "info" | "warning" | "error" }
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] }
  | { type: "plan_state"; taskId: string } & PlanState
  | ({ type: "todo_state"; taskId: string } & TodoState);
