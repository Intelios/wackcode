import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { createBuiltinExtensions } from "./builtin/index.js";
import type { BuiltinHost } from "./builtin/host.js";
import { JsonLineDecoder } from "./framing.js";
import {
  THINKING_LEVELS,
  type ImageContent,
  type InitCommand,
  type NormalizedBlock,
  type NormalizedMessage,
  type QuestionAnswer,
  type SessionSnapshot,
  type ThinkingLevel,
  type ExtensionUIRequest,
  type ToolCatalogEntry,
  type WorkerCommand,
  type WorkerModel,
  type WorkerOutput
} from "./protocol.js";

process.env.PI_TELEMETRY = "0";
process.env.PI_SKIP_VERSION_CHECK = "1";
process.env.PI_OFFLINE = "1";
process.env.AI_AGENT = "pi";
process.env.PI_CODING_AGENT = "true";

type PiModule = typeof import("@earendil-works/pi-coding-agent");
type AgentSession = Awaited<ReturnType<PiModule["createAgentSession"]>>["session"];
type ModelRuntime = Awaited<ReturnType<PiModule["ModelRuntime"]["create"]>>;

let taskId: string | undefined;
let session: AgentSession | undefined;
let piModule: PiModule | undefined;
let modelRuntime: ModelRuntime | undefined;
let activeRunId: string | undefined;
let activeCredential: string | undefined;
let stopRequested = false;
let disabledTools = new Set<string>();
const pendingDialogs = new Map<string, (response: DialogResponse) => void>();

interface DialogResponse {
  value?: string;
  confirmed?: boolean;
  cancelled?: true;
  answers?: QuestionAnswer[];
}

/**
 * Built-in extensions reach the desktop through this bridge. Declared at module scope because
 * the factories are handed to the resource loader during `initialize`; the closures only use
 * `taskId`/`send` once commands run, so the ordering is safe.
 */
const builtinHost: BuiltinHost = {
  askQuestions: (questions) =>
    askHost<QuestionAnswer[] | undefined>(
      { method: "questions", title: "Questions", questions },
      (response) => response.answers,
      undefined
    ),
  publishPlanState: (state) => {
    if (taskId) send({ type: "plan_state", taskId, ...state });
  },
  publishTodoState: (state) => {
    if (taskId) send({ type: "todo_state", taskId, tasks: state.tasks });
  }
};
const builtins = createBuiltinExtensions(builtinHost);

// Pi registers a `powershell` base tool on every platform. WackCode is macOS-only, so keep it
// out of the registry entirely rather than shipping a dead tool the user has to switch off.
const UNSUPPORTED_TOOLS = ["powershell"];

// grep and find shell out to these. Pi normally downloads them on demand, but the worker runs
// with PI_OFFLINE=1, so a missing binary stays missing. Report the tool as unavailable instead
// of letting the model call something that errors mid-task.
const TOOL_BINARIES: Record<string, string> = { grep: "rg", find: "fd" };
const binaryCache = new Map<string, boolean>();

// Mirrors Pi's own resolution order (utils/tools-manager.ts): its bin directory first,
// then whatever is on PATH.
function hasBinary(binary: string): boolean {
  const cached = binaryCache.get(binary);
  if (cached !== undefined) return cached;
  const piBin = join(process.env.PI_CODING_AGENT_DIR ?? join(homedir(), ".pi", "agent"), "bin", binary);
  let found = existsSync(piBin);
  if (!found) {
    const probe = spawnSync(binary, ["--version"], { stdio: "ignore" });
    found = !probe.error;
  }
  binaryCache.set(binary, found);
  return found;
}
let commandQueue = Promise.resolve();
let snapshotTimer: NodeJS.Timeout | undefined;
let partialTimer: NodeJS.Timeout | undefined;
let pendingPartial: unknown;

function send(output: WorkerOutput): void {
  process.stdout.write(`${JSON.stringify(output)}\n`);
}

function response(id: string, success: boolean, error?: string): void {
  if (success) send({ type: "response", taskId, id, success: true });
  else send({ type: "response", taskId, id, success: false, error: error ?? "Unknown worker error" });
}

function textFromContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((item) => {
      if (typeof item === "string") return item;
      if (!item || typeof item !== "object") return "";
      const block = item as Record<string, unknown>;
      return typeof block.text === "string" ? block.text : typeof block.content === "string" ? block.content : "";
    })
    .join("");
}

// Pi sends accumulated progress on every update. Keep activity frames small even when a tool
// produces a long stream; the completed result still comes from the session snapshot.
function toolUpdateText(partialResult: unknown): string {
  if (!partialResult || typeof partialResult !== "object") return "";
  const content = (partialResult as Record<string, unknown>).content;
  const text = typeof content === "string" ? content : Array.isArray(content)
    ? content.map((block) => block?.type === "text" && typeof block.text === "string" ? block.text : "").join("")
    : "";
  return text.slice(-64 * 1024);
}

// Snapshots carry a small preview of each image, never the original: a snapshot is re-sent on
// every message boundary, and the originals can run to megabytes each.
const THUMBNAIL_OPTIONS = { maxWidth: 512, maxHeight: 512, maxBytes: 128 * 1024 };

interface ImagePreview {
  id: string;
  url?: string;
}

// Keyed by the image block itself. Pi keeps message objects stable across `session.messages`
// reads (the array is copied, its entries are not), so each image is previewed once per worker.
const imagePreviews = new WeakMap<object, ImagePreview>();
let nextImageId = 0;
// One preview at a time: each resize spawns a thread, and a restored session may hold dozens.
let previewQueue = Promise.resolve();

function imageBlock(block: Record<string, unknown>): NormalizedBlock {
  const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image/png";
  let preview = imagePreviews.get(block);
  if (!preview) {
    const created: ImagePreview = { id: `image-${++nextImageId}` };
    preview = created;
    imagePreviews.set(block, created);
    if (typeof block.data === "string" && block.data) {
      const data = block.data;
      previewQueue = previewQueue
        .then(async () => {
          if (!piModule) return;
          const resized = await piModule.resizeImage(Buffer.from(data, "base64"), mimeType, THUMBNAIL_OPTIONS);
          if (!resized) return;
          created.url = `data:${resized.mimeType};base64,${resized.data}`;
          scheduleSnapshot();
        })
        .catch(() => undefined);
    }
  }
  return { type: "image", mimeType, imageId: preview.id, thumbnail: preview.url };
}

function normalizeBlocks(content: unknown, role: string): NormalizedBlock[] {
  if (typeof content === "string") {
    return [{ type: role === "toolResult" ? "tool-result" : "text", text: content }];
  }
  if (!Array.isArray(content)) return [];

  return content.flatMap((item): NormalizedBlock[] => {
    if (typeof item === "string") return [{ type: "text", text: item }];
    if (!item || typeof item !== "object") return [];
    const block = item as Record<string, unknown>;
    if (block.type === "text") return [{ type: "text", text: String(block.text ?? "") }];
    // A tool result's images (e.g. `read` on a PNG) still reach the model; the transcript shows
    // only its text. Left in, each would become an empty result sharing the call's id and
    // overwrite the real one.
    if (block.type === "image") return role === "toolResult" ? [] : [imageBlock(block)];
    if (block.type === "thinking") return [{ type: "thinking", text: String(block.thinking ?? block.text ?? "") }];
    if (block.type === "toolCall") {
      return [{
        type: "tool-call",
        toolName: String(block.name ?? "tool"),
        toolCallId: String(block.id ?? ""),
        arguments: block.arguments
      }];
    }
    return [{ type: role === "toolResult" ? "tool-result" : "text", text: textFromContent([block]) }];
  });
}

function normalizeMessage(message: unknown, index: number): NormalizedMessage | undefined {
  if (!message || typeof message !== "object") return undefined;
  const raw = message as Record<string, unknown>;
  const rawRole = String(raw.role ?? "system");
  const role = rawRole === "toolResult" ? "tool" : rawRole;
  if (role !== "user" && role !== "assistant" && role !== "tool" && role !== "system") return undefined;
  const blocks = normalizeBlocks(raw.content, rawRole);
  if (rawRole === "toolResult") {
    // An image-only result still has to mark its call as finished.
    if (blocks.length === 0) blocks.push({ type: "tool-result", text: "" });
    for (const block of blocks) {
      block.type = "tool-result";
      block.toolName = typeof raw.toolName === "string" ? raw.toolName : undefined;
      block.toolCallId = typeof raw.toolCallId === "string" ? raw.toolCallId : undefined;
      block.isError = raw.isError === true;
      if (raw.details !== undefined) block.details = raw.details;
    }
  }
  const timestamp = typeof raw.timestamp === "number" ? raw.timestamp : undefined;
  return {
    id: `${role}-${timestamp ?? "na"}-${index}`,
    role,
    timestamp,
    blocks,
    stopReason: typeof raw.stopReason === "string" ? raw.stopReason : undefined,
    errorMessage: typeof raw.errorMessage === "string" ? raw.errorMessage : undefined
  };
}

// Per-category token estimates. `system` is the remainder between the provider's
// own context count and estimated message tokens — it covers the system prompt,
// tool definitions, and any estimation error.
function contextBreakdown(stats: ReturnType<AgentSession["getSessionStats"]>): SessionSnapshot["stats"]["contextBreakdown"] {
  if (!session || !piModule || !stats.contextUsage || stats.contextUsage.tokens == null) return undefined;
  const roles = { user: 0, assistant: 0, tool: 0 };
  for (const message of session.messages) {
    const role = message.role === "toolResult" ? "tool" : message.role;
    if (role === "user" || role === "assistant" || role === "tool") {
      roles[role] += piModule.estimateTokens(message);
    }
  }
  const used = stats.contextUsage.tokens;
  let entries = [
    { id: "system" as const, tokens: Math.max(0, used - roles.user - roles.assistant - roles.tool) },
    { id: "user" as const, tokens: roles.user },
    { id: "assistant" as const, tokens: roles.assistant },
    { id: "tool" as const, tokens: roles.tool }
  ];
  const total = entries.reduce((sum, entry) => sum + entry.tokens, 0);
  if (total > used && total > 0) {
    const factor = used / total;
    entries = entries.map((entry) => ({ ...entry, tokens: Math.round(entry.tokens * factor) }));
  }
  const { input, cacheRead } = stats.tokens;
  return {
    entries,
    cacheHitRate: input + cacheRead > 0 ? cacheRead / (input + cacheRead) : null
  };
}

// Pi attributes its own tools with a synthetic source of "builtin" (or "sdk" for tools
// registered through the SDK). Anything else came from an extension file.
//
// Note: sourceInfo.source is NOT the package name for extensions loaded through
// additionalExtensionPaths — Pi labels those "cli"/"temporary". Only the file path is
// meaningful, so the host maps path -> package; `packageId` is deliberately left unset here.
function toolCatalog(): ToolCatalogEntry[] {
  if (!session) return [];
  return session.getAllTools().map((tool) => {
    const info = tool.sourceInfo as { source?: string; path?: string } | undefined;
    const origin = info?.source;
    const builtin = origin === "builtin" || origin === "sdk";
    // Inline factories are WackCode's own built-in extensions, e.g. ask_user_question,
    // plan_mode_complete, and todo. They are neither Pi builtins nor user-installed packages.
    const wackcode = origin === "inline";
    const binary = TOOL_BINARIES[tool.name];
    const available = !binary || hasBinary(binary);
    return {
      name: tool.name,
      description: tool.description ?? "",
      source: builtin
        ? { kind: "builtin" as const }
        : wackcode
          ? { kind: "wackcode" as const }
          : { kind: "package" as const, path: info?.path },
      available,
      unavailableReason: available ? undefined : `Requires ${binary}, which is not installed`
    };
  });
}

// Every tool in the registry is active unless the user switched it off. A denylist keeps
// tools contributed by a newly installed package on by default. Built-in extension tools are
// exempt: the denylist is never offered for them and a disabled plan_mode_complete would
// silently break Plan mode.
function applyDisabledTools(): void {
  if (!session) return;
  session.setActiveToolsByName(
    toolCatalog()
      .filter((tool) => tool.available && (!disabledTools.has(tool.name) || tool.source.kind === "wackcode"))
      .map((tool) => tool.name)
  );
}

function getSnapshot(): SessionSnapshot {
  if (!session) throw new Error("Worker is not initialized");
  const stats = session.getSessionStats();
  const model = session.model;
  return {
    sessionId: session.sessionId,
    sessionFile: session.sessionFile,
    messages: session.messages.map(normalizeMessage).filter((message): message is NormalizedMessage => Boolean(message)),
    stats: {
      tokens: stats.tokens,
      cost: stats.cost,
      contextUsage: stats.contextUsage,
      contextBreakdown: contextBreakdown(stats)
    },
    thinkingLevel: session.thinkingLevel as ThinkingLevel,
    availableThinkingLevels: session.getAvailableThinkingLevels() as ThinkingLevel[],
    model: model ? { provider: model.provider, id: model.id, name: model.name } : undefined,
    tools: toolCatalog(),
    activeTools: session.getActiveToolNames(),
    planState: builtins.planMode.getState(),
    todoState: builtins.todo.getState()
  };
}

function emitSnapshot(): void {
  if (!taskId || !session) return;
  send({ type: "snapshot", taskId, snapshot: getSnapshot() });
}

function scheduleSnapshot(): void {
  if (snapshotTimer) return;
  snapshotTimer = setTimeout(() => {
    snapshotTimer = undefined;
    emitSnapshot();
  }, 32);
}

// Streaming text: forward the in-flight assistant message at ~60 fps instead of
// reserializing the whole session. Full snapshots remain authoritative at message_end.
function schedulePartial(message: unknown): void {
  pendingPartial = message;
  if (partialTimer) return;
  partialTimer = setTimeout(() => {
    partialTimer = undefined;
    flushPartial();
  }, 16);
}

function flushPartial(): void {
  if (partialTimer) {
    clearTimeout(partialTimer);
    partialTimer = undefined;
  }
  if (!taskId || pendingPartial === undefined) return;
  const message = normalizeMessage(pendingPartial, 0);
  pendingPartial = undefined;
  if (message) send({ type: "partial", taskId, message });
}

function dropPartial(): void {
  if (partialTimer) clearTimeout(partialTimer);
  partialTimer = undefined;
  pendingPartial = undefined;
}

/**
 * Ask the host a question on an extension's behalf.
 *
 * Resolving rather than rejecting on abort is deliberate and copied from Pi's RPC mode: an
 * extension awaiting a dialog inside a tool call must never be left hanging when the user stops
 * the run, so a cancelled dialog resolves to the caller's stated default.
 */
function askHost<T>(request: ExtensionUIRequest, onResponse: (response: DialogResponse) => T, fallback: T): Promise<T> {
  if (!taskId) return Promise.resolve(fallback);
  const requestId = crypto.randomUUID();
  return new Promise<T>((resolve) => {
    pendingDialogs.set(requestId, (response) => {
      pendingDialogs.delete(requestId);
      resolve(response.cancelled ? fallback : onResponse(response));
    });
    send({ type: "extension_ui_request", taskId: taskId as string, requestId, ...request });
  });
}

function cancelPendingDialogs(): void {
  for (const resolve of [...pendingDialogs.values()]) resolve({ cancelled: true });
  pendingDialogs.clear();
}

function notice(message: string, level: "info" | "warning" | "error" = "info"): void {
  if (taskId) send({ type: "extension_notice", taskId, message, level });
}

/**
 * The subset of ExtensionUIContext that makes sense without a terminal. Dialogs are bridged to
 * the desktop UI; ambient TUI affordances (status text, widgets, custom components, themes) are
 * accepted and discarded, exactly as Pi's own RPC mode does for the ones it cannot render.
 */
function createExtensionUIContext(): Record<string, unknown> {
  const ignore = () => undefined;
  return {
    select: (title: string, options: string[]) =>
      askHost<string | undefined>({ method: "select", title, options }, (response) => response.value, undefined),
    confirm: (title: string, message: string) =>
      askHost<boolean>({ method: "confirm", title, message }, (response) => response.confirmed === true, false),
    input: (title: string, placeholder?: string) =>
      askHost<string | undefined>({ method: "input", title, placeholder }, (response) => response.value, undefined),
    editor: (title: string, prefill?: string) =>
      askHost<string | undefined>({ method: "editor", title, prefill }, (response) => response.value, undefined),
    notify: (message: string, type?: "info" | "warning" | "error") => notice(message, type ?? "info"),
    onTerminalInput: () => ignore,
    setStatus: ignore,
    setWorkingMessage: ignore,
    setWorkingVisible: ignore,
    setWorkingIndicator: ignore,
    setHiddenThinkingLabel: ignore,
    setWidget: ignore,
    setFooter: ignore,
    setHeader: ignore,
    setTitle: ignore,
    custom: () => Promise.resolve(undefined),
    pasteToEditor: ignore,
    setEditorText: ignore,
    getEditorText: () => "",
    addAutocompleteProvider: ignore,
    setEditorComponent: ignore,
    getEditorComponent: () => undefined,
    theme: undefined,
    getAllThemes: () => [],
    getTheme: () => undefined,
    setTheme: () => ({ success: false, error: "Themes are not available in WackCode" }),
    getToolsExpanded: () => false,
    setToolsExpanded: ignore
  };
}

function modelDefinition(model: WorkerModel): Record<string, unknown> {
  const thinkingLevelMap = Object.fromEntries(
    THINKING_LEVELS.map((level) => {
      if (!model.thinkingLevels.includes(level)) return [level, null];
      const mapped = model.thinkingLevelMap[level];
      return [level, mapped === undefined ? (level === "off" ? null : level) : mapped];
    })
  );
  return {
    id: model.id,
    name: model.name,
    reasoning: model.reasoning,
    // Pi's own capability flag. Without "image", Pi replaces images with an "image omitted"
    // placeholder before the request is built, and `read` stops returning image content.
    input: model.vision ? ["text", "image"] : ["text"],
    contextWindow: model.contextWindow,
    maxTokens: model.maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    ...(model.reasoning ? { thinkingLevelMap } : {})
  };
}

/**
 * Pi's CLI runs attachments through `processImage` (resize to Pi's inline limits) before they
 * reach a session, but `session.prompt` itself forwards images untouched. `processImage` is not
 * exported, so this repeats it with the exported `resizeImage` and Pi's own defaults. The
 * dimension note Pi adds for tool reads is left out: it is for coordinate mapping, and here it
 * would show up in the user's own message.
 */
async function prepareImages(images: ImageContent[] | undefined): Promise<ImageContent[]> {
  if (!images?.length) return [];
  if (!piModule) throw new Error("Worker is not initialized");
  const prepared: ImageContent[] = [];
  for (const image of images) {
    const resized = await piModule.resizeImage(Buffer.from(image.data, "base64"), image.mimeType);
    if (!resized) throw new Error("An attached image could not be read, or could not be resized below the inline image size limit.");
    prepared.push({ type: "image", data: resized.data, mimeType: resized.mimeType });
  }
  return prepared;
}

async function initialize(command: InitCommand): Promise<void> {
  if (session) throw new Error("Worker is already initialized");
  taskId = command.taskId;
  activeCredential = command.apiKey;
  await mkdir(command.agentDir, { recursive: true });
  await mkdir(command.sessionDir, { recursive: true });
  const modelsPath = join(command.agentDir, "models.json");
  const modelsConfig = {
    providers: {
      [command.provider.id]: {
        name: command.provider.name,
        baseUrl: command.provider.baseUrl,
        api: command.provider.api,
        models: command.provider.models.map(modelDefinition)
      }
    }
  };
  await writeFile(modelsPath, `${JSON.stringify(modelsConfig, null, 2)}\n`, { mode: 0o600 });

  const pi = await import("@earendil-works/pi-coding-agent");
  piModule = pi;
  modelRuntime = await pi.ModelRuntime.create({
    authPath: join(command.agentDir, "auth.json"),
    modelsPath,
    modelsStorePath: join(command.agentDir, "models-store.json"),
    allowModelNetwork: false,
    refreshOnCreate: false
  });
  await modelRuntime.setRuntimeApiKey(command.provider.id, command.apiKey);
  const selectedModel = modelRuntime.getModel(command.provider.id, command.modelId);
  if (!selectedModel) throw new Error(`Configured model was not found: ${command.provider.name}/${command.modelId}`);

  const sessionManager = command.sessionFile && existsSync(command.sessionFile)
    ? pi.SessionManager.open(command.sessionFile, command.sessionDir, command.cwd)
    : pi.SessionManager.create(command.cwd, command.sessionDir);
  const settingsManager = pi.SettingsManager.inMemory({
    enableInstallTelemetry: false,
    enableAnalytics: false,
    cacheWarming: "off",
    defaultProjectTrust: "never",
    compaction: { enabled: true },
    retry: { enabled: true }
  }, { projectTrusted: false });
  // Every no* flag stays true: nothing is ever auto-discovered from settings or a project's
  // own .pi/ directory. The additional*Paths below are the sole load route, and the host only
  // puts paths there for packages the user installed and trusted.
  const resources = command.resources;
  const resourceLoader = new pi.DefaultResourceLoader({
    cwd: command.cwd,
    agentDir: command.agentDir,
    settingsManager,
    noExtensions: true,
    noSkills: true,
    noPromptTemplates: true,
    noThemes: true,
    noContextFiles: false,
    additionalExtensionPaths: resources?.extensions ?? [],
    additionalSkillPaths: resources?.skills ?? [],
    additionalPromptTemplatePaths: resources?.prompts ?? [],
    additionalThemePaths: resources?.themes ?? [],
    // WackCode's own extensions (ask_user_question, Plan mode, todo). Factories are not paths, so
    // they bypass the package trust machinery by construction and load even with noExtensions.
    extensionFactories: builtins.factories
  });
  await resourceLoader.reload();
  const created = await pi.createAgentSession({
    cwd: command.cwd,
    agentDir: command.agentDir,
    modelRuntime,
    model: selectedModel,
    thinkingLevel: command.thinkingLevel,
    excludeTools: UNSUPPORTED_TOOLS,
    sessionManager,
    settingsManager,
    resourceLoader
  });
  session = created.session;
  disabledTools = new Set(command.disabledTools ?? []);
  applyDisabledTools();

  // A broken extension must never stop a chat from starting, so load failures are reported
  // and the session continues without them. Built-in extensions (`<inline:…>`) always load;
  // the notice stays about installed packages.
  send({
    type: "extensions_loaded",
    taskId: command.taskId,
    loaded: created.extensionsResult.extensions
      .map((extension) => String(extension.path))
      .filter((path) => !path.startsWith("<inline:")),
    errors: created.extensionsResult.errors.map((entry) => ({ path: entry.path, error: safeError(entry.error) }))
  });

  // Binding gives extensions their UI surface and fires session_start. An extension that throws
  // during startup must not prevent `ready` either.
  try {
    await session.bindExtensions({
      uiContext: createExtensionUIContext() as never,
      mode: "rpc",
      abortHandler: () => { stopRequested = true; void session?.abort(); },
      onError: (error: unknown) => notice(safeError(error), "error")
    } as never);
  } catch (error) {
    notice(`An extension failed to start: ${safeError(error)}`, "error");
  }
  // Re-apply: an extension may register tools during session_start, and Pi auto-activates
  // anything new in the registry, which would quietly undo the user's denylist.
  applyDisabledTools();

  // The task record's mode wins over whatever the restored session says — it carries the
  // user's latest explicit choice (e.g. toggled while no worker was running). The restored
  // plan itself still comes from the session.
  if (command.mode && builtins.planMode.getState().mode !== command.mode) {
    try {
      builtins.planMode.setMode(command.mode);
    } catch (error) {
      notice(`Plan mode state could not be applied: ${safeError(error)}`, "error");
    }
  }

  session.subscribe((event) => {
    const value = event as unknown as Record<string, unknown>;
    const eventType = String(value.type ?? "event");
    if (eventType === "tool_execution_start" || eventType === "tool_execution_update" || eventType === "tool_execution_end") {
      send({
        type: "activity",
        taskId: command.taskId,
        event: eventType,
        detail: {
          toolName: value.toolName,
          toolCallId: value.toolCallId,
          args: eventType === "tool_execution_start" ? value.args : undefined,
          text: eventType === "tool_execution_update" ? toolUpdateText(value.partialResult) : undefined,
          isError: value.isError
        }
      });
    } else if (
      eventType.startsWith("compaction_") ||
      eventType.startsWith("auto_retry_") ||
      eventType.startsWith("summarization_retry_")
    ) {
      send({ type: "activity", taskId: command.taskId, event: eventType, detail: value });
    }
    if (eventType === "agent_end" && value.willRetry !== true) {
      const messages = Array.isArray(value.messages) ? value.messages : [];
      const failed = [...messages].reverse().find((message) => {
        const entry = message as Record<string, unknown> | undefined;
        return entry?.role === "assistant" && entry.stopReason === "error";
      }) as Record<string, unknown> | undefined;
      if (typeof failed?.errorMessage === "string" && failed.errorMessage) {
        send({ type: "worker_error", taskId: command.taskId, message: safeError(failed.errorMessage) });
      }
    }
    if (eventType === "message_update") {
      schedulePartial(value.message);
    } else if (
      eventType === "message_start" ||
      eventType === "message_end" ||
      eventType === "tool_execution_end" ||
      eventType === "compaction_end"
    ) {
      dropPartial();
      scheduleSnapshot();
    }
    if (eventType === "agent_settled") {
      activeRunId = undefined;
      dropPartial();
      send({ type: "run_state", taskId: command.taskId, state: "idle" });
      emitSnapshot();
    }
  });
  send({ type: "ready", taskId: command.taskId, snapshot: getSnapshot() });
}

async function handle(command: WorkerCommand): Promise<void> {
  try {
    if (command.type === "init") {
      await initialize(command);
    } else if (!session || !taskId) {
      throw new Error("Worker is not initialized");
    } else if (command.type === "prompt") {
      // A mode recorded on the task (or chosen for a draft) is applied before the prompt so
      // the first message of a plan-mode task arrives with the contract already in place.
      if (command.mode) builtins.planMode.setMode(command.mode);
      stopRequested = false;
      activeRunId = command.runId;
      send({ type: "run_state", taskId, runId: command.runId, state: "running" });
      response(command.id, true);
      try {
        const images = await prepareImages(command.images);
        await session.prompt(command.message, images.length > 0 ? { images } : undefined);
      } catch (error) {
        activeRunId = undefined;
        if (!stopRequested) send({ type: "worker_error", taskId, message: safeError(error) });
        send({ type: "run_state", taskId, runId: command.runId, state: "idle" });
      }
      stopRequested = false;
      emitSnapshot();
      return;
    } else if (command.type === "abort") {
      stopRequested = true;
      cancelPendingDialogs();
      send({ type: "run_state", taskId, runId: activeRunId, state: "stopping" });
      await session.abort();
      activeRunId = undefined;
      send({ type: "run_state", taskId, state: "idle" });
      emitSnapshot();
    } else if (command.type === "snapshot") {
      emitSnapshot();
    } else if (command.type === "set_model") {
      if (session.isStreaming) throw new Error("Wait for the current run before changing model");
      const model = modelRuntime?.getModel(session.model?.provider ?? "", command.modelId);
      if (!model) throw new Error(`Configured model was not found: ${command.modelId}`);
      await session.setModel(model);
      emitSnapshot();
    } else if (command.type === "set_thinking") {
      if (session.isStreaming) throw new Error("Wait for the current run before changing reasoning effort");
      session.setThinkingLevel(command.level);
      emitSnapshot();
    } else if (command.type === "set_mode") {
      if (session.isStreaming) throw new Error("Wait for the current run before changing mode");
      builtins.planMode.setMode(command.mode);
      emitSnapshot();
    } else if (command.type === "extension_ui_response") {
      pendingDialogs.get(command.requestId)?.({
        value: command.value,
        confirmed: command.confirmed,
        cancelled: command.cancelled,
        answers: command.answers
      });
    } else if (command.type === "set_tools") {
      disabledTools = new Set(command.disabledTools);
      applyDisabledTools();
      emitSnapshot();
    } else if (command.type === "shutdown") {
      if (!session.isIdle) await session.abort();
      session.dispose();
      response(command.id, true);
      process.exit(0);
    }
    response(command.id, true);
  } catch (error) {
    response(command.id, false, safeError(error));
    send({ type: "worker_error", taskId, message: safeError(error) });
  }
}

function safeError(error: unknown): string {
  let message = error instanceof Error ? error.message : String(error);
  if (activeCredential) message = message.split(activeCredential).join("[credential redacted]");
  return message.replace(/(sk-[A-Za-z0-9_-]{8,}|Bearer\s+[A-Za-z0-9._-]+)/gi, "[credential redacted]");
}

const decoder = new JsonLineDecoder();
process.stdin.on("data", (chunk: Buffer) => {
  for (const line of decoder.push(chunk)) {
    let command: WorkerCommand;
    try {
      command = JSON.parse(line) as WorkerCommand;
    } catch {
      send({ type: "worker_error", taskId, message: "The desktop bridge sent invalid JSON" });
      continue;
    }
    // Cancellation and dialog answers must bypass the prompt queue: a prompt holds the queue
    // until the agent settles. An extension awaiting ctx.ui.confirm() is doing so *inside* that
    // prompt, so queueing its answer behind the prompt would deadlock the run outright.
    if (command.type === "abort" || command.type === "extension_ui_response") {
      void handle(command).catch((error) => {
        send({ type: "worker_error", taskId, message: safeError(error) });
      });
      continue;
    }
    commandQueue = commandQueue.then(() => handle(command)).catch((error) => {
      send({ type: "worker_error", taskId, message: safeError(error) });
    });
  }
});

process.stdin.resume();
process.on("SIGTERM", () => {
  void (async () => {
    try {
      if (session && !session.isIdle) await session.abort();
      session?.dispose();
    } finally {
      process.exit(0);
    }
  })();
});

process.on("uncaughtException", (error) => {
  send({ type: "worker_error", taskId, message: safeError(error) });
  process.exitCode = 1;
});

process.on("unhandledRejection", (error) => {
  send({ type: "worker_error", taskId, message: safeError(error) });
  process.exitCode = 1;
});
