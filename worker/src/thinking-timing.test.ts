import { describe, expect, it } from "vitest";
import { RUN_TIMING_ENTRY_TYPE, RUN_TIMING_VERSION } from "./run-timing.js";
import { ThinkingClock, resolveThinkingDurations } from "./thinking-timing.js";

const thinking = (text: string) => ({ type: "thinking", thinking: text });
const text = (value: string) => ({ type: "text", text: value });

describe("ThinkingClock", () => {
  it("times each thinking block from its start to its end, and has none while it is open", () => {
    const clock = new ThinkingClock();
    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 0 }, 1_000);
    clock.update({ type: "thinking_delta", contentIndex: 0 }, 2_000);
    clock.update({ type: "thinking_start", contentIndex: 0 }, 3_000);
    expect(clock.live({ content: [thinking("Hmm")] })).toEqual([null]);
    expect(clock.liveStarts({ content: [thinking("Hmm")] })).toEqual([1_000]);

    clock.update({ type: "thinking_end", contentIndex: 0 }, 4_500);
    clock.update({ type: "text_start", contentIndex: 1 }, 4_600);
    expect(clock.live({ content: [thinking("Hmm"), text("A")] })).toEqual([3_500]);
    expect(clock.liveStarts({ content: [thinking("Hmm"), text("A")] })).toEqual([null]);

    const final = { content: [thinking("Hmm"), text("Answer")] };
    clock.end(final, 5_000);
    expect(clock.durations(final)).toEqual([3_500]);
    expect(clock.liveStarts(final)).toEqual([null]);
    expect(clock.takeUnsaved().get(final)).toEqual([3_500]);
    expect(clock.takeUnsaved().size).toBe(0);
  });

  it("ends an open block when the model starts writing something else, or the message ends", () => {
    const clock = new ThinkingClock();
    clock.begin();
    clock.update({ type: "thinking_delta", contentIndex: 0 }, 1_000);
    expect(clock.liveStarts({ content: [thinking("a")] })).toEqual([1_000]);
    clock.update({ type: "toolcall_start", contentIndex: 1 }, 1_800);
    clock.update({ type: "thinking_start", contentIndex: 2 }, 2_000);
    const final = { content: [thinking("a"), { type: "toolCall" }, thinking("b")] };
    expect(clock.liveStarts(final)).toEqual([null, 2_000]);
    expect(clock.live(final)).toEqual([800, null]);
    clock.end(final, 2_250);
    expect(clock.durations(final)).toEqual([800, 250]);
    expect(clock.liveStarts(final)).toEqual([null, null]);
  });

  it("maps starts to thinking ordinals, never reopens a closed block, and resets for a new message", () => {
    const clock = new ThinkingClock();
    const message = { content: [text("First"), thinking("a"), { type: "toolCall" }, thinking("unclocked"), thinking("b")] };
    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 1 }, 0);
    clock.update({ type: "thinking_start", contentIndex: 4 }, 25);
    const starts = clock.liveStarts(message);
    expect(starts).toEqual([0, null, 25]);
    expect(clock.live(message)).toEqual([null, null, null]);

    clock.update({ type: "thinking_end", contentIndex: 1 }, 40);
    clock.update({ type: "thinking_end", contentIndex: 1 }, 60);
    clock.update({ type: "thinking_delta", contentIndex: 1 }, 70);
    clock.update({ type: "thinking_start", contentIndex: 1 }, 80);
    expect(clock.liveStarts(message)).toEqual([null, null, 25]);
    expect(clock.live(message)).toEqual([40, null, null]);
    // Arrays already emitted are not mutated as blocks close.
    expect(starts).toEqual([0, null, 25]);

    clock.end(message, 100);
    expect(clock.durations(message)).toEqual([40, null, 75]);
    expect(clock.takeUnsaved().get(message)).toEqual([40, null, 75]);
    expect(clock.liveStarts(message)).toEqual([null, null, null]);
    clock.begin();
    clock.update({ type: "thinking_delta", contentIndex: 1 }, 200);
    expect(clock.liveStarts(message)).toEqual([200, null, null]);
    expect(clock.live(message)).toEqual([null, null, null]);
    expect(clock.liveStarts(undefined)).toEqual([]);
    expect(clock.liveStarts({ content: "not blocks" })).toEqual([]);
  });

  it("starts afresh for every message and keeps nothing for one that never reasoned", () => {
    const clock = new ThinkingClock();
    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 0 }, 1_000);
    clock.begin();
    expect(clock.liveStarts({ content: [thinking("old")] })).toEqual([null]);
    clock.update({ type: "text_start", contentIndex: 0 }, 2_000);
    const final = { content: [text("Plain")] };
    clock.end(final, 3_000);
    expect(clock.durations(final)).toBeUndefined();
    expect(clock.takeUnsaved().size).toBe(0);
  });
});

describe("resolveThinkingDurations", () => {
  const timing = (data: Record<string, unknown>) => ({
    type: "custom",
    customType: RUN_TIMING_ENTRY_TYPE,
    data: { version: RUN_TIMING_VERSION, runId: "run", userMessageEntryId: "user", startedAt: 0, endedAt: 1, durationMs: 1, ...data }
  });

  it("reads the durations saved with each run by assistant entry id", () => {
    const durations = resolveThinkingDurations([
      timing({ thinking: { "assistant-a": [1_200], "assistant-b": [null, 300] } }),
      { type: "message", id: "assistant-c" },
      timing({ thinking: { "assistant-c": [4_000] } })
    ]);
    expect(Object.fromEntries(durations)).toEqual({ "assistant-a": [1_200], "assistant-b": [null, 300], "assistant-c": [4_000] });
  });

  it("ignores runs saved without durations and malformed values", () => {
    const durations = resolveThinkingDurations([
      timing({}),
      timing({ thinking: [1] }),
      timing({ thinking: { "assistant-a": [-5], "assistant-b": "slow", "assistant-c": [700] } })
    ]);
    expect(Object.fromEntries(durations)).toEqual({ "assistant-c": [700] });
  });
});
