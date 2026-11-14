import { describe, expect, it } from "vitest";
import { layoutTranscript } from "./explore-utils";
import { summarizeTurnFileChanges } from "./turn-changes";
import type { NormalizedBlock, NormalizedMessage } from "./types";

const user = (id = "u1"): NormalizedMessage => ({ id, role: "user", blocks: [{ type: "text", text: "Update files" }] });
const editCall = (id: string, path: string): NormalizedBlock => ({ type: "tool-call", toolName: "edit", toolCallId: id, arguments: { path } });
const writeCall = (id: string, path: string, content = "line1\nline2"): NormalizedBlock => ({ type: "tool-call", toolName: "write", toolCallId: id, arguments: { path, content } });
const assistant = (id: string, blocks: NormalizedBlock[]): NormalizedMessage => ({ id, role: "assistant", blocks });
const toolRes = (id: string, extra: Partial<NormalizedBlock> = {}): NormalizedMessage => ({
  id: `res-${id}`,
  role: "tool",
  blocks: [{ type: "tool-result", toolCallId: id, text: "ok", ...extra }]
});

function summarize(messages: NormalizedMessage[], foldedByWorkTurn = false, grouping = true) {
  const results = new Map<string, NormalizedBlock>();
  for (const m of messages) {
    for (const b of m.blocks) {
      if (b.type === "tool-result" && b.toolCallId) results.set(b.toolCallId, b);
    }
  }
  const layout = layoutTranscript(messages, undefined, { grouping, results });
  return summarizeTurnFileChanges(messages, 1, messages.length, results, { layout, foldedByWorkTurn });
}

describe("summarizeTurnFileChanges", () => {
  it("suppresses a single standalone bare edit row unless folded behind WorkTurn or inside a group", () => {
    const msgs = [user(), assistant("a1", [editCall("e1", "src/App.tsx")]), toolRes("e1", { details: { diff: "-a\n+b" } })];
    expect(summarize(msgs, false)).toBeUndefined();
    const folded = summarize(msgs, true)!;
    expect(folded.heading).toBe("Edited 1 file");
    expect(folded.additions).toBe(1);
    expect(folded.deletions).toBe(1);
  });

  it("shows Edited 1 file when inside a collapsed work group even when unfolded at the turn level", () => {
    const msgs = [
      user(),
      assistant("a1", [
        { type: "tool-call", toolName: "read", toolCallId: "r1", arguments: { path: "src/App.tsx" } },
        editCall("e1", "src/App.tsx")
      ]),
      toolRes("r1"),
      toolRes("e1", { details: { diff: "-old\n+new1\n+new2" } })
    ];
    const summary = summarize(msgs, false, true)!;
    expect(summary.heading).toBe("Edited 1 file");
    expect(summary.additions).toBe(2);
    expect(summary.deletions).toBe(1);
  });

  it("accumulates multi-file edits, distinguishes Created/Edited/Changed, skips failed edits, and computes folder hints", () => {
    const msgs = [
      user(),
      assistant("a1", [
        writeCall("w1", "packages/web/src/Button.tsx", "a\nb\nc"),
        editCall("e1", "packages/cli/src/Button.tsx"),
        editCall("e2", "packages/cli/src/Button.tsx"),
        editCall("bad", "src/broken.ts")
      ]),
      toolRes("w1", { details: { created: true } }),
      toolRes("e1", { details: { diff: "-x\n+y\n+z" } }),
      toolRes("e2", { details: { diff: "-z" } }),
      toolRes("bad", { isError: true, text: "failed" })
    ];
    const summary = summarize(msgs, false)!;
    expect(summary.heading).toBe("Changed 2 files");
    expect(summary.files.map((f) => ({ path: f.path, folderHint: f.folderHint, kind: f.kind, add: f.additions, del: f.deletions }))).toEqual([
      { path: "packages/web/src/Button.tsx", folderHint: "web/src", kind: "create", add: 3, del: 0 },
      { path: "packages/cli/src/Button.tsx", folderHint: "cli/src", kind: "edit", add: 2, del: 2 }
    ]);
  });
});
