import { describe, expect, it } from "vitest";
import { applyRunEvent, EMPTY_RUN_REGISTRY, runKey, runStatusLabel } from "./run-state";
import type { RunInfo } from "./types";

const run: RunInfo = { kind: "run", sessionId: "one", generation: 1, revision: 1, cwd: "/project", command: "pnpm dev", status: "running", exit: null };

describe("checkout run state", () => {
  it("keeps newer states and replacements when replies/events arrive out of order", () => {
    const finished = { ...run, revision: 2, status: "finished" as const, exit: { code: 0, signal: null } };
    const state = applyRunEvent(EMPTY_RUN_REGISTRY, { type: "changed", run: finished });
    expect(applyRunEvent(state, { type: "changed", run })).toBe(state);
    const next = { ...run, sessionId: "two", generation: 2 };
    const replaced = applyRunEvent(state, { type: "changed", run: next });
    expect(applyRunEvent(replaced, { type: "changed", run: finished })).toBe(replaced);
    expect(applyRunEvent(replaced, { type: "removed", kind: "run", cwd: run.cwd, sessionId: run.sessionId, generation: 1 }).sessions[runKey(run.cwd, "run")]).toBe(next);
  });

  it("does not resurrect removed runs from late exits or queries", () => {
    const state = applyRunEvent(EMPTY_RUN_REGISTRY, { type: "removed", kind: "run", cwd: run.cwd, sessionId: run.sessionId, generation: 1 });
    expect(applyRunEvent(state, { type: "changed", run: { ...run, revision: 3 } })).toBe(state);
    expect(applyRunEvent(state, { type: "changed", run: { ...run, sessionId: "two", generation: 2 } }).sessions[runKey(run.cwd, "run")].sessionId).toBe("two");
  });

  it("keeps builds independent of runs, including removal watermarks and late replies", () => {
    const build: RunInfo = { ...run, kind: "build", sessionId: "build", generation: 2, command: "pnpm build" };
    const state = applyRunEvent(applyRunEvent(EMPTY_RUN_REGISTRY, { type: "changed", run }), { type: "changed", run: build });
    expect(state.sessions[runKey(run.cwd, "run")]).toBe(run);
    expect(state.sessions[runKey(run.cwd, "build")]).toBe(build);
    const removed = applyRunEvent(state, { type: "removed", kind: "build", cwd: run.cwd, sessionId: build.sessionId, generation: 2 });
    expect(removed.sessions[runKey(run.cwd, "run")]).toBe(run);
    expect(applyRunEvent(removed, { type: "changed", run: { ...build, status: "finished", revision: 2 } })).toBe(removed);
    const updatedRun = { ...run, revision: 2, status: "finished" as const };
    expect(applyRunEvent(removed, { type: "changed", run: updatedRun }).sessions[runKey(run.cwd, "run")]).toBe(updatedRun);
    expect(runStatusLabel(build)).toBe("Building");
  });

  it("keeps independent checkouts and distinguishes failures from intentional stops", () => {
    const state = applyRunEvent(applyRunEvent(EMPTY_RUN_REGISTRY, { type: "changed", run }),
      { type: "changed", run: { ...run, cwd: "/worktree", sessionId: "tree", generation: 2 } });
    expect(Object.keys(state.sessions)).toHaveLength(2);
    expect(runStatusLabel({ ...run, status: "failed", exit: { code: 7, signal: null } })).toBe("Failed (code 7)");
    expect(runStatusLabel({ ...run, status: "stopped", exit: { code: 137, signal: "Killed" } })).toBe("Stopped");
  });
});
