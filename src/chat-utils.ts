import type { PlanState, SessionSnapshot, SnapshotDelta, TaskMode, TodoState } from "./types";

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

export function formatRunDuration(durationMs: number): string {
  const seconds = Math.floor(Math.max(0, durationMs) / 1000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${remainder}s`;
  if (minutes > 0) return `${minutes}m ${remainder}s`;
  return `${remainder}s`;
}

const THINKING_PREVIEW_MAX = 200;
const BOLD_HEADING = /^\s*(?:\*\*|__)(.+?)(?:\*\*|__)[\s:.]*$/;
const HASH_HEADING = /^\s*#{1,6}\s+(.+)$/;

function plainPreviewText(text: string): string {
  return text
    .replace(/^\s*(?:#{1,6}|[-*+]|>|\d+\.)\s+/, "")
    .replace(/\*\*|__|`/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

function capPreview(text: string): string {
  return text.length > THINKING_PREVIEW_MAX ? `${text.slice(0, THINKING_PREVIEW_MAX - 1).trimEnd()}…` : text;
}

/**
 * A one-line gist of streaming reasoning: the latest heading when the model writes them (Claude
 * and OpenAI reasoning summaries open each section with one), otherwise the latest finished
 * sentence. The unfinished tail is ignored so the line only changes when a thought completes.
 */
export function thinkingPreview(text: string): string | undefined {
  const lines = text.split("\n");
  // The last line is still being written unless the text ends with a newline.
  const lastComplete = text.endsWith("\n") ? lines.length - 1 : lines.length - 2;
  let heading: string | undefined;
  let sentence: string | undefined;
  lines.forEach((line, index) => {
    const complete = index <= lastComplete;
    const bold = BOLD_HEADING.exec(line);
    const hash = complete ? HASH_HEADING.exec(line) : null;
    const title = bold?.[1] ?? hash?.[1];
    if (title && plainPreviewText(title)) {
      heading = plainPreviewText(title);
      return;
    }
    const boundary = /[.!?:]+\s+/g;
    let start = 0;
    for (let match = boundary.exec(line); match; match = boundary.exec(line)) {
      const end = match.index + match[0].length;
      const candidate = plainPreviewText(line.slice(start, end));
      if (candidate) sentence = candidate;
      start = end;
    }
    const rest = plainPreviewText(line.slice(start));
    if (complete && rest) sentence = rest;
  });
  const preview = heading ?? sentence;
  return preview ? capPreview(preview) : undefined;
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

/**
 * Apply a worker `snapshot_delta` to the snapshot the renderer holds. Untouched messages keep
 * their object identity, so the transcript's per-message memos and its derived maps (built on
 * the messages array) are recomputed only for what actually changed. Absent delta fields mean
 * "unchanged"; `activeRun: null` clears the run.
 */
export function applySnapshotDelta(snapshot: SessionSnapshot, delta: SnapshotDelta): SessionSnapshot {
  const removed = new Set(delta.removed);
  let messages = snapshot.messages;
  if (removed.size > 0 || delta.upserts.length > 0) {
    const next = snapshot.messages.filter((message) => !removed.has(message.id));
    for (const upsert of delta.upserts) {
      const index = next.findIndex((message) => message.id === upsert.id);
      if (index >= 0) next[index] = upsert;
      else next.push(upsert);
    }
    messages = next;
  }
  const activeRun = delta.activeRun === undefined ? snapshot.activeRun : delta.activeRun ?? undefined;
  return {
    ...snapshot,
    rev: delta.rev,
    messages,
    modelSwitches: delta.modelSwitches ?? snapshot.modelSwitches,
    sessionFile: delta.sessionFile ?? snapshot.sessionFile,
    runTimings: delta.runTimings ?? snapshot.runTimings,
    activeRun,
    tree: delta.tree ?? snapshot.tree,
    stats: delta.stats ?? snapshot.stats,
    planState: delta.planState === undefined || samePlanState(snapshot.planState, delta.planState) ? snapshot.planState : delta.planState,
    todoState: delta.todoState === undefined || sameTodoState(snapshot.todoState, delta.todoState) ? snapshot.todoState : delta.todoState
  };
}
