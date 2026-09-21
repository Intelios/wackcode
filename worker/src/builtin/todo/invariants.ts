/**
 * The todo status machine. Verbatim port of `@juicesharp/rpiv-todo` v2.11.0 (MIT)
 * `state/invariants.ts`: `completed` is one-way to `deleted` (never back to
 * `in_progress`), `deleted` is terminal.
 */
import type { TodoStatus } from "../../protocol.js";

/**
 * Allowed forward transitions per source status. Idempotent same→same is checked
 * separately in `isTransitionValid` so this table only enumerates real transitions.
 */
export const VALID_TRANSITIONS: Record<TodoStatus, ReadonlySet<TodoStatus>> = {
  pending: new Set(["in_progress", "completed", "deleted"]),
  in_progress: new Set(["pending", "completed", "deleted"]),
  completed: new Set(["deleted"]),
  deleted: new Set(),
};

export function isTransitionValid(from: TodoStatus, to: TodoStatus): boolean {
  if (from === to) return true;
  return VALID_TRANSITIONS[from].has(to);
}
