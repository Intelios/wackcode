import type { ProjectRecord, TaskRecord } from "./types";

/** Creation order keeps new chats at the top without moving rows as a run updates them. */
export function newestChats(tasks: TaskRecord[]): TaskRecord[] {
  const created = (task: TaskRecord) => {
    const time = Date.parse(task.createdAt);
    return Number.isNaN(time) ? 0 : time;
  };
  return [...tasks].sort((a, b) => created(b) - created(a));
}

/** Search sidebar metadata only; saved transcripts need not be loaded or workers started. */
export function matchingChats(tasks: TaskRecord[], projects: ProjectRecord[], query: string): TaskRecord[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return tasks;
  const names = new Map(projects.map((project) => [project.id, project.name]));
  return tasks.filter((task) => {
    // A Chat mode chat has no project group, so "No project" is not something to find it by.
    const project = task.kind === "chat" ? "" : task.projectId === null ? "No project" : names.get(task.projectId) ?? "No project";
    const text = `${task.name} ${project}`.toLocaleLowerCase();
    return terms.every((term) => text.includes(term));
  });
}

/** When a chat last started a run, or was created if it never has. */
export function lastActivity(task: TaskRecord): number {
  const time = Date.parse(task.lastActivityAt ?? task.createdAt);
  return Number.isNaN(time) ? 0 : time;
}

export type RecencyGroup = { key: "today" | "yesterday" | "week" | "earlier"; label: string; tasks: TaskRecord[] };

const RECENCY_LABELS: Record<RecencyGroup["key"], string> = { today: "Today", yesterday: "Yesterday", week: "Previous 7 days", earlier: "Earlier" };

/**
 * The Chat area's list: most recently active first, in calendar groups. Activity is stamped
 * when a run starts (`lastActivityAt`), so a row moves when the user sends and then stays put
 * while the reply streams. Empty groups are left out.
 */
export function groupByRecency(tasks: TaskRecord[], now: Date = new Date()): RecencyGroup[] {
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 24 * 60 * 60 * 1000;
  const keyOf = (time: number): RecencyGroup["key"] =>
    time >= startOfToday ? "today" : time >= startOfToday - day ? "yesterday" : time >= startOfToday - 7 * day ? "week" : "earlier";
  const groups = new Map<RecencyGroup["key"], TaskRecord[]>();
  for (const task of [...tasks].sort((a, b) => lastActivity(b) - lastActivity(a))) {
    const key = keyOf(lastActivity(task));
    groups.set(key, [...(groups.get(key) ?? []), task]);
  }
  return (["today", "yesterday", "week", "earlier"] as const)
    .filter((key) => groups.has(key))
    .map((key) => ({ key, label: RECENCY_LABELS[key], tasks: groups.get(key)! }));
}
