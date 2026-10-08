import type { TaskRecord } from "./types";

/**
 * The app's two peer areas. Code is the coding agent with its projects; Chat is Chat mode, a
 * general-purpose chat with no project. Each has its own chat list, selection, draft and tabs.
 *
 * A chat belongs to exactly one area for life (`TaskRecord.kind`), so every list the Code area
 * shows is built from `tasksInArea(tasks, "code")` and never sees a Chat mode chat, and the other
 * way round. The area on screen is App's `area` state; a chat that is opened from anywhere else
 * (the tray menu, a tab) brings its own area forward first.
 */
export type Area = "code" | "chat";

/** In switch order: moving right goes forward, which is the way the sidebar slides. */
export const AREAS: readonly Area[] = ["code", "chat"];

export const AREA_LABELS: Record<Area, string> = { code: "Code", chat: "Chat" };

export const AREA_KEY = "wackcode:area";

export function taskArea(task: Pick<TaskRecord, "kind">): Area {
  return task.kind === "chat" ? "chat" : "code";
}

export function tasksInArea(tasks: TaskRecord[], area: Area): TaskRecord[] {
  return tasks.filter((task) => taskArea(task) === area);
}

/** Which way a switch travels: 1 enters from the right, -1 from the left. */
export function areaDirection(from: Area, to: Area): 1 | -1 {
  return AREAS.indexOf(to) < AREAS.indexOf(from) ? -1 : 1;
}

export function parseArea(value: unknown): Area {
  return value === "chat" ? "chat" : "code";
}
