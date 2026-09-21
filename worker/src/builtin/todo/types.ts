/**
 * Todo tool identity, input normalization, and the persistence envelope shape. Ported from
 * `@juicesharp/rpiv-todo` v2.11.0 (MIT) `tool/types.ts`, adapted to WackCode: the `owner`
 * and `metadata` fields are gone, the parameter schema is plain JSON Schema like the other
 * built-ins (no TypeBox/pi-ai import), `TaskDetails` is versioned and drops upstream's
 * `params` echo (nothing reads it), and the `/todos` command strings are gone — the desktop
 * panel is the view. The tool name stays `todo`: branch replay keys on it.
 */
import type { TodoStatus, TodoTask } from "../../protocol.js";

export const TODO_TOOL_NAME = "todo";
export const TODO_TOOL_LABEL = "Todo";
export const TODO_DETAILS_VERSION = 1;

export type TaskAction = "create" | "update" | "list" | "get" | "delete" | "clear";

export const TASK_ACTIONS: readonly TaskAction[] = ["create", "update", "list", "get", "delete", "clear"];
export const TASK_STATUSES: readonly TodoStatus[] = ["pending", "in_progress", "completed", "deleted"];

/**
 * Persistence + replay snapshot. Every `todo` tool call returns this shape under `details`;
 * `state.ts` reads the latest one from the branch to rebuild the list. Field names are
 * pinned by replay compatibility.
 */
export interface TodoDetails {
  version: typeof TODO_DETAILS_VERSION;
  action: TaskAction;
  tasks: TodoTask[];
  nextId: number;
  error?: string;
}

/** The reducer's input bag. Types are enforced by `normalizeTodoParams`. */
export interface TaskMutationParams {
  subject?: string;
  description?: string;
  activeForm?: string;
  status?: TodoStatus;
  blockedBy?: number[];
  addBlockedBy?: number[];
  removeBlockedBy?: number[];
  id?: number;
  includeDeleted?: boolean;
}

export const TODO_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["action"],
  properties: {
    action: {
      type: "string",
      enum: [...TASK_ACTIONS],
      description:
        "What to do: create (new task), update (change status, fields, or dependencies), list (all tasks, optionally filtered by status), get (single task with dependencies), delete (tombstone a task), clear (reset the whole list).",
    },
    subject: { type: "string", description: "Task subject line (required for create)" },
    description: { type: "string", description: "Long-form task description" },
    activeForm: {
      type: "string",
      description: "Present-continuous label shown while status is in_progress (e.g. 'writing tests')",
    },
    status: {
      type: "string",
      enum: [...TASK_STATUSES],
      description:
        "Set this task's status (update), or filter returned tasks by status (list). One of pending, in_progress, completed, deleted.",
    },
    blockedBy: { type: "array", items: { type: "number" }, description: "Initial blockedBy ids (create only)" },
    addBlockedBy: {
      type: "array",
      items: { type: "number" },
      description: "Task ids to add to blockedBy (update only, additive merge)",
    },
    removeBlockedBy: {
      type: "array",
      items: { type: "number" },
      description: "Task ids to remove from blockedBy (update only)",
    },
    id: { type: "number", description: "Task id (required for update, get, delete)" },
    includeDeleted: {
      type: "boolean",
      description: "If true, list also returns deleted (tombstoned) tasks. Default: false.",
    },
  },
} as const;

type NormalizeResult = { ok: true; action: TaskAction; params: TaskMutationParams } | { ok: false; error: string };

/** Structural validation; state-aware rules live in the reducer (and stay in-band there). */
export function normalizeTodoParams(input: unknown): NormalizeResult {
  if (!isRecord(input)) return { ok: false, error: "parameters must be an object" };
  const action = input.action;
  if (typeof action !== "string" || !TASK_ACTIONS.includes(action as TaskAction)) {
    return { ok: false, error: `action must be one of ${TASK_ACTIONS.join(", ")}` };
  }
  const params: TaskMutationParams = {};
  for (const field of ["subject", "description", "activeForm"] as const) {
    const value = input[field];
    if (value === undefined) continue;
    if (typeof value !== "string") return { ok: false, error: `${field} must be a string` };
    params[field] = value;
  }
  if (input.status !== undefined) {
    if (typeof input.status !== "string" || !TASK_STATUSES.includes(input.status as TodoStatus)) {
      return { ok: false, error: `status must be one of ${TASK_STATUSES.join(", ")}` };
    }
    params.status = input.status as TodoStatus;
  }
  for (const field of ["blockedBy", "addBlockedBy", "removeBlockedBy"] as const) {
    const value = input[field];
    if (value === undefined) continue;
    if (!isNumberList(value)) return { ok: false, error: `${field} must be an array of task ids` };
    params[field] = value;
  }
  if (input.id !== undefined) {
    if (typeof input.id !== "number") return { ok: false, error: "id must be a number" };
    params.id = input.id;
  }
  if (input.includeDeleted !== undefined) {
    if (typeof input.includeDeleted !== "boolean") return { ok: false, error: "includeDeleted must be a boolean" };
    params.includeDeleted = input.includeDeleted;
  }
  return { ok: true, action: action as TaskAction, params };
}

function isNumberList(value: unknown): value is number[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === "number");
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
