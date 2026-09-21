/**
 * Todo list state and session persistence. Ported from `@juicesharp/rpiv-todo` v2.11.0
 * (MIT) `state/state.ts`, `state/store.ts` and `state/replay.ts`: like upstream there are
 * no disk writes — every `todo` tool result embeds the full list under `details`, and a
 * branch replay rebuilds it after a restart or compaction (last snapshot wins). The
 * sid-keyed multi-session store is dropped: one worker hosts exactly one session, so the
 * state lives in the extension's closure (`index.ts`) instead.
 */
import type { TodoTask } from "../../protocol.js";
import { TODO_DETAILS_VERSION, type TodoDetails, isRecord } from "./types.js";

/** The reducer's state: the task list plus the next id counter. */
export interface TaskState {
  tasks: TodoTask[];
  nextId: number;
}

export const EMPTY_STATE: TaskState = { tasks: [], nextId: 1 };

/**
 * Discriminator for `details` envelopes that match the persisted `TodoDetails` shape.
 * Defensive — entries from older, corrupt, or foreign sessions are skipped silently.
 */
export function isTaskDetails(value: unknown): value is TodoDetails {
  if (!isRecord(value)) return false;
  return (
    value.version === TODO_DETAILS_VERSION &&
    Array.isArray(value.tasks) &&
    typeof value.nextId === "number"
  );
}

/** Keep only well-formed task fields so a corrupt entry can never break the panel. */
function normalizeTask(value: unknown): TodoTask | undefined {
  if (!isRecord(value)) return undefined;
  if (typeof value.id !== "number" || typeof value.subject !== "string") return undefined;
  const status = value.status;
  if (status !== "pending" && status !== "in_progress" && status !== "completed" && status !== "deleted") {
    return undefined;
  }
  const task: TodoTask = { id: value.id, subject: value.subject, status };
  if (typeof value.description === "string") task.description = value.description;
  if (typeof value.activeForm === "string") task.activeForm = value.activeForm;
  if (Array.isArray(value.blockedBy) && value.blockedBy.length > 0) {
    task.blockedBy = value.blockedBy.filter((entry): entry is number => typeof entry === "number");
  }
  return task;
}

type SessionEntry = {
  type?: string;
  message?: {
    role?: string;
    toolName?: string;
    details?: unknown;
  };
};

/**
 * Walk a session branch in chronological order; the LAST `toolResult` whose
 * `toolName === "todo"` and whose `details` match `TodoDetails` wins (last-write-wins).
 * With no matching entry, returns `EMPTY_STATE`. Pure — the caller stores the result.
 */
export function replayFromBranch(branch: Iterable<unknown>): TaskState {
  let result: TaskState = { tasks: [], nextId: EMPTY_STATE.nextId };
  for (const raw of branch) {
    const entry = raw as SessionEntry;
    if (entry?.type !== "message") continue;
    const message = entry.message;
    if (message?.role !== "toolResult" || message.toolName !== "todo") continue;
    if (!isTaskDetails(message.details)) continue;
    result = {
      tasks: message.details.tasks.map(normalizeTask).filter((task): task is TodoTask => task !== undefined),
      nextId: message.details.nextId,
    };
  }
  return result;
}
