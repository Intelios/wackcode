import { describe, expect, it } from "vitest";
import { diffStats, editStats, summarizeTool } from "./tool-utils";
import type { NormalizedBlock } from "./types";

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
