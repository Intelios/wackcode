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
    const project = task.projectId === null ? "No project" : names.get(task.projectId) ?? "No project";
    const text = `${task.name} ${project}`.toLocaleLowerCase();
    return terms.every((term) => text.includes(term));
  });
}
