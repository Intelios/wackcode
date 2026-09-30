/**
 * Read-only projection of a saved Pi session. This never creates an AgentSession, loads
 * resources, resolves a model, or reads credentials. Pi's pinned tree/compaction projection
 * and WackCode's live normalizer are shared so cold history cannot acquire different rules.
 * Migrations happen in memory only; opening history must never rewrite the session file.
 */
import { readFile } from "node:fs/promises";
import type { FileEntry, SessionEntry, SessionProjection } from "@earendil-works/pi-coding-agent";
import type { NormalizedBlock, SessionSnapshot, TaskMode, ThinkingLevel } from "./protocol.js";
import { normalizeMessage, THUMBNAIL_OPTIONS } from "./message-normalization.js";
import { transcriptMessages } from "./transcript.js";
import { buildTreeIndex, modelSwitchesOnPath, undoTarget, type EntryLike } from "./tree.js";
import { resolveRunTimings } from "./run-timing.js";
import { resolveThinkingDurations } from "./thinking-timing.js";
import { restorePlanState, PLAN_STATE_ENTRY_TYPE } from "./builtin/plan-mode/state.js";
import { replayFromBranch } from "./builtin/todo/state.js";
import { restoreGoalState } from "./builtin/goal/state.js";

type Pi = typeof import("@earendil-works/pi-coding-agent");
// Import the data-only module, bypassing Pi's CLI/SDK entry point. These paths are part of
// the pinned Pi installation shipped in the app, and exercised by the integration suite.
const piBase = () => import.meta.resolve("@earendil-works/pi-coding-agent");
const sessionHelpers = () => import(new URL("core/session-manager.js", piBase()).href) as Promise<Pick<Pi, "parseSessionEntries" | "migrateSessionEntries" | "buildSessionProjection">>;

export interface SavedSessionOptions {
  sessionFile?: string | null;
  taskId: string;
  mode: TaskMode;
  thinkingLevel: ThinkingLevel;
  contextWindow?: number;
}

export async function readSavedSession(options: SavedSessionOptions): Promise<SessionSnapshot> {
  let fileEntries: FileEntry[] = [];
  if (options.sessionFile) {
    const text = await readFile(options.sessionFile, "utf8");
    const pi = await sessionHelpers();
    fileEntries = pi.parseSessionEntries(text);
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

  // Previews use exactly the live worker's limits; originals and child transcripts never ride
  // a snapshot. Resize sequentially, without starting a model or loading any extensions.
  const previews = new WeakMap<object, NormalizedBlock>();
  let nextImageId = 0;
  let resize: Pi["resizeImage"] | undefined;
  const imageBlock = (block: Record<string, unknown>): NormalizedBlock => previews.get(block) ?? { type: "image", mimeType: String(block.mimeType ?? "image/png") };
  const prepare = async (content: unknown, maxBytes = THUMBNAIL_OPTIONS.maxBytes, maxWidth = THUMBNAIL_OPTIONS.maxWidth) => {
    if (!Array.isArray(content)) return;
    for (const block of content) {
      if (!block || typeof block !== "object" || block.type !== "image" || previews.has(block)) continue;
      const mimeType = typeof block.mimeType === "string" ? block.mimeType : "image/png";
      const preview: NormalizedBlock = { type: "image", mimeType, imageId: `image-${++nextImageId}` };
      previews.set(block, preview);
      if (typeof block.data !== "string" || !block.data) continue;
      try {
        resize ??= (await import(new URL("utils/image-resize.js", piBase()).href)).resizeImage as Pi["resizeImage"];
        const resized = await resize(Buffer.from(block.data, "base64"), mimeType, { maxWidth, maxHeight: maxWidth, maxBytes });
        if (resized) preview.thumbnail = `data:${resized.mimeType};base64,${resized.data}`;
      } catch { /* An unreadable image must not hide the rest of the transcript. */ }
    }
  };
  for (const message of context?.messages ?? []) {
    if (message.role === "user") await prepare(message.content);
    else if (message.role === "toolResult" && (message.toolName === "browser_screenshot" || message.toolName === "computer_screenshot")) await prepare(message.content, 64 * 1024, 480);
  }
  const messages = transcriptMessages(context?.messages ?? [], path as EntryLike[], index, resolveThinkingDurations(entries), {
    normalize: (raw, position, thinking) => normalizeMessage(raw, position, thinking, imageBlock), cache: new WeakMap()
  });
  const positions = new Map(messages.flatMap((message, position) => message.entryId ? [[message.entryId, position] as const] : []));
  const users = messages.filter((message) => message.role === "user" && message.entryId);
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
  return {
    rev: 0, sessionId: header?.id ?? options.taskId,
    ...(options.sessionFile ? { sessionFile: options.sessionFile } : {}), messages,
    modelSwitches: modelSwitchesOnPath(path as EntryLike[], positions),
    runTimings: resolveRunTimings(path, users.map((message) => message.entryId!), users.map((message) => message.id)),
    tree: { leafId: leaf?.id ?? null, undo: undoTarget(leaf as EntryLike | undefined) },
    stats: { tokens, cost, ...(context && options.contextWindow ? await contextStats(context, path, options.contextWindow, tokens) : {}) }, thinkingLevel: options.thinkingLevel, availableThinkingLevels: [],
    tools: [], activeTools: [],
    planState: { mode, phase: plan.plan !== undefined ? "ready" : "planning", ...(plan.plan !== undefined ? { plan: plan.plan } : {}) },
    todoState: { tasks: replayFromBranch(path).tasks }, goalState: restoreGoalState(path)
  };
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
