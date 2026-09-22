import { describe, expect, it } from "vitest";
import { resolveRunTimings, RUN_TIMING_ENTRY_TYPE, RUN_TIMING_VERSION } from "./run-timing.js";

function entry(userMessageEntryId: string, durationMs: number) {
  return {
    type: "custom",
    customType: RUN_TIMING_ENTRY_TYPE,
    data: {
      version: RUN_TIMING_VERSION,
      runId: `run-${userMessageEntryId}`,
      userMessageEntryId,
      startedAt: 1_000,
      endedAt: 1_000 + durationMs,
      durationMs
    }
  };
}

describe("resolveRunTimings", () => {
  it("maps saved durations to user message ids in branch order", () => {
    expect(resolveRunTimings(
      [entry("user-a", 12_000), entry("user-b", 34_000)],
      ["user-a", "user-b"],
      ["user-message-a", "user-message-b"]
    )).toEqual([
      { userMessageId: "user-message-a", durationMs: 12_000 },
      { userMessageId: "user-message-b", durationMs: 34_000 }
    ]);
  });

  it("omits records for messages removed from the visible context by compaction", () => {
    expect(resolveRunTimings(
      [entry("summarized-user", 12_000), entry("kept-user", 34_000)],
      ["kept-user"],
      ["visible-user-message"]
    )).toEqual([{ userMessageId: "visible-user-message", durationMs: 34_000 }]);
  });

  it("ignores malformed and duplicate timing entries", () => {
    const duplicate = entry("user-a", 12_000);
    expect(resolveRunTimings(
      [duplicate, duplicate, { type: "custom", customType: RUN_TIMING_ENTRY_TYPE, data: { durationMs: -1 } }],
      ["user-a"],
      ["user-message-a"]
    )).toEqual([{ userMessageId: "user-message-a", durationMs: 12_000 }]);
  });
});
