import { describe, expect, it } from "vitest";
import { areaDirection, parseArea, taskArea, tasksInArea } from "./areas";
import type { TaskRecord } from "./types";

const task = (id: string, kind: TaskRecord["kind"]) => ({ id, kind }) as TaskRecord;

describe("areas", () => {
  it("files every chat under exactly one area by its kind", () => {
    const tasks = [task("a", "code"), task("b", "chat"), task("c", "code")];
    expect(taskArea(tasks[1])).toBe("chat");
    expect(tasksInArea(tasks, "code").map((item) => item.id)).toEqual(["a", "c"]);
    expect(tasksInArea(tasks, "chat").map((item) => item.id)).toEqual(["b"]);
  });

  it("slides forward to Chat and back to Code", () => {
    expect(areaDirection("code", "chat")).toBe(1);
    expect(areaDirection("chat", "code")).toBe(-1);
  });

  it("falls back to Code for anything it does not recognise", () => {
    expect(parseArea("chat")).toBe("chat");
    for (const value of ["code", "", null, undefined, 7, "CHAT"]) expect(parseArea(value)).toBe("code");
  });
});
