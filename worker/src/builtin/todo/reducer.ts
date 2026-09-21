/**
 * Pure reducer: (state, action, params) → (state, op). Ported from `@juicesharp/rpiv-todo`
 * v2.11.0 (MIT) `state/state-reducer.ts`, adapted to WackCode: the `owner` and `metadata`
 * branches are gone. Validation stays in-band here — structural guards plus state-aware
 * checks (transition legality, dangling/deleted blockedBy, self-block, cycles) — so a
 * rejected mutation never changes state and the error travels in `op`.
 */
import type { TodoStatus, TodoTask } from "../../protocol.js";
import { isTransitionValid } from "./invariants.js";
import type { TaskState } from "./state.js";
import { detectCycle } from "./task-graph.js";
import type { TaskAction, TaskMutationParams } from "./types.js";

/**
 * Reducer outcome. Closed tagged union — adding a new action requires extending this
 * union AND the envelope's `formatContent` switch (compiler-enforced exhaustive).
 * `error` carries the message in-band so callers can match on `op.kind === "error"`.
 */
export type Op =
  | { kind: "create"; taskId: number }
  | { kind: "update"; id: number; fromStatus: TodoStatus; toStatus: TodoStatus; changed: boolean }
  | { kind: "delete"; id: number; subject: string }
  | { kind: "list"; statusFilter?: TodoStatus; includeDeleted: boolean }
  | { kind: "get"; task: TodoTask }
  | { kind: "clear"; count: number }
  | { kind: "error"; message: string };

export interface ApplyResult {
  state: TaskState;
  op: Op;
}

function errorResult(state: TaskState, message: string): ApplyResult {
  return { state, op: { kind: "error", message } };
}

function sameNumberList(a: number[] | undefined, b: number[] | undefined): boolean {
  const x = a ?? [];
  const y = b ?? [];
  return x.length === y.length && x.every((v, i) => v === y[i]);
}

/**
 * Did this `update` change anything? A no-effect update — `status` set to its current
 * value, or any field re-sent unchanged — reports "No change" instead of "Updated #N".
 * Without this, a no-op update is indistinguishable from a real mutation, which can
 * drive a model to re-issue the same call in a loop. blockedBy is order-sensitive (the
 * reducer preserves insertion order).
 */
function taskChanged(before: TodoTask, after: TodoTask): boolean {
  return (
    before.subject !== after.subject ||
    before.status !== after.status ||
    before.description !== after.description ||
    before.activeForm !== after.activeForm ||
    !sameNumberList(before.blockedBy, after.blockedBy)
  );
}

export function applyTaskMutation(
  state: TaskState,
  action: TaskAction,
  params: TaskMutationParams,
): ApplyResult {
  switch (action) {
    case "create": {
      if (!params.subject?.trim()) {
        return errorResult(state, "subject required for create");
      }
      if (params.blockedBy?.length) {
        for (const dependency of params.blockedBy) {
          const dependencyTask = state.tasks.find((task) => task.id === dependency);
          if (!dependencyTask) return errorResult(state, `blockedBy: #${dependency} not found`);
          if (dependencyTask.status === "deleted") return errorResult(state, `blockedBy: #${dependency} is deleted`);
        }
      }
      const newTask: TodoTask = {
        id: state.nextId,
        subject: params.subject,
        status: "pending",
      };
      if (params.description) newTask.description = params.description;
      if (params.activeForm) newTask.activeForm = params.activeForm;
      if (params.blockedBy?.length) newTask.blockedBy = [...params.blockedBy];

      return {
        state: { tasks: [...state.tasks, newTask], nextId: state.nextId + 1 },
        op: { kind: "create", taskId: newTask.id },
      };
    }

    case "update": {
      if (params.id === undefined) return errorResult(state, "id required for update");
      const index = state.tasks.findIndex((task) => task.id === params.id);
      if (index === -1) return errorResult(state, `#${params.id} not found`);
      const current = state.tasks[index];

      const hasMutation =
        params.subject !== undefined ||
        params.description !== undefined ||
        params.activeForm !== undefined ||
        params.status !== undefined ||
        (params.addBlockedBy && params.addBlockedBy.length > 0) ||
        (params.removeBlockedBy && params.removeBlockedBy.length > 0);
      if (!hasMutation) {
        return errorResult(
          state,
          "update requires at least one mutable field: subject, description, activeForm, status, addBlockedBy, or removeBlockedBy",
        );
      }

      let newStatus = current.status;
      if (params.status !== undefined) {
        if (!isTransitionValid(current.status, params.status)) {
          return errorResult(state, `illegal transition ${current.status} → ${params.status}`);
        }
        newStatus = params.status;
      }

      let newBlockedBy = current.blockedBy ? [...current.blockedBy] : [];
      if (params.removeBlockedBy?.length) {
        const toRemove = new Set(params.removeBlockedBy);
        newBlockedBy = newBlockedBy.filter((dependency) => !toRemove.has(dependency));
      }
      if (params.addBlockedBy?.length) {
        for (const dependency of params.addBlockedBy) {
          if (dependency === current.id) return errorResult(state, `cannot block #${current.id} on itself`);
          const dependencyTask = state.tasks.find((task) => task.id === dependency);
          if (!dependencyTask) return errorResult(state, `addBlockedBy: #${dependency} not found`);
          if (dependencyTask.status === "deleted") return errorResult(state, `addBlockedBy: #${dependency} is deleted`);
          if (!newBlockedBy.includes(dependency)) newBlockedBy.push(dependency);
        }
        if (detectCycle(state.tasks, current.id, newBlockedBy)) {
          return errorResult(state, "addBlockedBy would create a cycle in the blockedBy graph");
        }
      }

      const updated: TodoTask = { ...current, status: newStatus };
      if (params.subject !== undefined) updated.subject = params.subject;
      if (params.description !== undefined) updated.description = params.description;
      if (params.activeForm !== undefined) updated.activeForm = params.activeForm;
      if (newBlockedBy.length) updated.blockedBy = newBlockedBy;
      else delete updated.blockedBy;

      const newTasks = [...state.tasks];
      newTasks[index] = updated;
      return {
        state: { tasks: newTasks, nextId: state.nextId },
        op: {
          kind: "update",
          id: updated.id,
          fromStatus: current.status,
          toStatus: newStatus,
          changed: taskChanged(current, updated),
        },
      };
    }

    case "list": {
      return {
        state,
        op: {
          kind: "list",
          includeDeleted: params.includeDeleted === true,
          ...(params.status !== undefined ? { statusFilter: params.status } : {}),
        },
      };
    }

    case "get": {
      if (params.id === undefined) return errorResult(state, "id required for get");
      const task = state.tasks.find((entry) => entry.id === params.id);
      if (!task) return errorResult(state, `#${params.id} not found`);
      return { state, op: { kind: "get", task } };
    }

    case "delete": {
      if (params.id === undefined) return errorResult(state, "id required for delete");
      const index = state.tasks.findIndex((task) => task.id === params.id);
      if (index === -1) return errorResult(state, `#${params.id} not found`);
      const current = state.tasks[index];
      if (current.status === "deleted") return errorResult(state, `#${current.id} is already deleted`);
      const updated: TodoTask = { ...current, status: "deleted" };
      const newTasks = [...state.tasks];
      newTasks[index] = updated;
      return {
        state: { tasks: newTasks, nextId: state.nextId },
        op: { kind: "delete", id: updated.id, subject: updated.subject },
      };
    }

    case "clear": {
      return {
        state: { tasks: [], nextId: 1 },
        op: { kind: "clear", count: state.tasks.length },
      };
    }
  }
}
