import { describe, expect, it } from "vitest";
import { applyHistoryPreviews } from "./history-previews";
import type { SessionSnapshot, TaskRuntime } from "./types";

function fixture(): TaskRuntime {
  return {
    historyId: "cold-1",
    partial: { id: "partial", role: "assistant", blocks: [{ type: "text", text: "Live" }] },
    snapshot: {
      rev: 0, sessionId: "saved", messages: [
        { id: "u", role: "user", blocks: [{ type: "text", text: "Look" }, { type: "image", imageId: "i1", mimeType: "image/png" }] },
        { id: "r", role: "tool", blocks: [{ type: "tool-result", toolCallId: "call", text: "Captured", images: [{ imageId: "i2" }, { imageId: "i3" }] }] },
        { id: "a", role: "assistant", blocks: [{ type: "text", text: "Answer" }] },
      ], modelSwitches: [], runTimings: [], stats: { tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }, cost: 0 },
      thinkingLevel: "off", availableThinkingLevels: [], tools: [], activeTools: [],
    } satisfies SessionSnapshot,
  };
}

describe("cold history previews", () => {
  it("hydrates attachments and tool images without changing unrelated identities or revisions", () => {
    const before = fixture();
    const after = applyHistoryPreviews(before, { historyId: "cold-1", previews: [{ imageId: "i1", thumbnail: "data:attachment" }, { imageId: "i2", thumbnail: "data:screenshot" }] })!;
    expect(after.snapshot!.messages[0].blocks[1].thumbnail).toBe("data:attachment");
    expect(after.snapshot!.messages[1].blocks[0].images![0].thumbnail).toBe("data:screenshot");
    expect(after.snapshot!.messages[1].blocks[0].images![1]).toBe(before.snapshot!.messages[1].blocks[0].images![1]);
    expect(after.snapshot!.messages[0].blocks[0]).toBe(before.snapshot!.messages[0].blocks[0]);
    expect(after.snapshot!.messages[2]).toBe(before.snapshot!.messages[2]);
    expect(after.snapshot!.stats).toBe(before.snapshot!.stats);
    expect(after.snapshot!.rev).toBe(0);
    expect(after.partial).toBe(before.partial);
    expect(before.snapshot!.messages[0].blocks[1]).not.toHaveProperty("thumbnail");
  });

  it("ignores late generations, missing images and live-worker takeovers", () => {
    const before = fixture();
    expect(applyHistoryPreviews(before, { historyId: "old", previews: [{ imageId: "i1", thumbnail: "late" }], done: true })).toBe(before);
    expect(applyHistoryPreviews(before, { historyId: "cold-1", previews: [{ imageId: "removed", thumbnail: "gone" }] })).toBe(before);
    const live = { ...before, historyId: undefined };
    expect(applyHistoryPreviews(live, { historyId: "cold-1", previews: [{ imageId: "i1", thumbnail: "late" }], done: true })).toBe(live);
    expect(applyHistoryPreviews(undefined, { historyId: "cold-1", previews: [] })).toBeUndefined();
  });

  it("settles unreadable and unfinished previews without retiring readable text or prepared images", () => {
    const prepared = applyHistoryPreviews(fixture(), { historyId: "cold-1", previews: [{ imageId: "i2", thumbnail: "data:screenshot" }] })!;
    const after = applyHistoryPreviews(prepared, { historyId: "cold-1", previews: [{ imageId: "i1", failed: true }], done: true })!;
    expect(after.historyId).toBeUndefined();
    expect(after.snapshot!.messages[0].blocks[1].failed).toBe(true);
    expect(after.snapshot!.messages[1].blocks[0].images).toEqual([{ imageId: "i2", thumbnail: "data:screenshot" }, { imageId: "i3", failed: true }]);
    expect(after.snapshot!.messages[2]).toBe(prepared.snapshot!.messages[2]);
    expect(applyHistoryPreviews(after, { historyId: "cold-1", previews: [{ imageId: "i1", thumbnail: "late" }] })).toBe(after);
  });
});
