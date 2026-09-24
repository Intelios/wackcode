import { describe, expect, it } from "vitest";
import { exploreActivity, exploreCounts, exploreLabel, layoutTranscript, type TranscriptSlot } from "./explore-utils";
import type { NormalizedBlock, NormalizedMessage } from "./types";

const read = (id: string, path = `${id}.ts`): NormalizedBlock => ({ type: "tool-call", toolName: "read", toolCallId: id, arguments: { path } });
const grep = (id: string): NormalizedBlock => ({ type: "tool-call", toolName: "grep", toolCallId: id, arguments: { pattern: "x" } });
const edit = (id: string): NormalizedBlock => ({ type: "tool-call", toolName: "edit", toolCallId: id, arguments: { path: "a.ts" } });
const text = (value: string): NormalizedBlock => ({ type: "text", text: value });
const thinking = (value: string): NormalizedBlock => ({ type: "thinking", text: value, durationMs: 100 });
const assistant = (id: string, blocks: NormalizedBlock[], extra: Partial<NormalizedMessage> = {}): NormalizedMessage => ({ id, role: "assistant", timestamp: Number(id.replace(/\D/g, "")) || undefined, blocks, ...extra });
const tool = (id: string, ...callIds: string[]): NormalizedMessage => ({ id, role: "tool", blocks: callIds.map((callId) => ({ type: "tool-result", toolCallId: callId, text: "ok" })) });
const user = (id: string): NormalizedMessage => ({ id, role: "user", blocks: [text("Go")] });

/** A slot as `b<index>` or `[call ids and t for thinking]`, for compact expectations. */
function describeSlots(slots: TranscriptSlot[] | undefined): string[] | undefined {
  return slots?.map((slot) => slot.type === "block"
    ? `b${slot.index}`
    : `[${slot.group.items.map((item) => item.block.toolCallId ?? "t").join(" ")}]`);
}

function layout(messages: NormalizedMessage[], partial?: NormalizedMessage, liveMessageId?: string, grouping = true) {
  const result = layoutTranscript(messages, partial, { grouping, liveMessageId });
  return {
    result,
    slots: Object.fromEntries([...result.messages].map(([id, slots]) => [id, describeSlots(slots)])),
    partial: describeSlots(result.partial)
  };
}

describe("layoutTranscript", () => {
  it("folds a run spanning messages into the message holding its first call", () => {
    const { slots } = layout([
      user("u1"),
      assistant("a1", [text("I'll look around."), read("r1")]),
      tool("t1", "r1"),
      assistant("a2", [read("r2")]),
      tool("t2", "r2"),
      assistant("a3", [grep("g1"), grep("g2")]),
      tool("t3", "g1", "g2"),
      assistant("a4", [text("Found it."), edit("e1")])
    ]);
    expect(slots).toEqual({ a1: ["b0", "[r1 r2 g1 g2]"], a2: [], a3: [], a4: ["b0", "b1"] });
  });

  it("is broken by prose, other tools, user messages and failed messages", () => {
    expect(layout([assistant("a1", [read("r1"), text("Hmm."), read("r2")])]).slots).toEqual({ a1: ["b0", "b1", "b2"] });
    expect(layout([assistant("a1", [read("r1"), edit("e1"), read("r2"), read("r3")])]).slots).toEqual({ a1: ["b0", "b1", "[r2 r3]"] });
    expect(layout([assistant("a1", [read("r1")]), user("u2"), assistant("a2", [read("r2")])]).slots).toEqual({ a1: ["b0"], a2: ["b0"] });
    expect(layout([assistant("a1", [read("r1")], { stopReason: "aborted" }), assistant("a2", [read("r2")])]).slots).toEqual({ a1: ["b0"], a2: ["b0"] });
  });

  it("leaves a lone call as its own row", () => {
    expect(layout([assistant("a1", [text("One look."), read("r1")]), tool("t1", "r1"), assistant("a2", [text("Done.")])]).slots)
      .toEqual({ a1: ["b0", "b1"], a2: ["b0"] });
  });

  it("takes in reasoning between calls, but not before the first or after the last", () => {
    const { slots } = layout([
      assistant("a1", [thinking("Where to start"), read("r1")]),
      assistant("a2", [thinking("Now the tests"), read("r2")]),
      assistant("a3", [thinking("Ready"), edit("e1")])
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

  it("folds nothing when switched off", () => {
    expect(layout([assistant("a1", [read("r1"), read("r2")])], undefined, undefined, false).slots).toEqual({ a1: ["b0", "b1"] });
  });

  it("folds read-only shell commands with the rest, but not ones that might write", () => {
    const bash = (id: string, command: string): NormalizedBlock => ({ type: "tool-call", toolName: "bash", toolCallId: id, arguments: { command } });
    expect(layout([assistant("a1", [bash("b1", "ls src"), bash("b2", "grep -rn x src | head")])]).slots).toEqual({ a1: ["[b1 b2]"] });
    expect(layout([assistant("a1", [bash("b1", "ls src"), bash("b2", "pnpm test"), read("r1")])]).slots).toEqual({ a1: ["b0", "b1", "b2"] });
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
    expect(counts).toEqual({ files: 2, searches: 1, lists: 1, commands: 1, failed: 0 });
    expect(exploreLabel(counts)).toBe("2 files, 1 search, 1 list, 1 command");
    expect(exploreLabel({ files: 1, searches: 2, lists: 0, commands: 0, failed: 0 })).toBe("1 file, 2 searches");
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
});
