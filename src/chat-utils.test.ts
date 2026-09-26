import { describe, expect, it } from "vitest";
import { applySnapshotDelta, displayPath, formatRunDuration, formatTokens, isPlanMode, nextMode, planButtonTarget, sameGoalState, samePlanState, sameTodoState, thinkingPreview, titleFromPrompt, validateInitCommand } from "./chat-utils";
import type { GoalState, NormalizedMessage, SessionSnapshot, SnapshotDelta } from "./types";

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
});

describe("thinkingPreview", () => {
  it("shows nothing until a sentence is finished", () => {
    expect(thinkingPreview("")).toBeUndefined();
    expect(thinkingPreview("The user wants a new settings pa")).toBeUndefined();
  });

  it("uses the latest finished sentence and ignores the unfinished tail", () => {
    expect(thinkingPreview("First I read the file. Then I check the tests. Now the st")).toBe("Then I check the tests.");
    expect(thinkingPreview("One thought.\nA whole line without a stop\nstill writ")).toBe("A whole line without a stop");
  });

  it("does not split on dots inside words", () => {
    expect(thinkingPreview("Open src/App.tsx and read it. Nex")).toBe("Open src/App.tsx and read it.");
  });

  it("prefers the latest heading over sentences", () => {
    const text = "**Exploring the code**\n\nI read App.tsx.\n\n**Planning the settings page**\n\nIt needs a toggle. Also";
    expect(thinkingPreview(text)).toBe("Planning the settings page");
    expect(thinkingPreview("## Checking tests\nThey pass. ")).toBe("Checking tests");
    expect(thinkingPreview("Done. \n## Checking te")).toBe("Done.");
  });

  it("strips markdown and collapses whitespace", () => {
    expect(thinkingPreview("- Use `thinkingPreview`   in **ThinkingRow**. Then")).toBe("Use thinkingPreview in ThinkingRow.");
  });

  it("caps long lines", () => {
    const preview = thinkingPreview(`${"word ".repeat(80)}end. `);
    expect(preview).toHaveLength(200);
    expect(preview?.endsWith("…")).toBe(true);
  });
});
