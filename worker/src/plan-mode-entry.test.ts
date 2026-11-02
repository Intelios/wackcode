import type { ToolDefinition } from "@earendil-works/pi-coding-agent";
import { describe, expect, it, vi } from "vitest";
import type { BuiltinHost } from "./builtin/host.js";
import { createPlanModeExtension } from "./builtin/plan-mode/index.js";
import { normalizePlanModeEntry, PLAN_MODE_ENTER_TOOL_NAME } from "./builtin/plan-mode/entry.js";
import { PLAN_STATE_ENTRY_TYPE, restorePlanState } from "./builtin/plan-mode/state.js";

function harness() {
  const tools = new Map<string, ToolDefinition>();
  const handlers = new Map<string, (event: any, ctx: any) => any>();
  const branch: unknown[] = [];
  const host = {
    publishPlanState: vi.fn(),
    hasRunningShellJobs: vi.fn(() => false),
    hasOutstandingSubagents: vi.fn(() => false),
  };
  const extension = createPlanModeExtension(host as unknown as BuiltinHost);
  extension.factory({
    registerTool: (tool: ToolDefinition) => tools.set(tool.name, tool),
    on: (name: string, handler: (event: any, ctx: any) => any) => handlers.set(name, handler),
    appendEntry: (customType: string, data: unknown) => branch.push({ type: "custom", customType, data }),
    sendMessage: (message: unknown) => branch.push({ type: "message", message }),
  } as never);
  const call = (name: string, params: unknown, signal?: AbortSignal) => tools.get(name)!.execute("call", params, signal, undefined, {} as never);
  return {
    ...extension, host, branch, tools,
    enter: (params: unknown = {}, signal?: AbortSignal) => call(PLAN_MODE_ENTER_TOOL_NAME, params, signal),
    complete: (plan: string) => call("plan_mode_complete", { plan }),
    fire: (event: string, input: unknown, entries: unknown[] = branch) => handlers.get(event)!(input, { cwd: "/tmp", sessionManager: { getBranch: () => entries } }),
  };
}

describe("agent-initiated planning", () => {
  it.each([{}, { mode: "plan" }, { mode: "ultraplan" }])("enters the requested variant and publishes its contract: %j", async (params) => {
    const h = harness();
    const mode = "mode" in params ? params.mode : "plan";
    expect(h.tools.get(PLAN_MODE_ENTER_TOOL_NAME)?.executionMode).toBe("sequential");
    const result = await h.enter(params);
    expect(result).not.toHaveProperty("terminate");
    expect(result.details).toEqual({ mode });
    expect(h.controller.getState()).toEqual({ mode, phase: "planning" });
    expect(h.host.publishPlanState).toHaveBeenLastCalledWith({ mode, phase: "planning" });
    expect(restorePlanState(h.branch).active).toBe(mode);
    const context = h.fire("context", { messages: [] });
    expect(context.messages[0].content).toContain(`CONTRACT v1: ${mode?.toUpperCase()}`);
  });

  it.each([null, undefined, [], "plan", { mode: null }, { mode: "build" }, { mode: "unknown" }, { mode: 1 }])("rejects invalid input: %j", (input) => {
    expect(() => normalizePlanModeEntry(input)).toThrow();
  });

  it("leaves mode and history alone on invalid or cancelled calls", async () => {
    const h = harness();
    await expect(h.enter({ mode: "build" })).rejects.toThrow("only the user");
    await expect(h.enter({}, AbortSignal.abort())).rejects.toThrow("cancelled");
    expect(h.controller.getState().mode).toBe("build");
    expect(h.branch).toEqual([]);
    expect(h.host.publishPlanState).not.toHaveBeenCalled();
  });

  it("keeps repeated entry idempotent and preserves a proposal across variants", async () => {
    const h = harness();
    await h.enter();
    await h.complete("# Ready plan");
    const before = [...h.branch];
    h.host.publishPlanState.mockClear();
    h.host.hasOutstandingSubagents.mockReturnValue(true);
    expect((await h.enter()).content).toEqual([{ type: "text", text: expect.stringContaining("already active") }]);
    expect(h.branch).toEqual(before);
    expect(h.host.publishPlanState).not.toHaveBeenCalled();
    h.host.hasOutstandingSubagents.mockReturnValue(false);
    await h.enter({ mode: "ultraplan" });
    expect(h.controller.getState()).toEqual({ mode: "ultraplan", phase: "ready", plan: "# Ready plan" });
    expect(restorePlanState(h.branch)).toEqual({ active: "ultraplan", plan: "# Ready plan" });
    expect(h.fire("tool_call", { toolName: PLAN_MODE_ENTER_TOOL_NAME })).toBeUndefined();
    await h.enter({ mode: "plan" });
    expect(h.controller.getState()).toEqual({ mode: "plan", phase: "ready", plan: "# Ready plan" });
  });

  it.each(["hasRunningShellJobs", "hasOutstandingSubagents"] as const)("refuses a mode change while %s, then allows retry", async (query) => {
    const h = harness();
    h.host[query].mockReturnValue(true);
    await expect(h.enter()).rejects.toThrow(query === "hasRunningShellJobs" ? "bash_job" : "subagent_job");
    expect(h.branch).toEqual([]);
    expect(h.controller.getState().mode).toBe("build");
    h.host[query].mockReturnValue(false);
    await h.enter();
    const before = [...h.branch];
    h.host[query].mockReturnValue(true);
    await expect(h.enter({ mode: "ultraplan" })).rejects.toThrow("retry plan_mode_enter");
    expect(h.branch).toEqual(before);
    expect(h.controller.getState().mode).toBe("plan");
  });

  it("restores the mode from the branch when navigating before and after entry", async () => {
    const h = harness();
    await h.enter({ mode: "ultraplan" });
    const saved = [...h.branch];
    h.fire("session_tree", {}, []);
    expect(h.controller.getState().mode).toBe("build");
    h.fire("session_tree", {}, saved);
    expect(h.controller.getState().mode).toBe("ultraplan");
    expect(saved.filter((entry: any) => entry.customType === PLAN_STATE_ENTRY_TYPE)).toHaveLength(1);
  });
});
