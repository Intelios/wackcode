export type ApiFormat = "openai-completions" | "openai-responses";
export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";
export type TaskStatus = "idle" | "running" | "stopping" | "interrupted" | "error";

export interface ModelRecord {
  id: string;
  name: string;
  contextWindow: number | null;
  maxTokens: number | null;
  reasoning: boolean;
  thinkingLevels: ThinkingLevel[];
  thinkingLevelMap: Partial<Record<ThinkingLevel, string | null>>;
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
  createdAt: string;
}

export interface TaskRecord {
  id: string;
  projectId: string;
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
  archived: boolean;
  lastError: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface AppData {
  version: number;
  providers: ProviderRecord[];
  projects: ProjectRecord[];
  tasks: TaskRecord[];
}

export interface BootstrapPayload {
  data: AppData;
  appDataPath: string;
}

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
}

export type WorkerEvent =
  | { type: "ready" | "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; state: TaskStatus }
  | { type: "activity"; taskId: string; event: string; detail?: Record<string, unknown> }
  | { type: "worker_error"; taskId?: string; message: string }
  | { type: "response"; taskId?: string; id: string; success: boolean; error?: string };

export interface TaskRuntime {
  snapshot?: SessionSnapshot;
  partial?: NormalizedMessage;
  activity?: string;
  error?: string;
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
