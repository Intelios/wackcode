import { describe, expect, it } from "vitest";
import {
  coalesceWorkItems,
  exploreActivity,
  exploreCounts,
  exploreLabel,
  exploreLabelParts,
  exploreLiveItems,
  exploreMarks,
  flyId,
  formatReadRanges,
  isBridgingNarration,
  layoutTranscript,
  liveTailWindow,
  settledGroup,
  summarizeWorkGroup,
  tailGroupKey,
  workSummaryText,
  type TranscriptSlot
} from "./explore-utils";
import type { NormalizedBlock, NormalizedMessage } from "./types";

const read = (id: string, path = `${id}.ts`, extra?: Record<string, unknown>): NormalizedBlock =>
  ({ type: "tool-call", toolName: "read", toolCallId: id, arguments: { path, ...extra } });
const grep = (id: string): NormalizedBlock => ({ type: "tool-call", toolName: "grep", toolCallId: id, arguments: { pattern: "x" } });
const edit = (id: string, path = "a.ts"): NormalizedBlock => ({ type: "tool-call", toolName: "edit", toolCallId: id, arguments: { path } });
const write = (id: string, path = "new.ts", content = "a\nb"): NormalizedBlock => ({ type: "tool-call", toolName: "write", toolCallId: id, arguments: { path, content } });
const bash = (id: string, command: string): NormalizedBlock => ({ type: "tool-call", toolName: "bash", toolCallId: id, arguments: { command } });
const text = (value: string): NormalizedBlock => ({ type: "text", text: value });
const thinking = (value: string): NormalizedBlock => ({ type: "thinking", text: value, durationMs: 100 });
const assistant = (id: string, blocks: NormalizedBlock[], extra: Partial<NormalizedMessage> = {}): NormalizedMessage => ({ id, role: "assistant", timestamp: Number(id.replace(/\D/g, "")) || undefined, blocks, ...extra });
const tool = (id: string, ...callIds: string[]): NormalizedMessage => ({ id, role: "tool", blocks: callIds.map((callId) => ({ type: "tool-result", toolCallId: callId, text: "ok" })) });
const user = (id: string): NormalizedMessage => ({ id, role: "user", blocks: [text("Go")] });

/** A slot as `b<index>`, `c(id1 id2)` for a coalesced lone file, or `[call ids and notes]`. */
function describeSlots(slots: TranscriptSlot[] | undefined): string[] | undefined {
  return slots?.map((slot) => slot.type === "block"
    ? `b${slot.index}`
    : slot.type === "coalesced"
      ? `c(${slot.step.items.map((item) => item.block.toolCallId ?? "t").join(" ")})`
      : `[${slot.group.items.map((item) => item.block.toolCallId ?? (item.block.type === "text" ? "n" : "t")).join(" ")}]`);
}

function layout(messages: NormalizedMessage[], partial?: NormalizedMessage, liveMessageId?: string, grouping = true, results?: Map<string, NormalizedBlock>) {
  const result = layoutTranscript(messages, partial, { grouping, liveMessageId, results });
  return {
    result,
    slots: Object.fromEntries([...result.messages].map(([id, slots]) => [id, describeSlots(slots)])),
    partial: describeSlots(result.partial)
  };
}

describe("layoutTranscript", () => {
  it("folds a unified run spanning messages into the message holding its first call", () => {
    const { slots } = layout([
      user("u1"),
      assistant("a1", [text("I'll look around."), read("r1")]),
      tool("t1", "r1"),
      assistant("a2", [read("r2")]),
      tool("t2", "r2"),
      assistant("a3", [grep("g1"), edit("e1")]),
      tool("t3", "g1", "e1"),
      assistant("a4", [text("## Outcome\n\nAll fixed."), edit("e2")])
    ]);
    expect(slots).toEqual({ a1: ["b0", "[r1 r2 g1 e1]"], a2: [], a3: [], a4: ["b0", "b1"] });
  });

  it("folds short bridging narration between calls while substantive prose, commits, cards, users and aborted messages break runs", () => {
    expect(layout([assistant("a1", [read("r1"), text("Hmm."), read("r2")])]).slots).toEqual({ a1: ["[r1 n r2]"] });
    expect(layout([assistant("a1", [read("r1"), text("## Summary\n\nFound it."), read("r2"), read("r3")])]).slots).toEqual({ a1: ["b0", "b1", "[r2 r3]"] });
    expect(layout([assistant("a1", [read("r1"), bash("c1", 'git commit -m "wip"'), read("r2"), read("r3")])]).slots).toEqual({ a1: ["b0", "b1", "[r2 r3]"] });
    const subagent: NormalizedBlock = { type: "tool-call", toolName: "subagent", toolCallId: "s1", arguments: { agent: "scout", task: "look" } };
    expect(layout([assistant("a1", [read("r1"), subagent, read("r2"), read("r3")])]).slots).toEqual({ a1: ["b0", "b1", "[r2 r3]"] });
    expect(layout([assistant("a1", [read("r1")]), user("u2"), assistant("a2", [read("r2")])]).slots).toEqual({ a1: ["b0"], a2: ["b0"] });
    expect(layout([assistant("a1", [read("r1")], { stopReason: "aborted" }), assistant("a2", [read("r2")])]).slots).toEqual({ a1: ["b0"], a2: ["b0"] });
  });

  it("keeps a lone call as its own row, and coalesces repeated calls to the same file into a single bare slot", () => {
    expect(layout([assistant("a1", [text("One look."), read("r1")]), tool("t1", "r1"), assistant("a2", [text("Done.")])]).slots)
      .toEqual({ a1: ["b0", "b1"], a2: ["b0"] });
    expect(layout([assistant("a1", [edit("e1", "src/App.tsx"), edit("e2", "src/App.tsx")]), tool("t1", "e1", "e2")]).slots)
      .toEqual({ a1: ["c(e1 e2)"] });
    expect(layout([assistant("a1", [read("r1", "a.ts", { offset: 1, limit: 50 }), read("r2", "a.ts", { offset: 80, limit: 41 })])]).slots)
      .toEqual({ a1: ["c(r1 r2)"] });
  });

  it("takes in reasoning between calls, but not before the first or after the last", () => {
    const { slots } = layout([
      assistant("a1", [thinking("Where to start"), read("r1")]),
      assistant("a2", [thinking("Now the tests"), read("r2")]),
      assistant("a3", [thinking("Ready to commit"), bash("c1", 'git commit -m "done"')])
    ]);
    expect(slots).toEqual({ a1: ["b0", "[r1 t r2]"], a2: [], a3: ["b0", "b1"] });
  });

  it("skips empty text, which would otherwise keep a folded message on screen", () => {
    const { slots } = layout([assistant("a1", [read("r1")]), assistant("a2", [text(" "), read("r2")])]);
    expect(slots).toEqual({ a1: ["[r1 r2]"], a2: [] });
  });

  it("continues a run into the streaming message, whose calls are live", () => {
    const partial = assistant("a2", [read("r2")]);
    const { result, slots, partial: partialSlots } = layout([assistant("a1", [read("r1")]), tool("t1", "r1")], partial, "a1");
    expect(slots).toEqual({ a1: ["[r1 r2]"] });
    expect(partialSlots).toEqual([]);
    const group = result.messages.get("a1")![0];
    expect(group.type === "explore" && group.group.items.map((item) => [item.live, item.streaming])).toEqual([[true, false], [true, true]]);
  });

  it("keeps the group's key when its streaming message is saved", () => {
    const streamed = layout([], assistant("a1", [read("r1"), read("r2")]));
    const saved = layout([assistant("a1", [read("r1"), read("r2")])]);
    const key = (slots?: TranscriptSlot[]) => slots?.[0]?.type === "explore" ? slots[0].group.key : undefined;
    expect(key(streamed.result.partial)).toBe("r1");
    expect(key(saved.result.messages.get("a1"))).toBe("r1");
  });

  it("holds live reasoning after an open run's last call in the group's lane, until the run closes", () => {
    const saved = [assistant("a1", [read("r1"), read("r2")]), tool("t1", "r1", "r2")];
    const open = layout(saved, assistant("a2", [thinking("Next, the tests")]), "a1");
    expect(open.slots).toEqual({ a1: ["[r1 r2]"] });
    expect(open.partial).toEqual([]);
    const group = open.result.messages.get("a1")![0];
    expect(group.type === "explore" && group.group.trailing?.map((item) => item.block.text)).toEqual(["Next, the tests"]);

    // Prose closes the run: the reasoning is back outside, before it.
    const closed = layout(saved, assistant("a2", [thinking("Next, the tests"), text("Found it.")]), "a1");
    expect(closed.partial).toEqual(["b0", "b1"]);
    // So does the run ending: settled history never holds a lane.
    expect(layout([...saved, assistant("a2", [thinking("Done")])]).slots).toEqual({ a1: ["[r1 r2]"], a2: ["b0"] });
    // A lone call has no group, so no lane.
    expect(layout([assistant("a1", [read("r1")])], assistant("a2", [thinking("Hm")]), "a1").partial).toEqual(["b0"]);
  });

  it("names the group a transcript ends with", () => {
    const saved = [user("u1"), assistant("a1", [read("r1"), read("r2")]), tool("t1", "r1", "r2")];
    expect(tailGroupKey(saved, layout(saved).result)).toBe("r1");
    // Folded messages that render nothing are passed over.
    const folded = [...saved, assistant("a2", [read("r3")]), tool("t2", "r3")];
    expect(tailGroupKey(folded, layout(folded).result)).toBe("r1");
    const answered = [...saved, assistant("a2", [text("Done.")])];
    expect(tailGroupKey(answered, layout(answered).result)).toBeUndefined();
    const asked = [...saved, user("u2")];
    expect(tailGroupKey(asked, layout(asked).result)).toBeUndefined();
  });

  it("folds nothing when switched off", () => {
    expect(layout([assistant("a1", [read("r1"), read("r2")])], undefined, undefined, false).slots).toEqual({ a1: ["b0", "b1"] });
  });

  it("folds non-commit shell commands (both read-only and build/test) with edits and reads, while git commits stand alone", () => {
    expect(layout([assistant("a1", [bash("b1", "ls src"), bash("b2", "pnpm test"), read("r1")])]).slots).toEqual({ a1: ["[b1 b2 r1]"] });
    expect(layout([assistant("a1", [bash("b1", "ls src"), bash("c1", 'git commit -m "feat"'), read("r1")])]).slots).toEqual({ a1: ["b0", "b1", "b2"] });
  });
});

describe("coalescing, bridging narration, tiered summaries, and live window", () => {
  const item = (block: NormalizedBlock, key: string, live = false) => ({ block, key, streaming: false, live });

  it("validates bridging narration heuristics", () => {
    expect(isBridgingNarration("Checking the reducer next.")).toBe(true);
    expect(isBridgingNarration("")).toBe(false);
    expect(isBridgingNarration("## Heading\nNext")).toBe(false);
    expect(isBridgingNarration("- item 1\n- item 2")).toBe(false);
    expect(isBridgingNarration("Para 1\n\nPara 2")).toBe(false);
    expect(isBridgingNarration("```ts\nconst x = 1;\n```")).toBe(false);
    expect(isBridgingNarration("a".repeat(221))).toBe(false);
  });

  it("coalesces adjacent same-file reads (merging line ranges) and adjacent edits/writes", () => {
    const results = new Map<string, NormalizedBlock>([
      ["e1", { type: "tool-result", toolCallId: "e1", details: { diff: "-a\n+b\n+c" } }],
      ["e2", { type: "tool-result", toolCallId: "e2", details: { diff: "-d\n-e\n+f" } }],
      ["w1", { type: "tool-result", toolCallId: "w1", details: { created: true } }]
    ]);
    const steps = coalesceWorkItems([
      item(read("r1", "src/App.tsx", { offset: 1, limit: 50 }), "r1"),
      item(read("r2", "src/App.tsx", { offset: 40, limit: 21 }), "r2"),
      item(read("r3", "src/App.tsx", { offset: 100, limit: 21 }), "r3"),
      item(thinking("Check edits"), "t1"),
      item(edit("e1", "src/App.tsx"), "e1"),
      item(edit("e2", "src/App.tsx"), "e2"),
      item(write("w1", "src/New.ts", "x\ny\nz"), "w1")
    ], results);
    expect(steps.map((s) => s.type)).toEqual(["call", "thinking", "call", "call"]);
    const first = steps[0];
    expect(first.type === "call" && formatReadRanges(first.ranges)).toBe("1–60, 100–120");
    const coalescedEdit = steps[2];
    expect(coalescedEdit.type === "call" && { add: coalescedEdit.additions, del: coalescedEdit.deletions }).toEqual({ add: 3, del: 3 });
    const createdWrite = steps[3];
    expect(createdWrite.type === "call" && { created: createdWrite.created, add: createdWrite.additions }).toEqual({ created: true, add: 3 });
  });

  it("summarizes mixed work groups with tier priority, path deduplication, +/− stats, and note count", () => {
    const results = new Map<string, NormalizedBlock>([
      ["w1", { type: "tool-result", toolCallId: "w1", details: { created: true } }],
      ["e1", { type: "tool-result", toolCallId: "e1", details: { diff: "-old\n+new1\n+new2" } }],
      ["e2", { type: "tool-result", toolCallId: "e2", details: { diff: "+new3" } }]
    ]);
    const group = {
      key: "g",
      items: [
        item(read("r1", "a.ts"), "1"),
        item(read("r2", "a.ts"), "2"),
        item(text("Applying fix now."), "3"),
        item(edit("e1", "a.ts"), "4"),
        item(edit("e2", "a.ts"), "5"),
        item(write("w1", "new.ts", "1\n2"), "6"),
        item(bash("b1", "pnpm test"), "7"),
        item({ type: "tool-call", toolName: "web_fetch", toolCallId: "f1", arguments: { url: "https://example.com" } }, "8")
      ]
    };
    const summary = summarizeWorkGroup(group, results);
    expect(summary.isPureExploration).toBe(false);
    expect(summary.additions).toBe(5);
    expect(summary.deletions).toBe(1);
    expect(summary.notes).toBe(1);
    expect(workSummaryText(summary)).toBe("Created 1 file, edited 1 file, and 3 more steps · 1 note");
  });

  it("pluralizes multi-tool live activity and caps live tail window at 3 steps while pinning failed calls", () => {
    const results = new Map<string, NormalizedBlock>([
      ["e1", { type: "tool-result", toolCallId: "e1", isError: true, text: "err" }]
    ]);
    const group = {
      key: "g",
      items: [
        item(edit("e1", "a.ts"), "1", true),
        item(read("r1", "b.ts"), "2", true),
        item(read("r2", "c.ts"), "3", true),
        item(read("r3", "d.ts"), "4", true),
        item(read("r4", "e.ts"), "5", true)
      ]
    };
    expect(exploreActivity(group, results)).toBe("Reading 4 files");
    const tail = liveTailWindow(group, results, 3);
    expect(tail.hiddenCount).toBe(2);
    expect(tail.steps.map((s) => s.key)).toEqual(["e1", "r3", "r4"]);
  });
});

describe("exploration summary", () => {
  const group = (blocks: NormalizedBlock[], live = false) => ({
    key: "k",
    items: blocks.map((block, index) => ({ block, key: String(index), streaming: false, live }))
  });

  it("counts distinct files, searches, listings and other commands", () => {
    const ls: NormalizedBlock = { type: "tool-call", toolName: "ls", toolCallId: "l1", arguments: { path: "src" } };
    const log: NormalizedBlock = { type: "tool-call", toolName: "bash", toolCallId: "c1", arguments: { command: "git log -3" } };
    const counts = exploreCounts(group([read("r1", "a.ts"), read("r2", "a.ts"), read("r3", "b.ts"), grep("g1"), ls, log, thinking("hm")]), new Map());
    expect(counts).toEqual({ files: 2, images: 0, searches: 1, lists: 1, commands: 1, failed: 0 });
    expect(exploreLabel(counts)).toBe("2 files, 1 search, 1 list, 1 command");
    expect(exploreLabel({ files: 1, images: 0, searches: 2, lists: 0, commands: 0, failed: 0 })).toBe("1 file, 2 searches");
  });

  it("counts a read that came back as a picture as an image, not a file", () => {
    const pictured = (id: string): [string, NormalizedBlock] => [id, { type: "tool-result", toolCallId: id, images: [{ imageId: id }] }];
    const results = new Map<string, NormalizedBlock>([pictured("r2"), pictured("r3"), ["r1", { type: "tool-result", toolCallId: "r1", text: "ok" }]]);
    const counts = exploreCounts(group([read("r1", "a.ts"), read("r2", "shot.png"), read("r3", "shot.png"), read("r4", "b.ts")]), results);
    expect(counts).toMatchObject({ files: 2, images: 1 });
    expect(exploreLabel(counts)).toBe("2 files, 1 image");
    expect(exploreLabel({ ...counts, files: 0, images: 2 })).toBe("2 images");
  });

  it("counts failed calls", () => {
    const results = new Map<string, NormalizedBlock>([["r1", { type: "tool-result", toolCallId: "r1", isError: true }]]);
    expect(exploreCounts(group([read("r1"), read("r2")]), results).failed).toBe(1);
  });

  it("names the latest call still in flight, only while live", () => {
    const results = new Map<string, NormalizedBlock>([["r1", { type: "tool-result", toolCallId: "r1" }]]);
    expect(exploreActivity(group([read("r1", "a.ts"), grep("g1")], true), results)).toBe("Searching x");
    expect(exploreActivity(group([read("r1", "a.ts"), grep("g1")], false), results)).toBeUndefined();
    expect(exploreActivity(group([read("r1", "a.ts")], true), results)).toBeUndefined();
  });

  it("splits the label into rolling parts that read the same as the label", () => {
    const counts = { files: 2, images: 1, searches: 0, lists: 1, commands: 0, failed: 1 };
    const parts = exploreLabelParts(counts);
    expect(parts).toEqual([{ key: "files", count: 2, noun: "files" }, { key: "images", count: 1, noun: "image" }, { key: "lists", count: 1, noun: "list" }]);
    expect(parts.map((part) => `${part.count} ${part.noun}`).join(", ")).toBe(exploreLabel(counts));
  });

  it("keeps calls in flight out of the head until they land", () => {
    const results = new Map<string, NormalizedBlock>([["r1", { type: "tool-result", toolCallId: "r1" }]]);
    const live = group([read("r1"), thinking("next"), grep("g1")], true);
    expect(exploreLiveItems(live, results).map((item) => item.block.toolCallId)).toEqual(["g1"]);
    expect(settledGroup(live, results).items.map((item) => item.block.toolCallId ?? "t")).toEqual(["r1", "t"]);
    // Saved history (not live) is all settled, and keeps the very same group object.
    const saved = group([read("r1"), grep("g1")]);
    expect(exploreLiveItems(saved, results)).toEqual([]);
    expect(settledGroup(saved, results)).toBe(saved);
  });

  it("stacks one mark per kind in a fixed order, each flying under its newest item's id", () => {
    const ls: NormalizedBlock = { type: "tool-call", toolName: "ls", toolCallId: "l1", arguments: { path: "src" } };
    const results = new Map<string, NormalizedBlock>([["r2", { type: "tool-result", toolCallId: "r2", images: [{ imageId: "i" }] }]]);
    const marks = exploreMarks(group([grep("g1"), read("r1"), thinking("hm"), read("r2", "shot.png"), ls, read("r3")]), results);
    expect(marks).toEqual([
      { mark: "file", id: "call:r3" }, { mark: "image", id: "call:r2" }, { mark: "search", id: "call:g1" },
      { mark: "list", id: "call:l1" }, { mark: "thought", id: "think:2" }
    ]);
    expect(flyId({ block: thinking("x"), key: "7:1" })).toBe("think:7:1");
    expect(flyId({ block: read("r9"), key: "7:2" })).toBe("call:r9");
  });
});
