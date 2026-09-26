import { describe, expect, it } from "vitest";
import { diffMessages, sameModelSwitches, samePlanState, sameRunTimings, sameStats, sameTodoState, sameTurn, sameVersions } from "./delta.js";
import type { NormalizedMessage } from "./protocol.js";

function message(id: string, text = id): NormalizedMessage {
  return { id, role: "assistant", blocks: [{ type: "text", text }] };
}

describe("diffMessages", () => {
  it("reports nothing for identical lists, reusing the same objects", () => {
    const a = message("a");
    const b = message("b");
    expect(diffMessages([a, b], [a, b])).toEqual({ upserts: [], removed: [] });
  });

  it("appends new messages and removes departed ids", () => {
    const a = message("a");
    const b = message("b");
    const c = message("c");
    const diff = diffMessages([a, b], [a, c]);
    expect(diff).not.toBeUndefined();
    expect(diff?.upserts).toEqual([c]);
    expect(diff?.removed).toEqual(["b"]);
  });

  it("reports a changed message (new object, same id) as an in-place upsert", () => {
    const a1 = message("a", "before");
    const a2 = message("a", "after");
    const b = message("b");
    const diff = diffMessages([a1, b], [a2, b]);
    expect(diff?.upserts).toEqual([a2]);
    expect(diff?.removed).toEqual([]);
  });

  it("treats an id change as remove-then-append when the message sits at the tail", () => {
    // Pi assigns the entry id just after a message settles, so the same logical message can
    // change identity from a positional id to the real one.
    const a = message("a");
    const positional = message("assistant-123-1", "streaming");
    const settled = message("entry-1", "streaming");
    const diff = diffMessages([a, positional], [a, settled]);
    expect(diff?.removed).toEqual(["assistant-123-1"]);
    expect(diff?.upserts).toEqual([settled]);
  });

  it("falls back to undefined when the diff cannot reproduce the new order", () => {
    // A mid-list insertion with no removals is not expressible as remove + replace-or-append.
    const a = message("a");
    const b = message("b");
    const mid = message("mid");
    expect(diffMessages([a, b], [a, mid, b])).toBeUndefined();
  });

  it("expresses a suffix truncation followed by appends, as a retry produces", () => {
    const u1 = message("u1");
    const a1 = message("a1");
    const u2 = message("u2");
    const a2 = message("a2");
    const u2v2 = message("u2v2");
    const a2v2 = message("a2v2");
    const diff = diffMessages([u1, a1, u2, a2], [u1, a1, u2v2, a2v2]);
    expect(diff?.removed).toEqual(["u2", "a2"]);
    expect(diff?.upserts).toEqual([u2v2, a2v2]);
  });

  it("rejects lists with duplicated ids rather than guessing", () => {
    const a = message("a");
    const other = message("a", "other");
    expect(diffMessages([a], [a, other])).toBeUndefined();
  });
});

describe("snapshot scalar equality", () => {
  it("sameModelSwitches compares positions and both model references", () => {
    const switches = [{
      id: "s", at: 2,
      from: { providerId: "p", modelId: "old" },
      to: { providerId: "p", modelId: "new" }
    }];
    expect(sameModelSwitches(switches, [{ ...switches[0], from: { ...switches[0].from }, to: { ...switches[0].to } }])).toBe(true);
    expect(sameModelSwitches(switches, [{ ...switches[0], at: 3 }])).toBe(false);
  });

  it("samePlanState compares mode, phase, and plan", () => {
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "p" }, { mode: "plan", phase: "ready", plan: "p" })).toBe(true);
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "p" }, { mode: "plan", phase: "ready", plan: "q" })).toBe(false);
    expect(samePlanState(undefined, { mode: "build", phase: "planning" })).toBe(false);
  });

  it("sameTodoState compares task lists field by field", () => {
    const tasks = [{ id: 1, subject: "Ship", status: "in_progress" as const }];
    expect(sameTodoState({ tasks }, { tasks: [{ ...tasks[0] }] })).toBe(true);
    expect(sameTodoState({ tasks }, { tasks: [{ ...tasks[0], blockedBy: [2] }] })).toBe(false);
    expect(sameTodoState({ tasks }, { tasks: [...tasks, { id: 2, subject: "More", status: "pending" as const }] })).toBe(false);
  });

  it("sameVersions compares index, total, links, and group", () => {
    const base = { index: 0, total: 2, group: "versions:x", next: "b" };
    expect(sameVersions(base, { ...base })).toBe(true);
    expect(sameVersions(base, { ...base, index: 1, previous: "a" })).toBe(false);
    expect(sameVersions(undefined, { ...base })).toBe(false);
  });

  it("sameTurn compares entry ids and the after checkpoint", () => {
    const base = { userEntryId: "u", endEntryId: "e", after: { id: "c" } };
    expect(sameTurn(base, { ...base })).toBe(true);
    expect(sameTurn(base, { ...base, endEntryId: "e2" })).toBe(false);
    expect(sameTurn({ ...base, after: undefined }, { ...base })).toBe(false);
  });

  it("sameRunTimings compares per-message durations", () => {
    expect(sameRunTimings([{ userMessageId: "u", durationMs: 5 }], [{ userMessageId: "u", durationMs: 5 }])).toBe(true);
    expect(sameRunTimings([{ userMessageId: "u", durationMs: 5 }], [{ userMessageId: "u", durationMs: 6 }])).toBe(false);
    expect(sameRunTimings([], [{ userMessageId: "u", durationMs: 5 }])).toBe(false);
  });

  it("sameStats compares token counters and context usage", () => {
    const stats = {
      tokens: { input: 1, output: 2, cacheRead: 3, cacheWrite: 4, total: 10 },
      cost: 0.5,
      contextUsage: { tokens: 10, contextWindow: 100, percent: 0.1 },
      contextBreakdown: { entries: [{ id: "user" as const, tokens: 5 }], cacheHitRate: 0.5 }
    };
    expect(sameStats(stats, { ...stats })).toBe(true);
    expect(sameStats(stats, { ...stats, tokens: { ...stats.tokens, total: 11 } })).toBe(false);
    expect(sameStats(stats, { ...stats, contextUsage: { tokens: 11, contextWindow: 100, percent: 0.11 } })).toBe(false);
    expect(sameStats(stats, { ...stats, contextBreakdown: { entries: [{ id: "user", tokens: 6 }], cacheHitRate: 0.5 } })).toBe(false);
  });
});
