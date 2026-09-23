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
    expect(clock.live({ content: [thinking("Hmm")] })).toEqual([null]);

    clock.update({ type: "thinking_end", contentIndex: 0 }, 4_500);
    clock.update({ type: "text_start", contentIndex: 1 }, 4_600);
    expect(clock.live({ content: [thinking("Hmm"), text("A")] })).toEqual([3_500]);

    const final = { content: [thinking("Hmm"), text("Answer")] };
    clock.end(final, 5_000);
    expect(clock.durations(final)).toEqual([3_500]);
    expect(clock.takeUnsaved().get(final)).toEqual([3_500]);
    expect(clock.takeUnsaved().size).toBe(0);
  });

  it("ends an open block when the model starts writing something else, or the message ends", () => {
    const clock = new ThinkingClock();
    clock.begin();
    clock.update({ type: "thinking_delta", contentIndex: 0 }, 1_000);
    clock.update({ type: "toolcall_start", contentIndex: 1 }, 1_800);
    clock.update({ type: "thinking_start", contentIndex: 2 }, 2_000);
    const final = { content: [thinking("a"), { type: "toolCall" }, thinking("b")] };
    clock.end(final, 2_250);
    expect(clock.durations(final)).toEqual([800, 250]);
  });

  it("starts afresh for every message and keeps nothing for one that never reasoned", () => {
    const clock = new ThinkingClock();
    clock.begin();
    clock.update({ type: "thinking_start", contentIndex: 0 }, 1_000);
    clock.begin();
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
