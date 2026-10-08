import { describe, expect, it } from "vitest";
import { groupByRecency, lastActivity, matchingChats, newestChats } from "./sidebar-utils";
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

describe("the Chat area's recency groups", () => {
  // Wednesday 7 October 2026, mid-afternoon, local time.
  const now = new Date(2026, 9, 7, 15, 0, 0);
  const at = (day: number, hour = 12) => new Date(2026, 9, day, hour).toISOString();
  const used = (id: string, lastActivityAt: string | null, createdAt = at(1)) => chat(id, createdAt, { kind: "chat", projectId: null, lastActivityAt });

  it("groups by the day a chat was last used, most recent first, and drops empty groups", () => {
    const groups = groupByRecency([
      used("last-week", at(2)),
      used("this-morning", at(7, 9)),
      used("yesterday", at(6, 23)),
      used("just-now", at(7, 14)),
      used("ancient", new Date(2026, 5, 1).toISOString()),
    ], now);
    expect(groups.map((group) => [group.label, group.tasks.map((task) => task.id)])).toEqual([
      ["Today", ["just-now", "this-morning"]],
      ["Yesterday", ["yesterday"]],
      ["Previous 7 days", ["last-week"]],
      ["Earlier", ["ancient"]],
    ]);
    expect(groupByRecency([used("only", at(7))], now).map((group) => group.key)).toEqual(["today"]);
    expect(groupByRecency([], now)).toEqual([]);
  });

  it("moves an old chat up when it is used again, whatever else changed on its record", () => {
    // Created long ago and renamed since (`updatedAt`), but only a run counts as use.
    const revived = used("revived", at(7, 13), new Date(2026, 0, 1).toISOString());
    const renamed = chat("renamed", new Date(2026, 0, 2).toISOString(), { kind: "chat", projectId: null, lastActivityAt: null, updatedAt: at(7, 14) });
    const groups = groupByRecency([renamed, revived], now);
    expect(groups.map((group) => [group.key, group.tasks.map((task) => task.id)])).toEqual([["today", ["revived"]], ["earlier", ["renamed"]]]);
  });

  it("falls back to the creation time until a chat has run, and tolerates a bad date", () => {
    expect(lastActivity(used("fresh", null, at(7, 10)))).toBe(Date.parse(at(7, 10)));
    expect(lastActivity(used("broken", "not a date"))).toBe(0);
  });

  it("does not find a Chat mode chat by the 'No project' group it is never shown in", () => {
    const tasks = [used("Trip plan", at(7)), chat("Loose coding chat", at(7), { projectId: null, kind: "code" })];
    expect(matchingChats(tasks, projects, "no project").map((task) => task.id)).toEqual(["Loose coding chat"]);
    expect(matchingChats(tasks, projects, "trip").map((task) => task.id)).toEqual(["Trip plan"]);
  });
});
