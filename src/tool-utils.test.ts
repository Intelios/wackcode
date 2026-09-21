import { describe, expect, it } from "vitest";
import { diffStats, editStats, groupTools, pruneDisabledTools, sameToolCatalog, summarizeTool } from "./tool-utils";
import type { NormalizedBlock, ToolCatalogEntry } from "./types";

function call(toolName: string, args: unknown): NormalizedBlock {
  return { type: "tool-call", toolName, toolCallId: "c1", arguments: args };
}

describe("summarizeTool", () => {
  it("summarizes read calls", () => {
    const summary = summarizeTool(call("read", { path: "/Users/jack/app/src/main.ts" }));
    expect(summary.doneVerb).toBe("Read");
    expect(summary.subject).toBe("…/src/main.ts");
  });

  it("summarizes bash calls with the command", () => {
    const summary = summarizeTool(call("bash", { command: "pnpm test" }));
    expect(summary.doneVerb).toBe("Ran");
    expect(summary.subject).toBe("pnpm test");
  });

  it("computes edit stats from the result diff", () => {
    const result: NormalizedBlock = {
      type: "tool-result",
      toolCallId: "c1",
      details: { diff: "--- a/f.ts\n+++ b/f.ts\n@@ -1,2 +1,2 @@\n-old\n+new\n+extra\n" }
    };
    const summary = summarizeTool(call("edit", { path: "f.ts", edits: [] }), result);
    expect(summary.doneVerb).toBe("Edited");
    expect(summary.additions).toBe(2);
    expect(summary.deletions).toBe(1);
  });

  it("falls back to edit arguments when no diff is present", () => {
    const summary = summarizeTool(call("edit", { path: "f.ts", edits: [{ oldText: "a\nb", newText: "a\nb\nc" }] }));
    expect(summary.additions).toBe(3);
    expect(summary.deletions).toBe(2);
  });

  it("summarizes write calls with a line count", () => {
    const summary = summarizeTool(call("write", { path: "src/new.ts", content: "one\ntwo\n" }));
    expect(summary.subject).toBe("src/new.ts");
    expect(summary.additions).toBe(2);
  });

  it("handles streamed arguments still arriving as a string", () => {
    const summary = summarizeTool(call("read", "{\"path\": \"src/x.ts\"}"));
    expect(summary.subject).toBe("src/x.ts");
  });

  it("falls back gracefully for unknown tools", () => {
    const summary = summarizeTool(call("mcp_deploy", {}));
    expect(summary.doneVerb).toBe("mcp_deploy");
  });
});

describe("diffStats", () => {
  it("ignores file header lines", () => {
    expect(diffStats("+++ b/a\n--- a/a\n+x\n-y")).toEqual({ additions: 1, deletions: 1 });
  });
  it("returns undefined without changes", () => {
    expect(diffStats("no diff")).toBeUndefined();
    expect(diffStats(undefined)).toBeUndefined();
  });
});

describe("tool catalogue", () => {
  const builtin = (name: string, available = true): ToolCatalogEntry =>
    ({ name, description: `${name} tool`, source: { kind: "builtin" }, available });
  const fromPackage = (name: string, packageId: string): ToolCatalogEntry =>
    ({ name, description: "", source: { kind: "package", packageId }, available: true });

  it("puts built-ins first and sorts packages, keeping unattributed tools in Other", () => {
    const groups = groupTools([
      fromPackage("web_search", "npm:pi-web-access"),
      builtin("read"),
      { name: "mystery", description: "", source: { kind: "package" }, available: true },
      fromPackage("delegate", "npm:pi-subagents"),
      builtin("bash")
    ]);
    expect(groups.map((group) => group.id)).toEqual(["builtin", "npm:pi-subagents", "npm:pi-web-access", "other"]);
    expect(groups[0].tools.map((tool) => tool.name)).toEqual(["bash", "read"]);
    expect(groups[3].label).toBe("Other");
  });

  it("drops disabled names whose tool no longer exists, but keeps them when the catalogue is empty", () => {
    const catalog = [builtin("read"), builtin("bash")];
    expect(pruneDisabledTools(["bash", "gone"], catalog)).toEqual(["bash"]);
    // An empty catalogue means "not loaded yet", not "nothing installed" — never prune then.
    expect(pruneDisabledTools(["bash", "gone"], [])).toEqual(["bash", "gone"]);
  });

  it("treats an availability change as a different catalogue so Settings re-renders", () => {
    expect(sameToolCatalog([builtin("grep")], [builtin("grep")])).toBe(true);
    expect(sameToolCatalog([builtin("grep")], [builtin("grep", false)])).toBe(false);
    expect(sameToolCatalog([builtin("grep")], [])).toBe(false);
  });
});
