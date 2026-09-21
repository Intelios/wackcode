/**
 * Dependency-graph analysis over the task list. Verbatim port of `@juicesharp/rpiv-todo`
 * v2.11.0 (MIT) `state/task-graph.ts` — pure of module state so the reducer can preview
 * an update without mutating first.
 */
import type { TodoTask } from "../../protocol.js";

/**
 * Detect whether merging `newBlockedBy` into `taskId`'s `blockedBy` set would introduce
 * a cycle in the dependency graph.
 */
export function detectCycle(
  taskList: readonly TodoTask[],
  taskId: number,
  newBlockedBy: readonly number[],
): boolean {
  const edges = new Map<number, number[]>();
  for (const task of taskList) {
    if (task.id === taskId) {
      const merged = new Set([...(task.blockedBy ?? []), ...newBlockedBy]);
      edges.set(task.id, [...merged]);
    } else {
      edges.set(task.id, task.blockedBy ? [...task.blockedBy] : []);
    }
  }

  const visiting = new Set<number>();
  const visited = new Set<number>();
  const hasCycleFrom = (node: number): boolean => {
    if (visiting.has(node)) return true;
    if (visited.has(node)) return false;
    visiting.add(node);
    for (const neighbor of edges.get(node) ?? []) {
      if (hasCycleFrom(neighbor)) return true;
    }
    visiting.delete(node);
    visited.add(node);
    return false;
  };

  for (const node of edges.keys()) {
    if (hasCycleFrom(node)) return true;
  }
  return false;
}

/**
 * Inverse adjacency map: for each task `T`, which tasks list `T` in their `blockedBy`.
 * Used by the `get` action's "blocks: #x, #y" suffix line.
 */
export function deriveBlocks(taskList: readonly TodoTask[]): Map<number, number[]> {
  const blocks = new Map<number, number[]>();
  for (const task of taskList) {
    for (const dependency of task.blockedBy ?? []) {
      const entries = blocks.get(dependency) ?? [];
      entries.push(task.id);
      blocks.set(dependency, entries);
    }
  }
  return blocks;
}
