export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

export type ThinkingLevel = (typeof THINKING_LEVELS)[number];
export type ApiFormat = "openai-completions" | "openai-responses";

/**
 * The agent's working mode. "plan" is the read-only, plan-first mode; "ultraplan" is the same
 * read-only mode with an exhaustive, one-question-at-a-time interview before the plan.
 */
export type TaskMode = "build" | "plan" | "ultraplan";

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

/**
 * Goal-loop state published by the built-in goal extension (`goal_state`). A goal is a
 * harness-level loop: after each working round a separate no-tools verifier call judges the
 * objective and the runtime injects the next turn itself — the working model never gets to
 * declare victory.
 */
export interface GoalState {
  /** The objective the loop is verifying against. */
  objective: string;
  /**
   * active: a working round is running (or a continuation is queued).
   * verifying: the completion verifier is judging the last round.
   * paused: waiting on the user (Stop, /goal pause, reload, or no-progress auto-pause).
   * complete: the verifier passed (or failed open), or stopped: the loop gave up.
   */
  phase: "active" | "verifying" | "paused" | "complete" | "stopped";
  /** Working rounds verified so far. */
  iteration: number;
  /** Hard cap on iterations; the only fixed stop. */
  maxIterations: number;
  /** Consecutive rounds with no detected progress. */
  noProgress: number;
  /** The verifier's latest reason and queued next action. */
  lastReason?: string;
  lastNextAction?: string;
  /** Why the loop paused or stopped ("user", "no-progress", "max-iterations", "no-next-action", "reload"). */
  note?: string;
}

/** What `goal_control` asks the worker to do with the chat's goal. */
export type GoalAction = "set" | "pause" | "resume" | "clear";

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

/** Credentials for the one title request, supplied only on the first prompt over stdin. */
export interface AutoTitleRequest {
  attemptId: string;
  provider: WorkerProvider;
  modelId: string;
  apiKey?: string;
  authPath?: string;
}

/** When the main agent should reach for sub-agents. Only changes the tool's guidance text. */
export type SubagentTrigger = "on_request" | "auto";

export interface SubagentModelChoice {
  providerId: string;
  modelId: string;
  thinkingLevel: ThinkingLevel;
}

/** One agent the `subagent` tool can launch, resolved by the host from Settings. */
export interface SubagentSpec {
  /** The name the model calls it by, e.g. "scout". */
  name: string;
  description: string;
  /** Appended to Pi's system prompt for the child. */
  prompt: string;
  /** Tools the agent may use (Pi's own, plus `web_fetch`), before availability and the user's denylist apply. */
  tools: string[];
  /** Read-only agents never get edit/write, their bash is limited, and they may run in Plan mode. */
  readOnly: boolean;
  /** A model of its own; absent means the chat's model and thinking level. */
  model?: SubagentModelChoice;
  /** Set when the chosen model can't be used (signed out, model removed); reported per call. */
  unavailable?: string;
}

/** A connection a sub-agent's model lives on, with the credential it needs. */
export interface SubagentProvider {
  provider: WorkerProvider;
  apiKey?: string;
  authPath?: string;
}

export interface SubagentRuntimeConfig {
  trigger: SubagentTrigger;
  /** Children one call runs at the same time, including those that can edit files. */
  maxConcurrency: number;
  /** Enabled agents only. */
  agents: SubagentSpec[];
  /** Connections referenced by agents' own models. Credentials travel only over stdin. */
  providers: SubagentProvider[];
}

export type SubagentStatus = "queued" | "running" | "done" | "failed" | "aborted";

/** One tool call a child made, summarized for the card. */
export interface SubagentActivity {
  tool: string;
  subject: string;
}

export interface SubagentUsage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  turns: number;
}

export interface SubagentResult {
  agent: string;
  task: string;
  readOnly: boolean;
  /** "Provider · Model", once the child has started. */
  model?: string;
  status: SubagentStatus;
  /** The most recent tool calls, oldest first. */
  activity: SubagentActivity[];
  /** The child's final answer, capped for the card (the model receives more). */
  output?: string;
  outputTruncated?: boolean;
  error?: string;
  usage: SubagentUsage;
  startedAt?: number;
  endedAt?: number;
  /**
   * What the child did, for the side panel: saved with the final tool result so it survives a
   * reload, fork or rewind, but stripped from snapshots and never sent with live card updates —
   * the panel fetches it with `watch_subagent`. Absent on calls made before transcripts were kept.
   */
  transcript?: SubagentTranscript;
}

/** A child's messages as the panel renders them: normalized, redacted and capped. */
export interface SubagentTranscript {
  v: 1;
  messages: NormalizedMessage[];
  /** Older tool output was trimmed to keep the chat's session file small. */
  truncated?: boolean;
}

/** The `subagent` tool's result details: what the transcript card renders, live and after a reload. */
export interface SubagentDetails {
  v: 1;
  mode: "single" | "parallel";
  results: SubagentResult[];
}

/** One child of one `subagent` call: the tool call's id and the child's position in it. */
export interface SubagentTarget {
  toolCallId: string;
  index: number;
}

/**
 * One frame of the watched child's transcript (`watch_subagent`). A `reset` frame carries the
 * whole transcript and replaces whatever the host holds; the frames after it apply on top of the
 * one carrying `rev - 1`, like `SnapshotDelta`: removed ids first, then each upsert replaces its
 * message by id or appends.
 */
export interface SubagentStreamFrame extends SubagentTarget {
  rev: number;
  reset?: true;
  /** Reset frames: the whole transcript. Other frames: new or changed messages. */
  upserts: NormalizedMessage[];
  removed: string[];
  /** The message the child is writing right now; null when it isn't writing one. */
  partial: NormalizedMessage | null;
  /** The child is still running in this worker. */
  live: boolean;
  truncated?: boolean;
  /** Nothing is known about this child yet: not started, or a call from before transcripts were kept. */
  missing?: boolean;
}

/**
 * User-customized built-in prompt texts from Settings, applied on top of the shipped defaults.
 * Like `set_tools` these live outside the worker fingerprint: they arrive in `init` and can be
 * replaced mid-session with `set_prompts`, and every consumer reads them per use.
 */
export interface PromptOverrides {
  /** Replaces Pi's default system-prompt persona; the tool/rules/context sections still follow. */
  systemPrompt?: string;
  /** Replaces the Plan-mode contract body (the marker line is always the app's own). */
  planPrompt?: string;
  /** Replaces the Ultra Plan contract body (the marker line is always the app's own). */
  ultraPlanPrompt?: string;
}

/** How WackCode reaches an MCP server. */
export type McpTransport = "stdio" | "http" | "sse";

/**
 * One MCP server as the worker receives it, in `init` and `set_mcp`: enabled servers only, with
 * the header and environment values the host resolved from `secrets.json`. Like every other
 * credential, those values travel only over stdin.
 */
export interface McpServerSpec {
  id: string;
  /** The user's name for the server, shown in tool descriptions and errors. */
  name: string;
  /** Lowercase `[a-z0-9_]`, unique across servers: the middle of the server's tool names. */
  slug: string;
  transport: McpTransport;
  /** Bounds connecting (start, handshake and tool list) and each tool call. */
  timeoutMs: number;
  /** stdio: the program to run, its arguments, and extra environment variables for it alone. */
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  /** http and sse: the endpoint and the headers sent with every request to it. */
  url?: string;
  headers?: Record<string, string>;
  /** The server's own tool names the user switched off. */
  disabledTools: string[];
}

/** One tool a server offers, as Settings lists it. */
export interface McpToolInfo {
  name: string;
  description: string;
  /** The server marks the tool read-only (`readOnlyHint`), so Plan mode lets it through. */
  readOnly: boolean;
}

/** Settings' "Test connection": `mcp-probe.js` reads `{ server }` and prints one of these. */
export type McpProbeResult = { ok: true; tools: McpToolInfo[] } | { ok: false; error: string };

/** One user skill folder (Settings › Skills), as an absolute path the host resolved. */
export interface UserSkillRoot {
  path: string;
  /** The skill's source in the `/` menu, e.g. "Your skills" or "Claude Code". */
  label: string;
}

/**
 * The user's own skill folders: `~/.agents/skills` first, then each other folder the user
 * switched on, in the order that decides a name clash. A project's own skill folders are never
 * among them. Like the prompt overrides these stay out of the worker fingerprint: they arrive in
 * `init` and are replaced live with `set_skills`.
 */
export interface UserSkillsPayload {
  roots: UserSkillRoot[];
  /** Skill files the user switched off, spelled exactly as a scan reports their `filePath`. */
  disabled: string[];
}

/** Settings' skill list: `skills-scan.js` reads one of these and prints a `SkillScanResult`. */
export interface SkillScanRequest {
  /** Every folder Settings lists, switched on or not, in the worker's priority order. */
  folders: Array<UserSkillRoot & { id: string; enabled: boolean }>;
  /** Trusted packages' skill resources, in the order the worker's resource loader takes them. */
  packages: Array<{ source: string; label: string; resources: Array<{ path: string; name: string; enabled: boolean }> }>;
  disabled: string[];
}

export interface ScannedSkill {
  name: string;
  description: string;
  filePath: string;
  baseDir: string;
  /** `disable-model-invocation: true`: only `/skill:name` uses it. */
  manual: boolean;
  /** The SKILL.md's optional `argument-hint`, shown next to the name while typing. */
  argumentHint?: string;
  /** Package skills: the package resource the skill came from, which its switch toggles. */
  resourceName?: string;
  /** Switched on, but a skill of the same name wins: that skill's folder or package label. */
  shadowedBy?: string;
}

export interface ScannedDiagnostic {
  type: "warning" | "error" | "collision";
  message: string;
  path?: string;
}

/** One folder's (by id) or one package's (by source) skills. */
export interface ScannedGroup {
  id: string;
  skills: ScannedSkill[];
  diagnostics: ScannedDiagnostic[];
}

export type SkillScanResult =
  | { ok: true; folders: ScannedGroup[]; packages: ScannedGroup[] }
  | { ok: false; error: string };

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
  /** Sub-agents, when the user has turned them on. Absent or null: the `subagent` tool stays off. */
  subagents?: SubagentRuntimeConfig | null;
  /** Custom built-in prompt texts from Settings; absent means every prompt stays at its default. */
  prompts?: PromptOverrides;
  /** Enabled MCP servers. Nothing connects until the chat's first run. */
  mcp?: McpServerSpec[];
  /** The user's own skill folders. Absent: none, only package skills load. */
  skills?: UserSkillsPayload;
  /** The user's own commands and the switched-off keys. Live via `set_commands`. */
  commands?: UserCommandsPayload;
}

export interface WorkerResources {
  extensions: string[];
  skills: string[];
  prompts: string[];
  themes: string[];
}

export interface SlashCommand {
  /** Its `commandKey` (`slash.ts`): stable across sessions, and the same key Settings' switches use. */
  id: string;
  name: string;
  description?: string;
  /** "custom" is a file in the user's own commands folder (Settings › Commands). */
  source: "app" | "extension" | "prompt" | "custom" | "skill";
  sourceLabel: string;
  /** A prompt template's `argument-hint`, shown next to its name. */
  argumentHint?: string;
}

/**
 * The user's own commands (Settings › Commands): the app-owned `commands/` folder, plus the
 * switched-off `commandKey`s covering every kind (`app:`, `extension:`, `prompt:`, `custom:`,
 * `skill:`). Live like skills: it arrives in `init`, is replaced with `set_commands`, and stays
 * out of the worker fingerprint so toggling never respawns a chat.
 */
export interface UserCommandsPayload {
  /** Absolute path of the folder holding the user's `<name>.md` prompt templates. */
  dir: string;
  disabled: string[];
}

/** Settings' command list: `commands-scan.js` reads one of these and prints a `CommandScanResult`. */
export interface CommandScanRequest {
  /** Working directory the future chat will use (or the user's home when there is no project). */
  cwd: string;
  /**
   * Trusted packages' extension and prompt resources, in Settings order. `enabled` mirrors the
   * package's own resource switches: a switched-off resource contributes nothing, like a chat.
   */
  packages: Array<{
    source: string;
    label: string;
    /** The package's install root, so load diagnostics can be attributed to it. */
    installedPath?: string;
    extensions: Array<{ path: string; enabled: boolean }>;
    skills: Array<{ path: string; enabled: boolean }>;
    prompts: Array<{ path: string; enabled: boolean }>;
  }>;
  /** Enabled user skill folders, in the same priority order as a chat. */
  skillRoots: UserSkillRoot[];
  /** Skill files switched off in Settings › Skills (separate from disabled slash commands). */
  skillDisabled: string[];
  /** The app-owned folder of the user's commands (`<app data>/commands`). */
  commandsDir: string;
  /** Switched-off `commandKey`s; they resolve names out of the running set exactly as a chat does. */
  disabled: string[];
}

/** One command as Settings rows it. */
export interface ScannedCommand {
  /** Its `commandKey` — also the `SlashCommand.id` a chat reports, so the two always agree. */
  key: string;
  /** What `/` offers: the resolved name, or the command's own name while it is switched off. */
  name: string;
  /** The name a clash renamed (`<source>:<name>`), when it was. */
  rawName?: string;
  description?: string;
  argumentHint?: string;
  kind: "extension" | "prompt" | "custom";
  /** The template file for prompt/custom, the extension file for extension. */
  filePath?: string;
  enabled: boolean;
}

/** One package's commands (id = its source). */
export interface ScannedCommandGroup {
  id: string;
  commands: ScannedCommand[];
  diagnostics: ScannedDiagnostic[];
}

export type CommandScanResult =
  | { ok: true; custom: ScannedCommand[]; packages: ScannedCommandGroup[]; catalog: SlashCommand[] }
  | { ok: false; error: string };

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
  | { id: string; type: "list_commands" }
  | { id: string; type: "execute_command"; commandId: string; args: string; runId: string; startedAt?: number; checkpoint?: CheckpointRef | null; images?: ImageContent[] }
  | { id: string; type: "init_agents"; runId: string; startedAt?: number; checkpoint?: CheckpointRef | null }
  | { id: string; type: "compact"; runId: string; startedAt?: number; instructions?: string }
  | {
      id: string;
      type: "prompt";
      autoTitle?: AutoTitleRequest | null;
      runId: string;
      startedAt?: number;
      message: string;
      literal?: boolean;
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
  | {
      id: string;
      type: "queue_message";
      /** "steer" delivers at the run's next boundary; "follow_up" waits for the run to finish. */
      behavior: "steer" | "follow_up";
      message: string;
      /** Sent raw (no command/skill/template expansion) — the composer's "Send as message". */
      literal?: boolean;
      images?: ImageContent[];
    }
  | { id: string; type: "dequeue" }
  | {
      id: string;
      type: "goal_control";
      action: GoalAction;
      /** "set": the objective to verify against. */
      objective?: string;
      /** "set"/"resume" start a run: they carry the same run bookkeeping as a prompt. */
      runId?: string;
      startedAt?: number;
      checkpoint?: CheckpointRef | null;
    }
  | { id: string; type: "snapshot" }
  /**
   * Stream one sub-agent's transcript to the host (`subagent_stream`), starting with a reset
   * frame; null stops. One child at a time: a new target replaces the last. Bypasses the command
   * queue, so it answers while the call that runs the child is still going.
   */
  | { id: string; type: "watch_subagent"; target: SubagentTarget | null }
  | { id: string; type: "generate_commit_message"; diff: string; truncated: boolean }
  | { id: string; type: "set_model"; modelId: string }
  | { id: string; type: "set_thinking"; level: ThinkingLevel }
  | { id: string; type: "set_mode"; mode: TaskMode }
  | { id: string; type: "set_tools"; disabledTools: string[] }
  | { id: string; type: "set_subagents"; subagents: SubagentRuntimeConfig | null }
  | { id: string; type: "set_prompts"; prompts: PromptOverrides }
  | { id: string; type: "set_mcp"; servers: McpServerSpec[] }
  | { id: string; type: "set_skills"; skills: UserSkillsPayload }
  | { id: string; type: "set_commands"; commands: UserCommandsPayload }
  | { id: string; type: "browser_response"; requestId: string; success: true; result: unknown }
  | { id: string; type: "browser_response"; requestId: string; success: false; error: string }
  | {
      id: string;
      type: "extension_ui_response";
      requestId: string;
      value?: string;
      confirmed?: boolean;
      cancelled?: true;
      answers?: QuestionAnswer[];
      /** The user pressed "Write the plan now" on an Ultra Plan questionnaire. */
      wrapUp?: true;
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
  /** Thinking blocks: how long the model reasoned. Absent while it still is, and when never clocked. */
  durationMs?: number;
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
  /** User messages with other versions. */
  versions?: MessageVersions;
  /** User messages: the files as they were just before this message was sent. */
  checkpoint?: CheckpointRef;
  /** User messages generated by a command; the blocks remain the exact model-visible prompt. */
  commandPresentation?: CommandPresentation;
  /** The last assistant message of each turn. */
  turn?: TurnInfo;
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

/** Where a tool came from, so the UI can group and attribute it. */
export interface ToolSource {
  /** "wackcode" tools ship inside the app itself (built-in extensions) and can't be switched off.
   *  "mcp" tools come from the user's MCP servers; their switches are in Settings › MCP servers. */
  kind: "builtin" | "package" | "wackcode" | "mcp";
  /** Package source string (e.g. "npm:pi-web-access") when kind is "package". */
  packageId?: string;
  /** The MCP server's id when kind is "mcp". */
  serverId?: string;
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
  modelSwitches: ModelSwitch[];
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
  /** Present only when it changed; null clears the goal, absent leaves it unchanged. */
  goalState?: GoalState | null;
}

/** An extension asking the user something. Mirrors Pi's own RPC dialog surface. */
export type ExtensionUIRequest =
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
    };

export type WorkerOutput =
  | { type: "usage_record"; taskId: string; record: import("./usage.js").UsageRecord }
  | { type: "title_result"; taskId: string; attemptId: string; title?: string }
  | { type: "response"; taskId?: string; id: string; success: true; result?: unknown }
  | { type: "response"; taskId?: string; id: string; success: false; error: string }
  | { type: "ready"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot"; taskId: string; snapshot: SessionSnapshot }
  | { type: "snapshot_delta"; taskId: string; delta: SnapshotDelta }
  | { type: "partial"; taskId: string; message: NormalizedMessage }
  | { type: "run_state"; taskId: string; runId?: string; startedAt?: number; state: "running" | "idle" | "stopping" | "interrupted" }
  | { type: "run_finished"; taskId: string; runId: string; outcome: "completed" | "stopped" | "failed" }
  | { type: "queue_state"; taskId: string; steering: string[]; followUp: string[] }
  | { type: "activity"; taskId: string; event: string; detail?: unknown }
  | { type: "worker_error"; taskId?: string; message: string }
  | { type: "browser_request"; taskId: string; requestId: string; request: Record<string, unknown> }
  | { type: "browser_cancel"; taskId: string; requestId: string }
  | ({ type: "extension_ui_request"; taskId: string; requestId: string } & ExtensionUIRequest)
  | { type: "extension_ui_resolved"; taskId: string; requestId: string; cancelled: boolean }
  | { type: "extension_notice"; taskId: string; message: string; level: "info" | "warning" | "error" }
  | { type: "extensions_loaded"; taskId: string; loaded: string[]; errors: { path: string; error: string }[] }
  | { type: "plan_state"; taskId: string } & PlanState
  | ({ type: "todo_state"; taskId: string } & TodoState)
  /** The goal loop's state, or null when it was cleared. */
  | { type: "goal_state"; taskId: string; goal: GoalState | null }
  | ({ type: "subagent_stream"; taskId: string } & SubagentStreamFrame);
