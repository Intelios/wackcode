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
