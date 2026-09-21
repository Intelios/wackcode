export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];
export type ApiFormat = "openai-completions" | "openai-responses";

export interface WorkerModel {
  id: string;
  name: string;
  contextWindow: number;
  maxTokens: number;
  reasoning: boolean;
  thinkingLevels: ThinkingLevel[];
  thinkingLevelMap: Partial<Record<ThinkingLevel, string | null>>;
}

export interface WorkerProvider {
  id: string;
  name: string;
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
  apiKey: string;
  thinkingLevel: ThinkingLevel;
  /**
   * Tools the user has switched off, built-in or extension-contributed.
   * A denylist, so a tool a newly installed package adds is on by default.
   */
  disabledTools?: string[];
  /**
   * Absolute paths of the resources this session may load, already resolved and filtered by
   * the host. Auto-discovery stays off, so these are the only resources that can execute:
   * nothing from a project's own `.pi/` is ever loaded.
   */
  resources?: WorkerResources;
}

export interface WorkerResources {
  extensions: string[];
  skills: string[];
  prompts: string[];
  themes: string[];
}

export type WorkerCommand =
  | InitCommand
  | { id: string; type: "prompt"; runId: string; message: string }
  | { id: string; type: "abort" }
  | { id: string; type: "snapshot" }
  | { id: string; type: "set_model"; modelId: string }
  | { id: string; type: "set_thinking"; level: ThinkingLevel }
  | { id: string; type: "set_tools"; disabledTools: string[] }
  | { id: string; type: "extension_ui_response"; requestId: string; value?: string; confirmed?: boolean; cancelled?: true }
  | { id: string; type: "shutdown" };

export interface NormalizedBlock {
  type: "text" | "thinking" | "tool-call" | "tool-result";
  text?: string;
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

/** Where a tool came from, so the UI can group and attribute it. */
export interface ToolSource {
  kind: "builtin" | "package";
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
}

/** An extension asking the user something. Mirrors Pi's own RPC dialog surface. */
export type ExtensionUIRequest =
  | { method: "select"; title: string; options: string[] }
  | { method: "confirm"; title: string; message: string }
  | { method: "input"; title: string; placeholder?: string }
  | { method: "editor"; title: string; prefill?: string };

export type WorkerOutput =
  | { type: "response"; taskId?: string; id: string; success: true }
  | { type: "response"; taskId?: string; id: string; success: false; error: string }
  | { type: "ready"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; state: "running" | "idle" | "stopping" | "interrupted" }
  | { type: "activity"; taskId: string; event: string; detail?: unknown }
  | { type: "worker_error"; taskId?: string; message: string }
  | ({ type: "extension_ui_request"; taskId: string; requestId: string } & ExtensionUIRequest)
  | { type: "extension_notice"; taskId: string; message: string; level: "info" | "warning" | "error" }
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] };
