import { imageDataUrl } from "./attachment-utils";
import type { GoalState, ImageContent, NormalizedBlock, NormalizedMessage, PlanState, SessionSnapshot, SnapshotDelta, SubagentStreamFrame, SubagentTarget, SubagentView, TaskMode, TaskRecord, TaskRuntime, TodoState } from "./types";

export function validateInitCommand(args: string, projectId: string | null, mode: TaskMode): void {
  if (args.trim()) throw new Error("/init does not accept arguments.");
  if (!projectId) throw new Error("/init needs a project chat. Start a new chat and select a project folder.");
  if (isPlanMode(mode)) throw new Error("Switch to Build mode before running /init.");
}

/** Plan and Ultra Plan are both the read-only planning mode. */
export function isPlanMode(mode: TaskMode | undefined): boolean {
  return mode === "plan" || mode === "ultraplan";
}

/** ⇧Tab cycles Build → Plan → Ultra Plan → Build. */
export function nextMode(mode: TaskMode): TaskMode {
  return mode === "build" ? "plan" : mode === "plan" ? "ultraplan" : "build";
}

/** The Plan button enters Plan from Build, then toggles Plan ↔ Ultra Plan. */
export function planButtonTarget(mode: TaskMode): TaskMode {
  return mode === "plan" ? "ultraplan" : "plan";
}

export function titleFromPrompt(message: string, max = 48): string {
  const line = message
    .split("\n")
    .map((part) => part.trim())
    .find((part) => part.length > 0);
  if (!line) return "New chat";
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

export function displayPath(path: string): string {
  const normalized = path.replace(/^\.\//, "").replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length === 0) return path;
  if (normalized.startsWith("/") && parts.length > 2) return `…/${parts.slice(-2).join("/")}`;
  if (parts.length > 3) return `…/${parts.slice(-2).join("/")}`;
  return normalized;
}

export function formatTokens(value?: number): string {
  if (value === undefined) return "—";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/** A model spend: cents once it reaches a cent, finer below. */
export function formatCost(cost: number): string {
  return cost >= 0.01 ? `$${cost.toFixed(2)}` : `$${cost.toFixed(4)}`;
}

export function formatRunDuration(durationMs: number): string {
  const seconds = Math.floor(Math.max(0, durationMs) / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${remainder}s`;
  if (minutes > 0) return `${minutes}m ${remainder}s`;
  return `${remainder}s`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** How long ago an ISO timestamp was, in the sidebar's shorthand ("now", "5m", "7h", "3d", then a short date). */
export function formatRelativeTime(iso: string, now: Date = new Date()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const seconds = Math.max(0, Math.floor((now.getTime() - then) / 1000));
  if (seconds < 60) return "now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const date = new Date(then);
  const day = date.getDate();
  const month = MONTHS[date.getMonth()];
  return date.getFullYear() === now.getFullYear() ? `${day} ${month}` : `${day} ${month} ${date.getFullYear()}`;
}

/** Archived chats newest-first; chats archived before `archivedAt` existed fall back to `updatedAt`. */
export function sortedArchived(tasks: TaskRecord[]): TaskRecord[] {
  const archivedAt = (task: TaskRecord): number => {
    const time = Date.parse(task.archivedAt ?? task.updatedAt);
    return Number.isNaN(time) ? 0 : time;
  };
  return tasks
    .filter((task) => task.archived)
    .sort((a, b) => archivedAt(b) - archivedAt(a));
}

const THINKING_STREAM_MAX = 200;

/** A reasoning line with its markdown stripped: heading/bullet/quote markers and inline emphasis. */
function plainStreamLine(line: string): string {
  return line
    .replace(/^\s*(?:#{1,6}|[-*+]|>|\d+\.)\s+/, "")
    .replace(/\*\*|__|`/g, "")
    .trim();
}

/**
 * The collapsed thinking row's live stream: the reasoning text itself flattened onto one line,
 * markdown removed. Blank-line breaks become a middot so section headings (Claude and OpenAI
 * summaries open each section with one) read inline rather than latching the row — a heading is
 * just the next thing the stream flows past. Only the tail is returned: the row right-anchors the
 * text and fades the left edge, so early reasoning scrolls off as new words arrive.
 */
export function thinkingStream(text: string): string | undefined {
  const stream = text
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.split("\n").map(plainStreamLine).filter(Boolean).join(" "))
    .filter(Boolean)
    .join(" · ")
    .replace(/\s+/g, " ")
    .trim();
  return stream ? stream.slice(-THINKING_STREAM_MAX) : undefined;
}

function sameNumberList(a: number[] | undefined, b: number[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((value, index) => value === b[index]);
}

/**
 * Structural equality for the worker's plan state. Snapshots arrive as freshly parsed objects
 * even when nothing changed; keeping the previous object preserves the identity the
 * transcript's message memos compare on, so an unchanged state re-renders nothing.
 */
export function samePlanState(a: PlanState | undefined, b: PlanState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.mode === b.mode && a.phase === b.phase && a.plan === b.plan;
}

export function sameTodoState(a: TodoState | undefined, b: TodoState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b || a.tasks.length !== b.tasks.length) return false;
  return a.tasks.every((task, index) => {
    const other = b.tasks[index];
    return task.id === other.id
      && task.subject === other.subject
      && task.description === other.description
      && task.activeForm === other.activeForm
      && task.status === other.status
      && sameNumberList(task.blockedBy, other.blockedBy);
  });
}

export function sameGoalState(a: GoalState | undefined, b: GoalState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.objective === b.objective
    && a.phase === b.phase
    && a.iteration === b.iteration
    && a.maxIterations === b.maxIterations
    && a.noProgress === b.noProgress
    && a.lastReason === b.lastReason
    && a.lastNextAction === b.lastNextAction
    && a.note === b.note;
}

/**
 * Apply a worker `snapshot_delta` to the snapshot the renderer holds. Untouched messages keep
 * their object identity, so the transcript's per-message memos and its derived maps (built on
 * the messages array) are recomputed only for what actually changed. Absent delta fields mean
 * "unchanged"; `activeRun: null` clears the run, `goalState: null` clears the goal.
 */
export function applySnapshotDelta(snapshot: SessionSnapshot, delta: SnapshotDelta): SessionSnapshot {
  const activeRun = delta.activeRun === undefined ? snapshot.activeRun : delta.activeRun ?? undefined;
  return {
    ...snapshot,
    rev: delta.rev,
    messages: mergeMessages(snapshot.messages, delta.upserts, delta.removed),
    modelSwitches: delta.modelSwitches ?? snapshot.modelSwitches,
    sessionFile: delta.sessionFile ?? snapshot.sessionFile,
    runTimings: delta.runTimings ?? snapshot.runTimings,
    activeRun,
    tree: delta.tree ?? snapshot.tree,
    stats: delta.stats ?? snapshot.stats,
    planState: delta.planState === undefined || samePlanState(snapshot.planState, delta.planState) ? snapshot.planState : delta.planState,
    todoState: delta.todoState === undefined || sameTodoState(snapshot.todoState, delta.todoState) ? snapshot.todoState : delta.todoState,
    goalState: delta.goalState === undefined
      ? snapshot.goalState
      : delta.goalState === null
        ? undefined
        : sameGoalState(snapshot.goalState, delta.goalState) ? snapshot.goalState : delta.goalState
  };
}

/**
 * Removed ids first, then each upsert replaces its message by id or appends: the merge behind
 * `snapshot_delta` and `subagent_stream` frames. Untouched messages keep their objects, and a
 * change with nothing in it returns `messages` itself.
 */
export function mergeMessages(messages: NormalizedMessage[], upserts: NormalizedMessage[], removed: string[]): NormalizedMessage[] {
  if (removed.length === 0 && upserts.length === 0) return messages;
  const gone = new Set(removed);
  const next = messages.filter((message) => !gone.has(message.id));
  for (const upsert of upserts) {
    const index = next.findIndex((message) => message.id === upsert.id);
    if (index >= 0) next[index] = upsert;
    else next.push(upsert);
  }
  return next;
}

/**
 * The optimistic echo of a message just sent: what the transcript shows until the worker's
 * snapshot carries the real one. A send can sit behind a worker respawn, a workspace
 * checkpoint and MCP servers connecting before the worker records the message, and the
 * composer has already cleared — without the echo the bubble simply pops in late. The echo
 * reuses the sent text (attached files ride it, `splitFileSection` reads them back out) and
 * previews images from the composer's own copies; ids and entry ids belong to the real message,
 * so transcript actions (which are disabled while the run is active anyway) never see it.
 */
export function pendingEchoMessage(sent: string, images: ImageContent[] | undefined, startedAt: number): NormalizedMessage {
  const blocks: NormalizedBlock[] = [];
  if (sent) blocks.push({ type: "text", text: sent });
  for (const image of images ?? []) blocks.push({ type: "image", mimeType: image.mimeType, thumbnail: imageDataUrl(image) });
  return { id: `pending:${startedAt}`, role: "user", timestamp: startedAt, blocks };
}

/**
 * The transcript's message list: the snapshot's, plus the pending echo while the run it opened
 * has no user message of its own yet. The same arrival test the transcript uses for the active
 * turn (`timestamp >= activeRun.startedAt`): the worker timestamps the real message when it
 * records it, always after `startedAt`. Returns the snapshot's array itself when there is
 * nothing to append, keeping its identity for the memoized rows.
 */
export function withPendingEcho(runtime: TaskRuntime | undefined): NormalizedMessage[] {
  const pending = runtime?.pendingMessage;
  const messages = runtime?.snapshot?.messages ?? [];
  if (!pending) return messages;
  const arrived = messages.some((message) => message.role === "user" && message.timestamp !== undefined
    && message.timestamp >= (pending.timestamp ?? Number.NEGATIVE_INFINITY));
  return arrived ? messages : [...messages, pending];
}

/** The side panel's copy of a sub-agent it has just started watching, before the first frame. */
export function pendingSubagentView(target: SubagentTarget): SubagentView {
  return { toolCallId: target.toolCallId, index: target.index, rev: -1, messages: [], live: false, truncated: false, missing: false, loading: true };
}

/**
 * Apply one `subagent_stream` frame to the panel's copy. A reset replaces it whole; any other
 * frame chains onto the last one applied. Returns `view` itself for a frame that doesn't apply
 * (another child's, or a straggler from an earlier watch while the reset is on its way), and
 * `undefined` when a frame went missing: the caller watches again for a fresh reset.
 */
export function applySubagentFrame(view: SubagentView, frame: SubagentStreamFrame): SubagentView | undefined {
  if (view.toolCallId !== frame.toolCallId || view.index !== frame.index) return view;
  if (!frame.reset) {
    if (view.loading) return view;
    if (frame.rev !== view.rev + 1) return undefined;
  }
  return {
    toolCallId: view.toolCallId,
    index: view.index,
    rev: frame.rev,
    messages: frame.reset ? frame.upserts : mergeMessages(view.messages, frame.upserts, frame.removed),
    ...(frame.partial ? { partial: frame.partial } : {}),
    live: frame.live,
    truncated: frame.truncated === true,
    missing: frame.missing === true,
    loading: false
  };
}
