import { describe, expect, it } from "vitest";
import { DEFAULT_EXECUTION_POLICY, isReadOnlyPlanning, normalizeExecutionPolicy } from "./execution-policy.js";
import type { ExecutionPolicyConfig, SubagentSpec, TaskMode } from "./protocol.js";
import { resolveSubagentSpec } from "./builtin/subagents/access.js";
import { resolveChildTools } from "./builtin/subagents/types.js";
import { createPlanModeExtension } from "./builtin/plan-mode/index.js";
import { createModeContractMessage } from "./builtin/plan-mode/contract.js";
import { EXECUTION_POLICY_MESSAGE_TYPE, reconcilePlanningAccess } from "./builtin/plan-mode/access.js";
import type { BuiltinHost } from "./builtin/host.js";

const readonlyRole = (name: string): SubagentSpec => ({
  name, builtin: name !== "custom", description: "Read-only investigation, and never edits.",
  prompt: "Role instructions.\n\nYou are read-only: never modify files.\n\nReport findings.",
  tools: ["read", "bash"], readOnly: true,
});
const editor: SubagentSpec = { ...readonlyRole("worker"), prompt: "Make the change.", tools: ["read", "bash", "write"], readOnly: false };

describe("execution policy", () => {
  it("defaults to restricted even with absent or malformed values", () => {
    expect(normalizeExecutionPolicy()).toEqual(DEFAULT_EXECUTION_POLICY);
    expect(normalizeExecutionPolicy({ unrestrictedPlanning: "true" as unknown as boolean })).toEqual(DEFAULT_EXECUTION_POLICY);
  });

  for (const mode of ["build", "plan", "ultraplan"] as TaskMode[]) {
    for (const unrestrictedPlanning of [false, true]) {
      for (const unrestrictedSubagents of [false, true]) {
        it(`${mode}: planning=${unrestrictedPlanning}, children=${unrestrictedSubagents}`, () => {
          const policy = { unrestrictedPlanning, unrestrictedSubagents };
          const ceiling = mode !== "build" && !unrestrictedPlanning;
          expect(isReadOnlyPlanning(mode, policy)).toBe(ceiling);
          for (const name of ["scout", "reviewer", "custom"]) {
            const original = readonlyRole(name);
            const resolved = resolveSubagentSpec(original, mode, policy);
            const unlocked = unrestrictedSubagents && !ceiling;
            expect(resolved.readOnly).toBe(!unlocked);
            expect(resolved.tools.includes("edit")).toBe(unlocked);
            expect(resolved.tools.includes("write")).toBe(unlocked);
            expect(resolved.prompt.startsWith(original.prompt)).toBe(name === "custom" || !unlocked);
            expect(original.readOnly).toBe(true);
            expect(original.tools).toEqual(["read", "bash"]);
          }
          const worker = resolveSubagentSpec(editor, mode, policy);
          expect(worker.tools).toEqual(editor.tools);
          expect(worker.readOnly).toBe(false);
        });
      }
    }
  }

  it("retains the denylist, supported child inventory, and custom instructions when unlocked", () => {
    const original = { ...readonlyRole("custom"), tools: ["read", "bash", "browser_act", "mcp__tool", "package_tool"] };
    const resolved = resolveSubagentSpec(original, "build", { unrestrictedSubagents: true, unrestrictedPlanning: false });
    expect(resolveChildTools(resolved.tools, resolved.readOnly, ["read", "edit", "browser_act", "mcp__tool", "package_tool"])).toEqual(["read", "edit"]);
    expect(resolved.prompt).toContain(original.prompt);
    expect(resolveSubagentSpec(original, "build", DEFAULT_EXECUTION_POLICY).readOnly).toBe(true);
  });
});

describe("planning access guidance", () => {
  it("preserves canonical contracts, never accumulates, and disappears when disabled", () => {
    const contract = createModeContractMessage("plan");
    const input = [contract];
    const enabled = reconcilePlanningAccess(input, true);
    expect(input).toEqual([contract]);
    expect(enabled[0]).toBe(contract);
    expect(enabled).toHaveLength(2);
    expect(reconcilePlanningAccess(enabled, true)).toHaveLength(2);
    expect(reconcilePlanningAccess(enabled, false)).toEqual(input);
    expect(enabled[1]).toMatchObject({ customType: EXECUTION_POLICY_MESSAGE_TYPE });
    expect(enabled[1].content).toContain("before the user approves");
    const request = { role: "user", content: "Review without making fixes." };
    const exchange = [{ role: "assistant", content: "Inspecting." }, { role: "tool", content: "Done." }];
    const context = reconcilePlanningAccess([contract, request, ...exchange], true);
    expect(context).toEqual([contract, enabled[1], request, ...exchange]);
  });
});

describe("planning tool enforcement", () => {
  for (const mode of ["plan", "ultraplan"] as const) {
    it(`unlocks all ordinary tools in ${mode} while retaining completion mode gates`, () => {
      let policy: ExecutionPolicyConfig = DEFAULT_EXECUTION_POLICY;
      const handlers = new Map<string, (event: any, ctx: any) => any>();
      const host = { publishPlanState() {} } as unknown as BuiltinHost;
      const extension = createPlanModeExtension(host, { owns: (name) => name.startsWith("mcp__"), isReadOnly: () => false }, () => policy);
      extension.factory({
        registerTool() {}, on(name: string, handler: any) { handlers.set(name, handler); },
        appendEntry() {}, sendMessage() {},
        getAllTools: () => [{ name: "bash", sourceInfo: { source: "builtin" } }, { name: "write", sourceInfo: { source: "builtin" } }, { name: "trusted_tool", sourceInfo: { source: "extension" } }],
      } as never);
      extension.controller.setMode(mode);
      const call = (toolName: string) => handlers.get("tool_call")!({ toolName, input: { command: "touch changed.txt" } }, { cwd: "/tmp" });
      for (const name of ["bash", "write", "trusted_tool", "mcp__mutate", "browser_act", "computer_act", "computer_open", "browser_open"]) {
        expect(call(name), name).toMatchObject({ block: true });
      }
      policy = { unrestrictedSubagents: false, unrestrictedPlanning: true };
      for (const name of ["bash", "write", "trusted_tool", "mcp__mutate", "browser_act", "computer_act", "computer_open", "browser_open"]) {
        expect(call(name), name).toBeUndefined();
      }
      extension.controller.setMode("build");
      expect(call("plan_mode_complete")).toMatchObject({ block: true });
      policy = DEFAULT_EXECUTION_POLICY;
      extension.controller.setMode(mode);
      expect(call("write")).toMatchObject({ block: true });
    });
  }
});
