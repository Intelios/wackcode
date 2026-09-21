/**
 * The LLM-facing tool envelope. Ported from `@juicesharp/rpiv-todo` v2.11.0 (MIT)
 * `tool/response-envelope.ts`, adapted to WackCode: the `owner` line is gone and `details`
 * drops upstream's `params` echo. `details` is the persistence + replay snapshot —
 * `state.ts` consumes this exact shape on session lifecycle events, and the desktop
 * renders its panel from the same data via `todo_state`.
 */
import type { TodoTask } from "../../protocol.js";
import type { Op } from "./reducer.js";
import { sanitizeTerminalText } from "./sanitize.js";
import type { TaskState } from "./state.js";
import { deriveBlocks } from "./task-graph.js";
import { TODO_DETAILS_VERSION, type TaskAction, type TodoDetails } from "./types.js";

/**
 * Format a single task as a `[status] #id subject [(activeForm)] [⛓ #dep,…]` line,
 * used by the `list` content branch only.
 */
function formatListLine(task: TodoTask): string {
  const block = task.blockedBy?.length ? ` ⛓ ${task.blockedBy.map((id) => `#${id}`).join(",")}` : "";
  const form = task.status === "in_progress" && task.activeForm ? ` (${sanitizeTerminalText(task.activeForm)})` : "";
  return `[${task.status}] #${task.id} ${sanitizeTerminalText(task.subject)}${form}${block}`;
}

/** Multi-line presentation for the `get` action: description, activeForm, blockedBy, blocks. */
function formatGetLines(task: TodoTask, state: TaskState): string {
  const blocks = deriveBlocks(state.tasks).get(task.id) ?? [];
  const lines = [`#${task.id} [${task.status}] ${sanitizeTerminalText(task.subject)}`];
  if (task.description) lines.push(`  description: ${sanitizeTerminalText(task.description)}`);
  if (task.activeForm) lines.push(`  activeForm: ${sanitizeTerminalText(task.activeForm)}`);
  if (task.blockedBy?.length) {
    lines.push(`  blockedBy: ${task.blockedBy.map((id) => `#${id}`).join(", ")}`);
  }
  if (blocks.length) {
    lines.push(`  blocks: ${blocks.map((id) => `#${id}`).join(", ")}`);
  }
  return lines.join("\n");
}

/**
 * Pure formatter: `(op, state) → string`. Closed switch on `op.kind` — adding a new `Op`
 * variant fails to compile here until a branch is added. Strings match upstream verbatim.
 */
export function formatContent(op: Op, state: TaskState): string {
  switch (op.kind) {
    case "create": {
      const task = state.tasks.find((entry) => entry.id === op.taskId);
      // Defensive — `op.taskId` always resolves on the success path.
      if (!task) return `Created #${op.taskId}`;
      return `Created #${task.id}: ${sanitizeTerminalText(task.subject)} (pending)`;
    }
    case "update": {
      if (!op.changed) {
        return `No change: #${op.id} already matches the requested values (status: ${op.toStatus})`;
      }
      const transition = op.fromStatus !== op.toStatus ? ` (${op.fromStatus} → ${op.toStatus})` : "";
      return `Updated #${op.id}${transition}`;
    }
    case "delete":
      return `Deleted #${op.id}: ${sanitizeTerminalText(op.subject)}`;
    case "clear":
      return `Cleared ${op.count} tasks`;
    case "list": {
      let view = state.tasks;
      if (!op.includeDeleted) view = view.filter((task) => task.status !== "deleted");
      if (op.statusFilter) view = view.filter((task) => task.status === op.statusFilter);
      return view.length === 0 ? "No tasks" : view.map(formatListLine).join("\n");
    }
    case "get":
      return formatGetLines(op.task, state);
    case "error":
      return `Error: ${op.message}`;
  }
}

/**
 * Build the LLM-facing tool envelope after the new state is committed. Errors are
 * in-band: `content` carries `Error: …` and `details.error` the bare message, with the
 * (unchanged) task state still embedded so replay stays consistent.
 */
export function buildToolResult(
  action: TaskAction,
  state: TaskState,
  op: Op,
): { content: Array<{ type: "text"; text: string }>; details: TodoDetails } {
  const details: TodoDetails = {
    version: TODO_DETAILS_VERSION,
    action,
    tasks: state.tasks,
    nextId: state.nextId,
    ...(op.kind === "error" ? { error: op.message } : {}),
  };
  return { content: [{ type: "text", text: formatContent(op, state) }], details };
}
