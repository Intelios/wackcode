import { describe, expect, it } from "vitest";
import { matchingChats, newestChats } from "./sidebar-utils";
import type { ProjectRecord, TaskRecord } from "./types";

const projects = [{ id: "p1", name: "TokenTrail" }] as ProjectRecord[];
const chat = (id: string, createdAt: string, extra: Partial<TaskRecord> = {}) => ({
  id, name: id, projectId: "p1", createdAt, updatedAt: "2030-01-01T12:00:00Z", ...extra
}) as TaskRecord;

describe("sidebar chat order", () => {
  it("uses creation time, preserves ties and leaves the source array untouched", () => {
    const tasks = [
      chat("old", "2026-09-01T12:00:00Z"),
      chat("new", "2026-09-30T13:00:00+01:00", { updatedAt: "2026-09-30T12:00:00Z" }),
      chat("tie", "2026-09-30T12:00:00Z"),
      chat("invalid", "not a date")
    ];
    expect(newestChats(tasks).map((task) => task.id)).toEqual(["new", "tie", "old", "invalid"]);
    expect(tasks.map((task) => task.id)).toEqual(["old", "new", "tie", "invalid"]);
  });
});

describe("sidebar search", () => {
  const tasks = [chat("Refactor parser", ""), chat("Fix typo", ""), chat("Loose chat", "", { projectId: null })];

  it("matches words across title and project name, ignoring case and extra whitespace", () => {
    expect(matchingChats(tasks, projects, "  PARSER   token ")).toEqual([tasks[0]]);
    expect(matchingChats(tasks, projects, "TokenTrail")).toEqual(tasks.slice(0, 2));
    expect(matchingChats(tasks, projects, "No project")).toEqual([tasks[2]]);
  });

  it("returns all chats for an empty query and none for a missing term", () => {
    expect(matchingChats(tasks, projects, " \t ")).toBe(tasks);
    expect(matchingChats(tasks, projects, "parser missing")).toEqual([]);
  });
});
