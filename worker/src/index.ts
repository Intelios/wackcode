import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { JsonLineDecoder } from "./framing.js";
import {
  THINKING_LEVELS,
  type InitCommand,
  type NormalizedBlock,
  type NormalizedMessage,
  type SessionSnapshot,
  type ThinkingLevel,
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
let modelRuntime: ModelRuntime | undefined;
let activeRunId: string | undefined;
let activeCredential: string | undefined;
let stopRequested = false;
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
      contextUsage: stats.contextUsage
    },
    thinkingLevel: session.thinkingLevel as ThinkingLevel,
    availableThinkingLevels: session.getAvailableThinkingLevels() as ThinkingLevel[],
    model: model ? { provider: model.provider, id: model.id, name: model.name } : undefined
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
    input: ["text"],
    contextWindow: model.contextWindow,
    maxTokens: model.maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    ...(model.reasoning ? { thinkingLevelMap } : {})
  };
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
  const resourceLoader = new pi.DefaultResourceLoader({
    cwd: command.cwd,
    agentDir: command.agentDir,
    settingsManager,
    noExtensions: true,
    noSkills: true,
    noPromptTemplates: true,
    noThemes: true,
    noContextFiles: false
  });
  await resourceLoader.reload();
  const created = await pi.createAgentSession({
    cwd: command.cwd,
    agentDir: command.agentDir,
    modelRuntime,
    model: selectedModel,
    thinkingLevel: command.thinkingLevel,
    tools: ["read", "bash", "edit", "write"],
    sessionManager,
    settingsManager,
    resourceLoader
  });
  session = created.session;
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
      stopRequested = false;
      activeRunId = command.runId;
      send({ type: "run_state", taskId, runId: command.runId, state: "running" });
      response(command.id, true);
      try {
        await session.prompt(command.message);
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
    // Cancellation must bypass the prompt queue: a prompt holds the queue until the
    // agent settles, so serializing abort behind it would make Stop ineffective.
    if (command.type === "abort") {
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
