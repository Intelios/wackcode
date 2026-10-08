import { describe, expect, it } from "vitest";
import { applySnapshotDelta, applySubagentFrame, displayPath, formatRelativeTime, formatRunDuration, formatThinkingDuration, formatTokens, isPlanMode, mergeMessages, nextMode, pendingEchoMessage, pendingSubagentView, planButtonTarget, sameGoalState, samePlanState, sameTodoState, sortedArchived, thinkingStream, titleFromPrompt, validateInitCommand, withPendingEcho } from "./chat-utils";
import type { GoalState, ImageContent, NormalizedMessage, SessionSnapshot, SnapshotDelta, SubagentStreamFrame, TaskRuntime } from "./types";

describe("titleFromPrompt", () => {
  it("uses the first non-empty line", () => {
    expect(titleFromPrompt("Fix the login bug\n\nMore context here")).toBe("Fix the login bug");
  });

  it("skips leading blank lines", () => {
    expect(titleFromPrompt("\n\n  add tests  ")).toBe("add tests");
  });

  it("truncates long prompts", () => {
    const title = titleFromPrompt("x".repeat(100));
    expect(title.length).toBe(48);
    expect(title.endsWith("…")).toBe(true);
  });

  it("falls back to a default", () => {
    expect(titleFromPrompt("   \n")).toBe("New chat");
  });
});

describe("/init validation", () => {
  it("requires no arguments, a project, and Build mode", () => {
    expect(() => validateInitCommand("", "project", "build")).not.toThrow();
    expect(() => validateInitCommand("extra", "project", "build")).toThrow("does not accept arguments");
    expect(() => validateInitCommand("", null, "build")).toThrow("needs a project");
    expect(() => validateInitCommand("", "project", "plan")).toThrow("Build mode");
    expect(() => validateInitCommand("", "project", "ultraplan")).toThrow("Build mode");
  });
});

describe("modes", () => {
  it("treats Plan and Ultra Plan as planning", () => {
    expect(isPlanMode("plan")).toBe(true);
    expect(isPlanMode("ultraplan")).toBe(true);
    expect(isPlanMode("build")).toBe(false);
    expect(isPlanMode(undefined)).toBe(false);
  });

  it("cycles Build → Plan → Ultra Plan → Build on ⇧Tab", () => {
    expect(nextMode("build")).toBe("plan");
    expect(nextMode("plan")).toBe("ultraplan");
    expect(nextMode("ultraplan")).toBe("build");
  });

  it("enters Plan from Build, then toggles Plan ↔ Ultra Plan from the Plan button", () => {
    expect(planButtonTarget("build")).toBe("plan");
    expect(planButtonTarget("plan")).toBe("ultraplan");
    expect(planButtonTarget("ultraplan")).toBe("plan");
  });
});

describe("displayPath", () => {
  it("shows short paths in full", () => {
    expect(displayPath("src/App.tsx")).toBe("src/App.tsx");
  });

  it("trims absolute paths to the last segments", () => {
    expect(displayPath("/Users/jack/project/src/App.tsx")).toBe("…/src/App.tsx");
  });
});

describe("formatTokens", () => {
  it("formats magnitudes", () => {
    expect(formatTokens(500)).toBe("500");
    expect(formatTokens(12_300)).toBe("12.3k");
    expect(formatTokens(2_500_000)).toBe("2.5m");
  });
});

describe("formatRunDuration", () => {
  it("formats seconds, minutes, and hours", () => {
    expect(formatRunDuration(999)).toBe("0s");
    expect(formatRunDuration(59_000)).toBe("59s");
    expect(formatRunDuration(60_000)).toBe("1m 0s");
    expect(formatRunDuration(2 * 60_000 + 21_000)).toBe("2m 21s");
    expect(formatRunDuration(60 * 60_000 + 24 * 60_000 + 20_000)).toBe("1h 24m 20s");
  });

  it("clamps negative values to zero", () => {
    expect(formatRunDuration(-1)).toBe("0s");
  });
});

describe("formatThinkingDuration", () => {
  it("counts whole seconds like the run duration, with ‘<1s’ below the first tick", () => {
    expect(formatThinkingDuration(0, "second")).toBe("<1s");
    expect(formatThinkingDuration(999, "second")).toBe("<1s");
    expect(formatThinkingDuration(1_000, "second")).toBe("1s");
    expect(formatThinkingDuration(4_200, "second")).toBe("4s");
    expect(formatThinkingDuration(75_000, "second")).toBe("1m 15s");
    expect(formatThinkingDuration(60 * 60_000 + 24 * 60_000 + 20_000, "second")).toBe("1h 24m 20s");
  });

  it("counts tenths as total seconds, never folding into minutes", () => {
    expect(formatThinkingDuration(99, "tenth")).toBe("<0.1s");
    expect(formatThinkingDuration(100, "tenth")).toBe("0.1s");
    expect(formatThinkingDuration(1_150, "tenth")).toBe("1.1s");
    expect(formatThinkingDuration(4_200, "tenth")).toBe("4.2s");
    expect(formatThinkingDuration(59_940, "tenth")).toBe("59.9s");
    expect(formatThinkingDuration(75_300, "tenth")).toBe("75.3s");
    expect(formatThinkingDuration(3_600_000 + 60_000 + 4_500, "tenth")).toBe("3664.5s");
  });

  it("clamps negative values to zero", () => {
    expect(formatThinkingDuration(-1, "second")).toBe("<1s");
    expect(formatThinkingDuration(-1, "tenth")).toBe("<0.1s");
  });
});

describe("formatRelativeTime", () => {
  // Timezone-less ISO strings parse as local time, keeping the day buckets stable on any machine.
  const now = new Date("2026-09-27T12:00:00");

  it("uses the archived view's shorthand buckets", () => {
    expect(formatRelativeTime("2026-09-27T11:59:30", now)).toBe("now");
    expect(formatRelativeTime("2026-09-27T11:55:00", now)).toBe("5m");
    expect(formatRelativeTime("2026-09-27T05:00:00", now)).toBe("7h");
    expect(formatRelativeTime("2026-09-25T12:00:00", now)).toBe("2d");
  });

  it("falls back to a short date after a week, with the year when it differs", () => {
    expect(formatRelativeTime("2026-09-14T12:00:00", now)).toBe("14 Sep");
    expect(formatRelativeTime("2025-03-02T12:00:00", now)).toBe("2 Mar 2025");
  });

  it("clamps future timestamps to now and tolerates garbage", () => {
    expect(formatRelativeTime("2026-09-27T12:00:30", now)).toBe("now");
    expect(formatRelativeTime("not a date", now)).toBe("");
  });
});

describe("sortedArchived", () => {
  const base = { projectId: null, name: "n", autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/tmp", worktreePath: null, branch: null, usesWorktree: false, providerId: "p", modelId: "m", thinkingLevel: "off" as const, sessionFile: null, status: "idle" as const, mode: "build" as const, lastError: null, kind: "code" as const, lastActivityAt: null, createdAt: "c" };
  const task = (id: string, extra: Partial<import("./types").TaskRecord>): import("./types").TaskRecord => ({ ...base, id, archived: true, archivedAt: null, updatedAt: "u", ...extra });

  it("orders by archivedAt, newest first", () => {
    const tasks = [
      task("old", { archivedAt: "2026-09-25T10:00:00Z", updatedAt: "2026-09-26T10:00:00Z" }),
      task("new", { archivedAt: "2026-09-27T10:00:00Z", updatedAt: "2026-09-26T10:00:00Z" }),
      task("mid", { archivedAt: "2026-09-26T10:00:00Z", updatedAt: "2026-09-27T11:00:00Z" })
    ];
    expect(sortedArchived(tasks).map((item) => item.id)).toEqual(["new", "mid", "old"]);
  });

  it("keeps only archived chats and falls back to updatedAt for legacy rows", () => {
    const tasks = [
      task("legacy", { archivedAt: null, updatedAt: "2026-09-20T10:00:00Z" }),
      task("stamped", { archivedAt: "2026-09-21T10:00:00Z", updatedAt: "2026-09-19T10:00:00Z" }),
      task("live", { archived: false, archivedAt: null, updatedAt: "2026-09-27T10:00:00Z" })
    ];
    expect(sortedArchived(tasks).map((item) => item.id)).toEqual(["stamped", "legacy"]);
  });
});

describe("samePlanState", () => {
  it("matches equal states across object identities", () => {
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "Do it" }, { mode: "plan", phase: "ready", plan: "Do it" })).toBe(true);
  });

  it("treats an absent and an empty plan string as different", () => {
    expect(samePlanState({ mode: "plan", phase: "planning" }, { mode: "plan", phase: "planning", plan: undefined })).toBe(true);
    expect(samePlanState({ mode: "plan", phase: "planning" }, { mode: "plan", phase: "ready", plan: "" })).toBe(false);
  });

  it("rejects differing mode, phase, or plan", () => {
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "a" }, { mode: "build", phase: "ready", plan: "a" })).toBe(false);
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "a" }, { mode: "plan", phase: "planning", plan: "a" })).toBe(false);
    expect(samePlanState({ mode: "plan", phase: "ready", plan: "a" }, { mode: "plan", phase: "ready", plan: "b" })).toBe(false);
  });

  it("handles undefined on either side", () => {
    expect(samePlanState(undefined, undefined)).toBe(true);
    expect(samePlanState({ mode: "build", phase: "planning" }, undefined)).toBe(false);
    expect(samePlanState(undefined, { mode: "build", phase: "planning" })).toBe(false);
  });
});

describe("sameTodoState", () => {
  const task = { id: 1, subject: "Write tests", status: "in_progress" as const };

  it("matches equal task lists across object identities", () => {
    expect(sameTodoState({ tasks: [task] }, { tasks: [{ ...task }] })).toBe(true);
  });

  it("compares blockedBy lists element-wise", () => {
    expect(sameTodoState({ tasks: [{ ...task, blockedBy: [2, 3] }] }, { tasks: [{ ...task, blockedBy: [2, 3] }] })).toBe(true);
    expect(sameTodoState({ tasks: [{ ...task, blockedBy: [2, 3] }] }, { tasks: [{ ...task, blockedBy: [3, 2] }] })).toBe(false);
    expect(sameTodoState({ tasks: [{ ...task, blockedBy: [2] }] }, { tasks: [{ ...task }] })).toBe(false);
  });

  it("rejects differing length, status, or fields", () => {
    expect(sameTodoState({ tasks: [task] }, { tasks: [task, { id: 2, subject: "Other", status: "pending" }] })).toBe(false);
    expect(sameTodoState({ tasks: [task] }, { tasks: [{ ...task, status: "completed" as const }] })).toBe(false);
    expect(sameTodoState({ tasks: [task] }, { tasks: [{ ...task, activeForm: "Writing tests" }] })).toBe(false);
  });
});

describe("sameGoalState", () => {
  const goal: GoalState = { objective: "Ship it", phase: "active", iteration: 1, maxIterations: 25, noProgress: 0 };

  it("matches equal states across object identities and handles undefined", () => {
    expect(sameGoalState(goal, { ...goal })).toBe(true);
    expect(sameGoalState(undefined, undefined)).toBe(true);
    expect(sameGoalState(goal, undefined)).toBe(false);
    expect(sameGoalState(undefined, goal)).toBe(false);
  });

  it("rejects any field that changes", () => {
    expect(sameGoalState(goal, { ...goal, phase: "verifying" })).toBe(false);
    expect(sameGoalState(goal, { ...goal, iteration: 2 })).toBe(false);
    expect(sameGoalState(goal, { ...goal, noProgress: 1 })).toBe(false);
    expect(sameGoalState(goal, { ...goal, lastNextAction: "try again" })).toBe(false);
    expect(sameGoalState(goal, { ...goal, note: "Paused." })).toBe(false);
  });
});

describe("applySnapshotDelta", () => {
  const stats = {
    tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    cost: 0,
    contextBreakdown: { entries: [], cacheHitRate: null }
  };

  function snapshot(messages: NormalizedMessage[], rev: number): SessionSnapshot {
    return {
      rev,
      sessionId: "s",
      messages,
      modelSwitches: [],
      runTimings: [],
      tree: { leafId: null },
      stats,
      thinkingLevel: "off",
      availableThinkingLevels: ["off"],
      tools: [],
      activeTools: []
    };
  }

  function message(id: string, text = id): NormalizedMessage {
    return { id, role: "assistant", blocks: [{ type: "text", text }] };
  }

  function delta(partial: Partial<SnapshotDelta> & { rev: number }): SnapshotDelta {
    return { upserts: [], removed: [], tree: { leafId: null }, stats, ...partial };
  }

  it("keeps the prompt clock across compaction and clears compaction status on its end delta", () => {
    const base = { ...snapshot([message("a")], 1), activeRun: { runId: "r", startedAt: 5 } };
    const compacting = applySnapshotDelta(base, delta({ rev: 2, compaction: { reason: "overflow" } }));
    expect(compacting.compaction).toEqual({ reason: "overflow" });
    expect(compacting.activeRun).toBe(base.activeRun);
    expect(applySnapshotDelta(compacting, delta({ rev: 3 })).compaction).toBe(compacting.compaction);
    const resumed = applySnapshotDelta(compacting, delta({ rev: 3, compaction: null }));
    expect(resumed.compaction).toBeUndefined();
    expect(resumed.activeRun).toBe(base.activeRun);
  });

  it("replaces applied execution policy in a delta and retains it when omitted", () => {
    const base = { ...snapshot([], 1), executionPolicy: { unrestrictedSubagents: false, unrestrictedPlanning: false } };
    const policy = { unrestrictedSubagents: true, unrestrictedPlanning: true };
    expect(applySnapshotDelta(base, delta({ rev: 2, executionPolicy: policy })).executionPolicy).toBe(policy);
    expect(applySnapshotDelta(base, delta({ rev: 2 })).executionPolicy).toBe(base.executionPolicy);
  });

  it("appends upserts with unknown ids and replaces known ones in place", () => {
    const a = message("a");
    const b = message("b");
    const next = applySnapshotDelta(snapshot([a, b], 1), delta({ rev: 2, upserts: [message("c"), message("b", "changed")] }));
    expect(next.messages.map((entry) => entry.id)).toEqual(["a", "b", "c"]);
    expect(next.messages[1].blocks[0].text).toBe("changed");
    expect(next.messages[0]).toBe(a);
    expect(next.rev).toBe(2);
  });

  it("removes departed ids before applying upserts, as a retry does", () => {
    const u1 = message("u1", "one");
    const a1 = message("a1");
    const u2v2 = message("u2v2", "two again");
    const next = applySnapshotDelta(snapshot([u1, a1, message("u2"), message("a2")], 4), delta({ rev: 5, removed: ["u2", "a2"], upserts: [u2v2] }));
    expect(next.messages).toEqual([u1, a1, u2v2]);
  });

  it("keeps untouched message object identities so memoization holds", () => {
    const a = message("a");
    const b = message("b");
    const next = applySnapshotDelta(snapshot([a, b], 1), delta({ rev: 2, upserts: [message("c")] }));
    expect(next.messages[0]).toBe(a);
    expect(next.messages[1]).toBe(b);
  });

  it("keeps the previous array when the delta touches no messages", () => {
    const base = snapshot([message("a")], 1);
    const next = applySnapshotDelta(base, delta({ rev: 2 }));
    expect(next.messages).toBe(base.messages);
  });

  it("retains absent model switches and replaces them as one identity-stable field", () => {
    const first = [{
      id: "switch-1", at: 1,
      from: { providerId: "p", modelId: "old" },
      to: { providerId: "p", modelId: "new" }
    }];
    const base = { ...snapshot([message("a")], 1), modelSwitches: first };
    expect(applySnapshotDelta(base, delta({ rev: 2 })).modelSwitches).toBe(first);
    const replacement = [{ ...first[0], id: "switch-2", at: 2 }];
    expect(applySnapshotDelta(base, delta({ rev: 2, modelSwitches: replacement })).modelSwitches).toBe(replacement);
  });

  it("treats absent fields as unchanged and activeRun null as cleared", () => {
    const base = { ...snapshot([message("a")], 1), activeRun: { runId: "r", startedAt: 5 }, planState: { mode: "build" as const, phase: "planning" as const } };
    const sameRun = applySnapshotDelta(base, delta({ rev: 2 }));
    expect(sameRun.activeRun).toEqual({ runId: "r", startedAt: 5 });
    expect(sameRun.planState).toBe(base.planState);

    const cleared = applySnapshotDelta(base, delta({ rev: 2, activeRun: null }));
    expect(cleared.activeRun).toBeUndefined();
    const replaced = applySnapshotDelta(base, delta({ rev: 2, activeRun: { runId: "r2", startedAt: 9 } }));
    expect(replaced.activeRun).toEqual({ runId: "r2", startedAt: 9 });
  });

  it("keeps the previous plan state object when the delta carries an equal one", () => {
    const base = { ...snapshot([], 1), planState: { mode: "plan" as const, phase: "ready" as const, plan: "p" } };
    const next = applySnapshotDelta(base, delta({ rev: 2, planState: { mode: "plan", phase: "ready", plan: "p" } }));
    expect(next.planState).toBe(base.planState);
    const changed = applySnapshotDelta(base, delta({ rev: 2, planState: { mode: "plan", phase: "planning" } }));
    expect(changed.planState).toEqual({ mode: "plan", phase: "planning" });
  });

  it("applies goal state: absent keeps, null clears, equal keeps identity", () => {
    const goal: GoalState = { objective: "Ship it", phase: "active", iteration: 1, maxIterations: 25, noProgress: 0 };
    const a = message("a");
    const base = { ...snapshot([a], 1), goalState: goal };

    const kept = applySnapshotDelta(base, delta({ rev: 2 }));
    expect(kept.goalState).toBe(goal);
    expect(kept.messages[0]).toBe(a);

    const equal = applySnapshotDelta(base, delta({ rev: 2, goalState: { ...goal } }));
    expect(equal.goalState).toBe(goal);

    const advanced = applySnapshotDelta(base, delta({ rev: 2, goalState: { ...goal, phase: "verifying" as const } }));
    expect(advanced.goalState?.phase).toBe("verifying");

    const cleared = applySnapshotDelta(base, delta({ rev: 2, goalState: null }));
    expect(cleared.goalState).toBeUndefined();
    expect(cleared.messages[0]).toBe(a);
  });

  it("applies skill-creator state: absent keeps, null clears", () => {
    const workflow = { draftId: "d-1", name: "pdf-tools", revision: "r-1" };
    const base = { ...snapshot([message("a")], 1), skillCreator: workflow };
    expect(applySnapshotDelta(base, delta({ rev: 2 })).skillCreator).toBe(workflow);
    expect(applySnapshotDelta(base, delta({ rev: 2, skillCreator: workflow })).skillCreator).toBe(workflow);
    const replaced = applySnapshotDelta(base, delta({ rev: 2, skillCreator: { draftId: "d-2", name: "other" } }));
    expect(replaced.skillCreator).toEqual({ draftId: "d-2", name: "other" });
    expect(applySnapshotDelta(base, delta({ rev: 2, skillCreator: null })).skillCreator).toBeUndefined();
  });
});

describe("thinkingStream", () => {
  it("shows nothing until reasoning exists, and keeps a partial tail", () => {
    expect(thinkingStream("")).toBeUndefined();
    expect(thinkingStream("The user wants a new settings pa")).toBe("The user wants a new settings pa");
  });

  it("keeps flowing text instead of latching a heading", () => {
    const text = "**Exploring the code**\n\nI read App.tsx.\n\n**Planning the settings page**\n\nIt needs a toggle. Also";
    expect(thinkingStream(text)).toBe("Exploring the code · I read App.tsx. · Planning the settings page · It needs a toggle. Also");
    expect(thinkingStream("## Checking tests\nThey pass.")).toBe("Checking tests They pass.");
  });

  it("strips markdown and collapses whitespace", () => {
    expect(thinkingStream("- Use `thinkingStream`   in **ThinkingRow**. Then")).toBe("Use thinkingStream in ThinkingRow. Then");
  });

  it("keeps only the tail", () => {
    const text = `${"word ".repeat(80)}end. `;
    expect(thinkingStream(text)).toBe(text.trim().slice(-200));
  });
});

describe("the pending echo of a just-sent message", () => {
  const images: ImageContent[] = [{ type: "image", data: "AAAA", mimeType: "image/png" }];

  it("carries the sent text and image previews, keyed by the run's start", () => {
    const echo = pendingEchoMessage("Ship it", images, 1_000);
    expect(echo).toEqual({
      id: "pending:1000",
      role: "user",
      timestamp: 1_000,
      blocks: [
        { type: "text", text: "Ship it" },
        { type: "image", mimeType: "image/png", thumbnail: "data:image/png;base64,AAAA" }
      ]
    });
    expect(pendingEchoMessage("", undefined, 2_000).blocks).toEqual([]);
  });

  it("appends the echo until a user message of the run arrives, then hands back the snapshot's list", () => {
    const history: NormalizedMessage[] = [
      { id: "old-user", role: "user", timestamp: 500, blocks: [{ type: "text", text: "Before" }] },
      { id: "old-answer", role: "assistant", timestamp: 600, blocks: [] }
    ];
    const runtime: TaskRuntime = {
      snapshot: { messages: history } as SessionSnapshot,
      pendingMessage: pendingEchoMessage("Ship it", undefined, 1_000)
    };
    // Older user messages never satisfy the arrival test, so the echo shows.
    const shown = withPendingEcho(runtime);
    expect(shown.map((message) => message.id)).toEqual(["old-user", "old-answer", "pending:1000"]);
    expect(shown[0]).toBe(history[0]);

    // The worker records the real message at or after the run's start: same array, no echo.
    const arrived: NormalizedMessage = { id: "real", role: "user", timestamp: 1_000, blocks: [{ type: "text", text: "Ship it" }] };
    const settled: TaskRuntime = { ...runtime, snapshot: { messages: [...history, arrived] } as SessionSnapshot };
    expect(withPendingEcho(settled)).toBe(settled.snapshot!.messages);
  });

  it("returns the snapshot's list untouched with no echo and for an empty runtime", () => {
    const history: NormalizedMessage[] = [{ id: "only", role: "assistant", blocks: [] }];
    const runtime: TaskRuntime = { snapshot: { messages: history } as SessionSnapshot };
    expect(withPendingEcho(runtime)).toBe(history);
    expect(withPendingEcho(undefined)).toEqual([]);
  });
});

describe("sub-agent transcript frames", () => {
  function message(id: string, text = id): NormalizedMessage {
    return { id, role: "assistant", blocks: [{ type: "text", text }] };
  }
  const target = { toolCallId: "call-1", index: 0 };
  function frame(partial: Partial<SubagentStreamFrame> & { rev: number }): SubagentStreamFrame {
    return { ...target, upserts: [], removed: [], partial: null, live: true, ...partial };
  }

  it("merges like a snapshot delta, keeping untouched messages and returning the list itself for an empty change", () => {
    const a = message("a");
    const b = message("b");
    const list = [a, b];
    expect(mergeMessages(list, [], [])).toBe(list);
    const next = mergeMessages(list, [message("b", "changed"), message("c")], ["a"]);
    expect(next.map((entry) => entry.id)).toEqual(["b", "c"]);
    expect(next[0].blocks[0].text).toBe("changed");
    expect(mergeMessages(list, [message("c")], [])[0]).toBe(a);
  });

  it("waits for a reset, then chains frames by rev", () => {
    const pending = pendingSubagentView(target);
    expect(pending).toMatchObject({ rev: -1, loading: true, messages: [] });
    // A straggler from an earlier watch is ignored while the reset is on its way.
    expect(applySubagentFrame(pending, frame({ rev: 7 }))).toBe(pending);

    const a = message("a");
    const seeded = applySubagentFrame(pending, frame({ rev: 0, reset: true, upserts: [a], live: true, missing: undefined }));
    expect(seeded).toMatchObject({ rev: 0, loading: false, live: true, missing: false, truncated: false, messages: [a] });

    const streaming = message("partial", "Look");
    const next = applySubagentFrame(seeded!, frame({ rev: 1, upserts: [message("b")], partial: streaming }));
    expect(next?.messages.map((entry) => entry.id)).toEqual(["a", "b"]);
    expect(next?.messages[0]).toBe(a);
    expect(next?.partial).toBe(streaming);

    const done = applySubagentFrame(next!, frame({ rev: 2, live: false, truncated: true }));
    expect(done).toMatchObject({ live: false, truncated: true });
    expect(done?.messages).toBe(next?.messages);
    expect(done).not.toHaveProperty("partial");
  });

  it("asks for a fresh reset when a frame went missing, and ignores other children", () => {
    const seeded = applySubagentFrame(pendingSubagentView(target), frame({ rev: 0, reset: true }))!;
    expect(applySubagentFrame(seeded, frame({ rev: 2 }))).toBeUndefined();
    expect(applySubagentFrame(seeded, { ...frame({ rev: 1 }), index: 1 })).toBe(seeded);
    expect(applySubagentFrame(seeded, frame({ rev: 5, reset: true, missing: true, live: false }))).toMatchObject({ rev: 5, missing: true });
  });
});
