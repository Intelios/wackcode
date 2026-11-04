/**
 * Read-only projection of a saved Pi session. This never creates an AgentSession, loads
 * resources, resolves a model, or reads credentials. Display history shares the live branch
 * projection; context statistics still use Pi's pinned compaction-aware model projection.
 * Migrations happen in memory only; opening history must never rewrite the session file.
 */
import { createReadStream } from "node:fs";
import { createInterface } from "node:readline";
import type { FileEntry, SessionEntry, SessionProjection } from "@earendil-works/pi-coding-agent";
import type { HistoryPreview, NormalizedBlock, ReasoningEffort, SavedHistoryFrame, SessionSnapshot, TaskMode, ThinkingLevel, ToolCatalogEntry } from "./protocol.js";
import { normalizeMessage, RESULT_THUMBNAIL_OPTIONS, THUMBNAIL_OPTIONS } from "./message-normalization.js";
import { createPreviewPool } from "./preview-decode.js";
import { transcriptEntries, transcriptMessages } from "./transcript.js";
import { buildTreeIndex, modelSwitchesOnPath, undoTarget, type EntryLike } from "./tree.js";
import { resolveRunTimings } from "./run-timing.js";
import { resolveThinkingDurations } from "./thinking-timing.js";
import { restorePlanState, PLAN_STATE_ENTRY_TYPE } from "./builtin/plan-mode/state.js";
import { replayFromBranch } from "./builtin/todo/state.js";
import { restoreGoalState } from "./builtin/goal/state.js";
import { backgroundCalls, interruptedDetails, projectBackgroundCards } from "./builtin/subagents/state.js";

type Pi = typeof import("@earendil-works/pi-coding-agent");
// Import the data-only module, bypassing Pi's CLI/SDK entry point. These paths are part of
// the pinned Pi installation shipped in the app, and exercised by the integration suite.
export const piBase = () => import.meta.resolve("@earendil-works/pi-coding-agent");
const sessionHelpers = () => import(new URL("core/session-manager.js", piBase()).href) as Promise<Pick<Pi, "migrateSessionEntries" | "buildSessionProjection">>;

export interface SavedSessionOptions {
  sessionFile?: string | null;
  taskId: string;
  mode: TaskMode;
  thinkingLevel: ReasoningEffort;
  contextWindow?: number;
  /** Host-owned metadata stamped onto the snapshot so Rust forwards its bytes untouched. */
  model?: { provider: string; id: string; name?: string | null };
  availableThinkingLevels?: ThinkingLevel[];
  modelMissing?: boolean;
  tools?: ToolCatalogEntry[];
  /** Host-owned development flag. Logs contain durations/counts only, never session contents. */
  debugTimings?: boolean;
}

interface PreviewJob {
  block: Record<string, unknown>;
  preview: NormalizedBlock;
  options: typeof THUMBNAIL_OPTIONS;
}

/** Test injection keeps the ordering contract verifiable without depending on image speed. */
interface SavedSessionDependencies {
  resizeImage?: Pi["resizeImage"];
  /** Stands in for the production pool so tests can observe bounded concurrency. */
  decodePool?: { submit: Pi["resizeImage"]; concurrency: number; close(): void };
}

async function prepareSavedSession(options: SavedSessionOptions): Promise<{ snapshot: SessionSnapshot; jobs: PreviewJob[] }> {
  const started = performance.now();
  let phaseStart = started;
  const mark = (phase: string, extra: Record<string, number> = {}) => {
    if (!options.debugTimings) return;
    const now = performance.now();
    process.stderr.write(`history timing: ${JSON.stringify({ phase, durationMs: Math.round(now - phaseStart), ...extra })}\n`);
    phaseStart = now;
  };
  let fileEntries: FileEntry[] = [];
  if (options.sessionFile) {
    // Stream the file a line at a time: session files run past 100 MB and holding the
    // whole text beside the parsed entries doubles peak memory for no benefit.
    const lines = createInterface({ input: createReadStream(options.sessionFile, { encoding: "utf8" }), crlfDelay: Infinity });
    for await (const line of lines) {
      if (!line.trim()) continue;
      try {
        fileEntries.push(JSON.parse(line) as FileEntry);
      } catch { /* Malformed lines are skipped, same as Pi's parser. */ }
    }
    const pi = await sessionHelpers();
    if (!fileEntries.some((entry) => entry?.type === "session")) throw new Error("This chat's saved session has no session header.");
    pi.migrateSessionEntries(fileEntries);
  }
  const header = fileEntries.find((entry) => entry.type === "session");
  const entries = fileEntries.filter((entry): entry is SessionEntry => entry.type !== "session" && typeof entry.id === "string");
  const index = buildTreeIndex(entries as EntryLike[]);
  const leaf = entries.at(-1);
  const path: SessionEntry[] = [];
  const visited = new Set<string>();
  let current = leaf;
  while (current) {
    if (visited.has(current.id)) throw new Error("This chat's saved session has a circular branch.");
    visited.add(current.id);
    path.push(current);
    current = current.parentId ? index.byId.get(current.parentId) as SessionEntry | undefined : undefined;
  }
  path.reverse();
  const context = entries.length ? (await sessionHelpers()).buildSessionProjection(entries, leaf?.id ?? null) : undefined;

  mark("read", { messages: fileEntries.length });
  // Register image identities without importing a decoder or touching pixels. The first
  // snapshot must be usable before ANY thumbnail work starts, including a decoder import.
  const previews = new WeakMap<object, NormalizedBlock>();
  const jobs: PreviewJob[] = [];
  let nextImageId = 0;
  const imageBlock = (block: Record<string, unknown>): NormalizedBlock => previews.get(block) ?? { type: "image", mimeType: String(block.mimeType ?? "image/png") };
  const register = (content: unknown, options = THUMBNAIL_OPTIONS) => {
    if (!Array.isArray(content)) return;
    for (const block of content) {
      if (!block || typeof block !== "object" || block.type !== "image" || previews.has(block)) continue;
      const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image/png";
      const preview: NormalizedBlock = { type: "image", mimeType, imageId: `image-${++nextImageId}` };
      previews.set(block, preview);
      jobs.push({ block, preview, options });
    }
  };
  for (const entry of transcriptEntries(path)) {
    const message = entry.raw as { role?: string; content?: unknown };
    if (message.role === "user") register(message.content);
    else if (message.role === "toolResult") register(message.content, RESULT_THUMBNAIL_OPTIONS);
  }
  const tokenHelpers = path.some((entry) => entry.type === "compaction")
    ? await import(new URL("core/compaction/compaction.js", piBase()).href) as Pick<Pi, "estimateTokens"> : undefined;
  const projectionHelpers = await sessionHelpers();
  const baseMessages = transcriptMessages([], path as EntryLike[], index, resolveThinkingDurations(entries), {
    normalize: (raw, position, thinking) => normalizeMessage(raw, position, thinking, imageBlock), cache: new WeakMap(),
    compactionTokensAfter: (entry) => tokenHelpers && projectionHelpers.buildSessionProjection(entries, entry.id).messages
      .reduce((sum, message) => sum + tokenHelpers.estimateTokens(message), 0)
  });
  const cards = backgroundCalls(path);
  for (const [id, call] of cards) cards.set(id, { ...call, details: interruptedDetails(call.details) });
  const messages = projectBackgroundCards(baseMessages, cards, new WeakMap());
  const positions = new Map(messages.flatMap((message, position) => message.entryId ? [[message.entryId, position] as const] : []));
  const users = messages.filter((message) => message.role === "user" && message.entryId);
  mark("normalize", { messages: messages.length, images: jobs.length });
  const plan = restorePlanState(path);
  const mode = path.some((entry) => entry.type === "custom" && entry.customType === PLAN_STATE_ENTRY_TYPE) ? plan.active ?? "build" : options.mode;
  const tokens = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 };
  let cost = 0;
  // Pi's session stats include all branches, summary usage and tool-attributed usage.
  for (const entry of entries) {
    const usage = entry.type === "message" && (entry.message.role === "assistant" || entry.message.role === "toolResult")
      ? entry.message.usage : entry.type === "usage" || entry.type === "compaction" || entry.type === "branch_summary" ? entry.usage : undefined;
    if (!usage) continue;
    for (const key of ["input", "output", "cacheRead", "cacheWrite"] as const) tokens[key] += usage[key] ?? 0;
    cost += usage.cost?.total ?? 0;
  }
  tokens.total = tokens.input + tokens.output + tokens.cacheRead + tokens.cacheWrite;
  const snapshot: SessionSnapshot = {
    rev: 0, sessionId: header?.id ?? options.taskId,
    workActivity: { parent: "idle", subagents: 0, pendingResults: 0 },
    ...(options.sessionFile ? { sessionFile: options.sessionFile } : {}), messages,
    modelSwitches: modelSwitchesOnPath(path as EntryLike[], positions, messages.length),
    runTimings: resolveRunTimings(path, users.map((message) => message.entryId!), users.map((message) => message.id)),
    tree: { leafId: leaf?.id ?? null, undo: undoTarget(leaf as EntryLike | undefined) },
    stats: { tokens, cost, ...(context && options.contextWindow ? await contextStats(context, path, options.contextWindow, tokens) : {}) }, thinkingLevel: options.thinkingLevel,
    availableThinkingLevels: options.availableThinkingLevels ?? [],
    ...(options.model ? { model: { ...options.model, name: options.model.name ?? undefined } } : {}),
    ...(options.modelMissing ? { modelMissing: true as const } : {}),
    tools: options.tools ?? [], activeTools: [],
    planState: { mode, phase: plan.plan !== undefined ? "ready" : "planning", ...(plan.plan !== undefined ? { plan: plan.plan } : {}) },
    todoState: { tasks: replayFromBranch(path).tasks }, goalState: restoreGoalState(path)
  };
  mark("stats", { messages: messages.length, images: jobs.length });
  return { snapshot, jobs };
}

/**
 * Stream complete text/state first, then small newest-first thumbnail batches. The host owns
 * process cancellation and its historyId: these frames never join the live revision chain.
 * Sequential resizing bounds decoder threads and memory even for hundreds of screenshots.
 */
export async function streamSavedSession(options: SavedSessionOptions, emit: (frame: SavedHistoryFrame) => void | Promise<void>, dependencies: SavedSessionDependencies = {}): Promise<void> {
  const { snapshot, jobs } = await prepareSavedSession(options);
  await emit({ type: "snapshot", snapshot, messageCount: snapshot.messages.length });
  const started = performance.now();
  // Tests inject a decoder; production goes through the persistent pool — the serial
  // one-worker-per-image path it replaced cost ~140 ms per saved screenshot.
  const pool = dependencies.resizeImage ? undefined : dependencies.decodePool ?? createPreviewPool(piBase());
  const decode = dependencies.resizeImage
    ?? ((bytes: Uint8Array, mimeType: string, opts: typeof THUMBNAIL_OPTIONS) => pool!.submit(bytes, mimeType, opts));
  const maxInflight = () => Math.max(1, pool?.concurrency ?? 1);
  let pending: HistoryPreview[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let writes = Promise.resolve();
  const flush = () => {
    if (timer) { clearTimeout(timer); timer = undefined; }
    if (!pending.length) return;
    const previews = pending;
    pending = [];
    writes = writes.then(() => emit({ type: "history_previews", previews }));
    // A timed flush may fail while a decoder is running. Observe it immediately, then
    // surface the failure at the next await rather than leaking an unhandled rejection.
    void writes.catch(() => undefined);
  };
  const inflight = new Set<Promise<void>>();
  const launch = (job: PreviewJob) => {
    const run = (async () => {
      const preview: HistoryPreview = { imageId: job.preview.imageId! };
      try {
        const resized = await decode(Buffer.from(job.block.data as string, "base64"), job.preview.mimeType!, job.options);
        if (resized) preview.thumbnail = `data:${resized.mimeType};base64,${resized.data}`;
        else preview.failed = true;
      } catch { /* An unreadable image cannot hide the rest of the transcript. */ }
      if (!preview.thumbnail) preview.failed = true;
      pending.push(preview);
      if (pending.length >= 8) flush();
      else timer ??= setTimeout(flush, 200);
    })();
    inflight.add(run);
    void run.then(() => inflight.delete(run));
  };
  try {
    for (let index = jobs.length - 1; index >= 0; index -= 1) {
      // Do not build an unbounded backlog if the host stops draining its pipe.
      await writes;
      if (inflight.size >= maxInflight()) await Promise.race(inflight);
      const job = jobs[index];
      if (typeof job.block.data !== "string" || !job.block.data) {
        pending.push({ imageId: job.preview.imageId!, failed: true });
        if (pending.length >= 8) flush();
        else timer ??= setTimeout(flush, 200);
        continue;
      }
      launch(job);
      // The job's base64 string is the only reference left once decode has copied it;
      // drop it so hundreds of megabytes of image text do not sit live for the whole stream.
      delete job.block.data;
    }
    await Promise.all(inflight);
  } finally {
    // Decoders that ran the WASM codec keep their parent's loop alive even unref'd.
    pool?.close();
    flush();
    await writes;
  }
  await emit({ type: "history_previews", previews: [], done: true });
  if (options.debugTimings) process.stderr.write(`history timing: ${JSON.stringify({ phase: "previews", durationMs: Math.round(performance.now() - started), images: jobs.length })}\n`);
}

/** Completed projection for callers that need all previews; the UI uses the stream above. */
export async function readSavedSession(options: SavedSessionOptions): Promise<SessionSnapshot> {
  let snapshot: SessionSnapshot | undefined;
  const previews = new Map<string, HistoryPreview>();
  await streamSavedSession(options, (frame) => {
    if (frame.type === "snapshot") snapshot = frame.snapshot;
    else for (const preview of frame.previews) previews.set(preview.imageId, preview);
  });
  if (!snapshot) throw new Error("This chat's saved session has no snapshot.");
  const messages = snapshot.messages.map((message) => ({
    ...message,
    blocks: message.blocks.map((block) => ({
      ...block,
      ...(block.imageId ? previews.get(block.imageId) : {}),
      ...(block.images ? { images: block.images.map((image) => ({ ...image, ...previews.get(image.imageId) })) } : {})
    }))
  }));
  return { ...snapshot, messages };
}

/** Match Pi's live context count, including retained compaction context and messages after usage. */
async function contextStats(projection: SessionProjection, path: SessionEntry[], contextWindow: number, tokens: SessionSnapshot["stats"]["tokens"]): Promise<Pick<SessionSnapshot["stats"], "contextUsage" | "contextBreakdown">> {
  const helpers = await import(new URL("core/compaction/compaction.js", piBase()).href) as Pick<Pi, "calculateContextTokens" | "estimateTokens"> & {
    estimateProjectedContextTokens(projection: SessionProjection, path: SessionEntry[]): { tokens: number };
  };
  let compactionIndex = -1;
  path.forEach((entry, position) => { if (entry.type === "compaction") compactionIndex = position; });
  if (compactionIndex >= 0) {
    const counted = new Set(projection.entries.filter((entry) => entry.messages.some((message) => message.role === "assistant"
      && message.stopReason !== "aborted" && message.stopReason !== "error" && helpers.calculateContextTokens(message.usage) > 0)).map((entry) => entry.sourceEntry.id));
    if (!path.slice(compactionIndex + 1).some((entry) => counted.has(entry.id))) {
      return { contextUsage: { tokens: null, contextWindow, percent: null } };
    }
  }
  const used = helpers.estimateProjectedContextTokens(projection, path).tokens;
  const roles = { user: 0, assistant: 0, tool: 0 };
  for (const message of projection.messages) {
    const role = message.role === "toolResult" ? "tool" : message.role;
    if (role === "user" || role === "assistant" || role === "tool") roles[role] += helpers.estimateTokens(message);
  }
  let entries = [
    { id: "system" as const, tokens: Math.max(0, used - roles.user - roles.assistant - roles.tool) },
    { id: "user" as const, tokens: roles.user }, { id: "assistant" as const, tokens: roles.assistant }, { id: "tool" as const, tokens: roles.tool }
  ];
  const total = entries.reduce((sum, entry) => sum + entry.tokens, 0);
  if (total > used && total > 0) entries = entries.map((entry) => ({ ...entry, tokens: Math.round(entry.tokens * used / total) }));
  return {
    contextUsage: { tokens: used, contextWindow, percent: used / contextWindow * 100 },
    contextBreakdown: { entries, cacheHitRate: tokens.input + tokens.cacheRead > 0 ? tokens.cacheRead / (tokens.input + tokens.cacheRead) : null }
  };
}
