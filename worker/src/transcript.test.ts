import { describe, expect, it, vi } from "vitest";
import { normalizeMessage } from "./message-normalization.js";
import type { ThinkingDurations } from "./run-timing.js";
import { ThinkingClock } from "./thinking-timing.js";
import { transcriptMessages, type CachedMessage } from "./transcript.js";
import { buildTreeIndex, type EntryLike } from "./tree.js";

const thinking = (text: string) => ({ type: "thinking", thinking: text });
const normalize = (raw: unknown, position: number, durations?: ThinkingDurations, starts?: Array<number | null>) =>
  normalizeMessage(raw, position, durations, () => ({ type: "image" }), starts);

describe("transcriptMessages live thinking", () => {
  it("supplies starts only to the streaming row and rebuilds it with duration only when finished", () => {
    const clock = new ThinkingClock();
    const previous = { role: "assistant", content: [thinking("Previous")] };
    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 0 }, 100);
    clock.end(previous, 500);
    const unclocked = { role: "assistant", content: [thinking("Old"), { type: "text", text: "Answer" }, thinking("Also old")] };
    const streaming = { role: "assistant", content: [thinking("Closed"), { type: "text", text: "More" }, thinking("Open")] };
    const path: EntryLike[] = [
      { type: "message", id: "previous", parentId: null, message: previous },
      { type: "message", id: "unclocked", parentId: "previous", message: unclocked }
    ];
    const index = buildTreeIndex(path);
    const cache = new WeakMap<object, CachedMessage>();
    const starts = vi.fn((raw: unknown) => clock.liveStarts(raw));
    let streamingMessage: unknown = streaming;
    const snapshot = () => transcriptMessages([previous, unclocked, streaming], path, index, new Map(), {
      normalize, cache, streamingMessage,
      durations: (raw) => raw === streamingMessage ? clock.live(raw) : clock.durations(raw),
      starts
    });

    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 0 }, 1_000);
    clock.update({ type: "thinking_end", contentIndex: 0 }, 1_250);
    clock.update({ type: "thinking_start", contentIndex: 2 }, 2_000);
    const live = snapshot();
    expect(starts).toHaveBeenCalledExactlyOnceWith(streaming);
    expect(live[0].blocks).toEqual([{ type: "thinking", text: "Previous", durationMs: 400 }]);
    expect(live[1].blocks).toEqual([
      { type: "thinking", text: "Old" }, { type: "text", text: "Answer" }, { type: "thinking", text: "Also old" }
    ]);
    expect(live[2].blocks).toEqual([
      { type: "thinking", text: "Closed", durationMs: 250 },
      { type: "text", text: "More" },
      { type: "thinking", text: "Open", startedAt: 2_000 }
    ]);
    expect(cache.has(streaming)).toBe(false);

    clock.update({ type: "thinking_delta", contentIndex: 2 }, 2_100);
    const next = snapshot();
    expect(next[0]).toBe(live[0]);
    expect(next[1]).toBe(live[1]);
    expect(next[2]).not.toBe(live[2]);
    expect(next[2].blocks[2].startedAt).toBe(2_000);

    clock.end(streaming, 2_500);
    streamingMessage = undefined;
    path.push({ type: "message", id: "finished", parentId: "unclocked", message: streaming });
    starts.mockClear();
    const finished = snapshot();
    expect(starts).not.toHaveBeenCalled();
    expect(finished[2].blocks).toEqual([
      { type: "thinking", text: "Closed", durationMs: 250 },
      { type: "text", text: "More" },
      { type: "thinking", text: "Open", durationMs: 500 }
    ]);
    expect(finished.flatMap((message) => message.blocks).every((block) => !Object.hasOwn(block, "startedAt"))).toBe(true);
    expect(snapshot()[2]).toBe(finished[2]);
    // The earlier snapshot remains immutable even after the same raw message finishes.
    expect(live[2].blocks[2].startedAt).toBe(2_000);
  });

  it("restores saved durations without invoking a live start callback", () => {
    const raw = { role: "assistant", content: [thinking("Saved")] };
    const path: EntryLike[] = [{ type: "message", id: "saved", parentId: null, message: raw }];
    const starts = vi.fn(() => [9_999]);
    const messages = transcriptMessages([raw], path, buildTreeIndex(path), new Map([["saved", [600]]]), {
      normalize, cache: new WeakMap(), starts
    });
    expect(starts).not.toHaveBeenCalled();
    expect(messages[0].blocks).toEqual([{ type: "thinking", text: "Saved", durationMs: 600 }]);
  });
});

describe("transcript history across compaction", () => {
  it("keeps entry identities, prompt anchors and timing annotations when model context drops a user", () => {
    const user = { role: "user", content: "Keep working", timestamp: 1_000 };
    const assistant = { role: "assistant", content: [{ type: "text", text: "Earlier work" }] };
    const path: EntryLike[] = [
      { type: "message", id: "u", parentId: null, message: user },
      { type: "message", id: "a", parentId: "u", message: assistant }
    ];
    const cache = new WeakMap<object, CachedMessage>();
    const before = transcriptMessages([user, assistant], path, buildTreeIndex(path), new Map(), { normalize, cache });
    const compaction = { type: "compaction", id: "c", parentId: "a", timestamp: "2026-10-06T10:00:00Z", summary: "Continue the work", firstKeptEntryId: "a", tokensBefore: 30_000 };
    path.push(compaction);
    const compactionTokensAfter = vi.fn(() => 8_000);
    const after = () => transcriptMessages([assistant], path, buildTreeIndex(path), new Map(), { normalize, cache, compactionTokensAfter });
    const compacted = after();
    expect(compacted.map((message) => message.id)).toEqual(["u", "a", "c"]);
    expect(compacted[0]).toBe(before[0]);
    expect(compacted[1]).toBe(before[1]);
    expect(compacted[1].turn?.userEntryId).toBe("u");
    expect(compacted[2].compaction).toEqual({ summary: "Continue the work", tokensBefore: 30_000, estimatedTokensAfter: 8_000 });
    expect(after()[2]).toBe(compacted[2]);
    expect(compactionTokensAfter).toHaveBeenCalledExactlyOnceWith(compaction);
  });

  it("applies context edits and omissions throughout the branch without duplicating projected clones", () => {
    const user = { role: "user", content: "Original", timestamp: 1_000 };
    const failed = { role: "assistant", content: [{ type: "text", text: "Discarded attempt" }] };
    const path: EntryLike[] = [
      { type: "message", id: "u", parentId: null, message: user },
      { type: "message", id: "failed", parentId: "u", message: failed },
      { type: "context_edit", id: "omit", parentId: "failed", targetId: "failed", replacement: null } as EntryLike,
      { type: "context_edit", id: "edit", parentId: "omit", targetId: "u", replacement: { content: "Edited" } } as EntryLike
    ];
    const cache = new WeakMap<object, CachedMessage>();
    const render = () => transcriptMessages([{ ...user, content: "Edited" }], path, buildTreeIndex(path), new Map(), { normalize, cache });
    const messages = render();
    expect(messages).toEqual([expect.objectContaining({ id: "u", entryId: "u", blocks: [{ type: "text", text: "Edited" }] })]);
    expect(render()[0]).toBe(messages[0]);
    path.push({ type: "context_edit", id: "edit2", parentId: "edit", targetId: "u", replacement: { content: "Edited again" } } as EntryLike);
    expect(render()[0].blocks).toEqual([{ type: "text", text: "Edited again" }]);
    expect(messages[0].blocks).toEqual([{ type: "text", text: "Edited" }]);
  });
});
