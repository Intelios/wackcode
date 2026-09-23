import { describe, expect, it } from "vitest";
import { normalizeAskQuestionsParams } from "./builtin/ask-user-question.js";
import {
  normalizePlanModeCompletion,
  planFromCompletionDetails,
  planModeCompleted,
} from "./builtin/plan-mode/completion.js";
import {
  createModeContractMessage,
  hasModeContractArtifact,
  latestModeContract,
  modeContractFromMessage,
  reconcileModeContract,
} from "./builtin/plan-mode/contract.js";
import {
  classifyPlanTool,
  findBlockedCommandSegment,
  isSafeCommand,
} from "./builtin/plan-mode/policy.js";
import { latestAssistantText, parseProposedPlan } from "./builtin/plan-mode/proposed-plan.js";
import { PLAN_STATE_ENTRY_TYPE, restorePlanState } from "./builtin/plan-mode/state.js";
import { buildToolResult } from "./builtin/todo/envelope.js";
import { VALID_TRANSITIONS, isTransitionValid } from "./builtin/todo/invariants.js";
import { applyTaskMutation } from "./builtin/todo/reducer.js";
import { EMPTY_STATE, type TaskState, replayFromBranch } from "./builtin/todo/state.js";
import { deriveBlocks, detectCycle } from "./builtin/todo/task-graph.js";
import { TODO_DETAILS_VERSION, normalizeTodoParams } from "./builtin/todo/types.js";
import {
  addUsage,
  createDetails,
  emptyUsage,
  modelContent,
  recordActivity,
  summarizeActivity,
  truncate,
} from "./builtin/subagents/details.js";
import { readOnlyDecision } from "./builtin/subagents/guard.js";
import { subagentGuidelines } from "./builtin/subagents/prompt.js";
import { runScheduled } from "./builtin/subagents/scheduler.js";
import { normalizeSubagentParams, subagentParams } from "./builtin/subagents/schema.js";
import { resolveChildTools } from "./builtin/subagents/types.js";
import type { TodoTask } from "./protocol.js";

describe("plan-mode tool policy", () => {
  const tool = (name: string, source = "builtin") => ({ name, sourceInfo: { source } }) as never;

  it("blocks mutating built-ins, allows read/search tools, and denies package tools outright", () => {
    expect(classifyPlanTool(tool("edit"))).toBe("blocked");
    expect(classifyPlanTool(tool("write"))).toBe("blocked");
    expect(classifyPlanTool(tool("update_plan"))).toBe("blocked");
    expect(classifyPlanTool(tool("read"))).toBe("read-only");
    expect(classifyPlanTool(tool("grep"))).toBe("read-only");
    expect(classifyPlanTool(tool("ls"))).toBe("read-only");
    expect(classifyPlanTool(tool("bash"))).toBe("limited");
    expect(classifyPlanTool(tool("web_fetch", "npm:pi-web-access"))).toBe("package");
  });

  it("allows ordinary read-only shell commands", () => {
    for (const command of [
      "ls -la",
      "cat src/index.ts | head -20",
      "rg 'fn main' src/",
      "git status",
      "git log --oneline -5",
      "git diff HEAD~1",
      "npm test",
      "npm run typecheck",
      "pnpm run lint",
      "cargo test",
      "tsc --noEmit",
      "echo hello && wc -l README.md",
    ]) {
      expect(isSafeCommand(command), command).toBe(true);
    }
  });

  it("blocks anything that writes, escapes the parser, or runs a mutating subcommand", () => {
    for (const command of [
      "rm -rf build",
      "ls > listing.txt",
      "cat a && touch b",
      "git commit -m 'x'",
      "git push",
      "git branch -D old",
      "npm install",
      "npm audit fix",
      "pnpm add lodash",
      "find . -exec rm {} ;",
      "sed -i 's/a/b/' file",
      "FOO=bar echo hi",
      "ls $(whoami)",
      "ls `pwd`",
      "ls; sleep 1 &",
      "sudo ls",
    ]) {
      expect(isSafeCommand(command), command).toBe(false);
    }
  });

  it("names the offending segment so the model can see exactly what was refused", () => {
    expect(findBlockedCommandSegment("ls && rm -rf x")).toBe("rm -rf x");
    expect(findBlockedCommandSegment("cat a | grep b")).toBeUndefined();
  });

  it("permits reviewed gh read paths only when they return --json", () => {
    const safe = { gh: ["pr view", "issue list"] };
    expect(isSafeCommand("gh pr view 12 --json title,body", safe)).toBe(true);
    expect(isSafeCommand("gh pr view 12", safe)).toBe(false);
    expect(isSafeCommand("gh pr view 12 --web", safe)).toBe(false);
    expect(isSafeCommand("gh pr merge 12", safe)).toBe(false);
  });
});

describe("ask_user_question normalization", () => {
  const valid = {
    questions: [
      {
        id: "approach",
        header: "Approach",
        question: "Which way?",
        options: [
          { label: "A", description: "first" },
          { label: "B", description: "second" },
        ],
      },
    ],
  };

  it("accepts a well-formed questionnaire", () => {
    const result = normalizeAskQuestionsParams(valid);
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.questions[0]?.id).toBe("approach");
  });

  it("accepts multiSelect and rejects underspecified questions", () => {
    const multi = normalizeAskQuestionsParams({ questions: [{ ...valid.questions[0], multiSelect: true }] });
    expect(multi.ok && multi.questions[0]?.multiSelect).toBe(true);
    expect(normalizeAskQuestionsParams({}).ok).toBe(false);
    expect(normalizeAskQuestionsParams({ questions: [] }).ok).toBe(false);
    expect(normalizeAskQuestionsParams({ questions: [{ ...valid.questions[0], options: [] }] }).ok).toBe(false);
    expect(normalizeAskQuestionsParams({ questions: [{ ...valid.questions[0], header: " " }] }).ok).toBe(false);
    expect(normalizeAskQuestionsParams({ questions: [1, 2, 3, 4].map(() => valid.questions[0]) }).ok).toBe(false);
  });
});

describe("plan_mode_complete parsing", () => {
  it("accepts a non-empty plan within bounds and rejects the rest", () => {
    expect(normalizePlanModeCompletion({ plan: " Do the thing " })).toEqual({ ok: true, plan: "Do the thing" });
    expect(normalizePlanModeCompletion({ plan: "  " }).ok).toBe(false);
    expect(normalizePlanModeCompletion({ plan: 5 }).ok).toBe(false);
    expect(normalizePlanModeCompletion({ plan: "x".repeat(50_001) }).ok).toBe(false);
  });

  it("round-trips through the tool result's details so restores can find it", () => {
    const result = planModeCompleted("# Plan\n\n- step one");
    expect(planFromCompletionDetails(result.details)).toBe("# Plan\n\n- step one");
    expect(planFromCompletionDetails({ version: 99, source: "plan_mode_complete", plan: "x" })).toBeUndefined();
    expect(planFromCompletionDetails(undefined)).toBeUndefined();
  });
});

describe("mode contract reconciliation", () => {
  it("round-trips a contract message and reports the latest mode", () => {
    const plan = createModeContractMessage("plan");
    expect(modeContractFromMessage(plan)).toBe("plan");
    expect(modeContractFromMessage({ role: "user", content: "hi" })).toBeUndefined();
    expect(hasModeContractArtifact([{ role: "user" }, plan])).toBe(true);
    expect(latestModeContract([plan, createModeContractMessage("normal")])?.mode).toBe("normal");
  });

  it("re-appends the current contract when the latest artifact disagrees (e.g. after restore)", () => {
    const history = [
      { role: "system", content: "sys" },
      { role: "user", content: "hi" },
      createModeContractMessage("plan"),
      { role: "assistant", content: "ok" },
    ];
    const reconciled = reconcileModeContract(history, "normal");
    expect(reconciled.length).toBe(history.length + 1);
    expect(latestModeContract(reconciled)?.mode).toBe("normal");
    // A matching latest contract is a no-op — the context stays cache-stable.
    expect(reconcileModeContract(reconciled, "normal")).toBe(reconciled);
  });
});

describe("plan state restore", () => {
  const stateEntry = (enabled: boolean, plan?: string) => ({
    type: "custom",
    customType: PLAN_STATE_ENTRY_TYPE,
    data: { version: 1, enabled, plan },
  });

  it("restores the latest persisted state, including a ready plan", () => {
    expect(restorePlanState([stateEntry(true, "the plan")])).toEqual({ enabled: true, plan: "the plan" });
    expect(restorePlanState([stateEntry(true), stateEntry(false)])).toEqual({ enabled: false });
    expect(restorePlanState([])).toEqual({ enabled: false });
    expect(restorePlanState([stateEntry(false, "stale")])).toEqual({ enabled: false });
  });

  it("recovers a plan from the completion tool result when the state write was lost", () => {
    const branch = [
      stateEntry(true),
      {
        message: {
          role: "toolResult",
          toolName: "plan_mode_complete",
          details: { version: 1, source: "plan_mode_complete", plan: "recovered" },
        },
      },
    ];
    expect(restorePlanState(branch)).toEqual({ enabled: true, plan: "recovered" });
  });
});

describe("proposed_plan fallback parsing", () => {
  it("accepts exactly one well-formed block and rejects the degenerate cases", () => {
    expect(parseProposedPlan("text\n<proposed_plan>\nthe plan\n</proposed_plan>\n")).toEqual({ kind: "valid", plan: "the plan" });
    expect(parseProposedPlan("no tags")).toEqual({ kind: "absent" });
    expect(parseProposedPlan("<proposed_plan>\n\n</proposed_plan>").kind).toBe("empty");
    expect(parseProposedPlan("<proposed_plan>a</proposed_plan><proposed_plan>b</proposed_plan>").kind).toBe("multiple");
    expect(parseProposedPlan("<proposed_plan>never closed").kind).toBe("unclosed");
  });

  it("finds the last assistant text in an agent_end message list", () => {
    const messages = [
      { message: { role: "assistant", content: [{ type: "text", text: "first" }] } },
      { message: { role: "user", content: "u" } },
      { message: { role: "assistant", content: [{ type: "text", text: "last" }] } },
    ];
    expect(latestAssistantText(messages)).toBe("last");
    expect(latestAssistantText([])).toBe("");
  });
});

describe("todo status machine", () => {
  it("follows the 4-state machine with a terminal tombstone", () => {
    expect(isTransitionValid("pending", "in_progress")).toBe(true);
    expect(isTransitionValid("pending", "completed")).toBe(true);
    expect(isTransitionValid("in_progress", "pending")).toBe(true);
    expect(isTransitionValid("completed", "in_progress")).toBe(false);
    expect(isTransitionValid("completed", "deleted")).toBe(true);
    expect(isTransitionValid("deleted", "pending")).toBe(false);
    // A same→same update is an accepted no-op, handled outside the table.
    expect(isTransitionValid("completed", "completed")).toBe(true);
    expect(VALID_TRANSITIONS.deleted.size).toBe(0);
  });
});

describe("todo parameter normalization", () => {
  it("accepts a well-formed mutation bag and rejects bad shapes", () => {
    const ok = normalizeTodoParams({ action: "update", id: 3, status: "completed", addBlockedBy: [1] });
    expect(ok).toEqual({ ok: true, action: "update", params: { id: 3, status: "completed", addBlockedBy: [1] } });
    expect(normalizeTodoParams(undefined).ok).toBe(false);
    expect(normalizeTodoParams({ action: "explode" }).ok).toBe(false);
    expect(normalizeTodoParams({ action: "create", subject: 5 }).ok).toBe(false);
    expect(normalizeTodoParams({ action: "create", blockedBy: ["1"] }).ok).toBe(false);
    expect(normalizeTodoParams({ action: "list", status: "sideways" }).ok).toBe(false);
  });
});

describe("todo reducer", () => {
  const seed: TaskState = {
    tasks: [
      { id: 1, subject: "First", status: "completed" },
      { id: 2, subject: "Second", status: "in_progress", blockedBy: [1] }
    ],
    nextId: 3
  };

  it("creates tasks in pending and advances the id counter", () => {
    const result = applyTaskMutation(seed, "create", { subject: "Third", activeForm: "building the third" });
    expect(result.op).toEqual({ kind: "create", taskId: 3 });
    expect(result.state.nextId).toBe(4);
    expect(result.state.tasks[2]).toEqual({ id: 3, subject: "Third", status: "pending", activeForm: "building the third" });
  });

  it("rejects invalid mutations and leaves state untouched", () => {
    const cases: Array<[TaskState, Parameters<typeof applyTaskMutation>[1], Parameters<typeof applyTaskMutation>[2], string]> = [
      [seed, "create", {}, "subject required for create"],
      [seed, "create", { subject: "X", blockedBy: [9] }, "blockedBy: #9 not found"],
      [seed, "update", {}, "id required for update"],
      [seed, "update", { id: 9 }, "#9 not found"],
      [seed, "update", { id: 2 }, "update requires at least one mutable field: subject, description, activeForm, status, addBlockedBy, or removeBlockedBy"],
      [seed, "update", { id: 1, status: "in_progress" }, "illegal transition completed → in_progress"],
      [seed, "update", { id: 2, addBlockedBy: [2] }, "cannot block #2 on itself"],
      [seed, "update", { id: 1, addBlockedBy: [2] }, "addBlockedBy would create a cycle in the blockedBy graph"],
      [seed, "get", { id: 9 }, "#9 not found"],
      [seed, "get", {}, "id required for get"]
    ];
    for (const [state, action, params, message] of cases) {
      const result = applyTaskMutation(state, action, params);
      expect(result.op, `${action} ${JSON.stringify(params)}`).toEqual({ kind: "error", message });
      expect(result.state).toBe(state);
    }
  });

  it("reports a no-op update as unchanged so the model stops re-issuing it", () => {
    const moved = applyTaskMutation(seed, "update", { id: 2, status: "completed" });
    expect(moved.op).toMatchObject({ kind: "update", id: 2, fromStatus: "in_progress", toStatus: "completed", changed: true });
    const noop = applyTaskMutation(seed, "update", { id: 2, status: "in_progress" });
    expect(noop.op).toMatchObject({ changed: false });
  });

  it("merges blockedBy additively and removes edges", () => {
    const removed = applyTaskMutation(seed, "update", { id: 2, removeBlockedBy: [1] });
    expect(removed.state.tasks[1]?.blockedBy).toBeUndefined();
    const added = applyTaskMutation(removed.state, "update", { id: 2, addBlockedBy: [1] });
    expect(added.state.tasks[1]?.blockedBy).toEqual([1]);
  });

  it("tombstones on delete, refuses to delete twice, and resets on clear", () => {
    const deleted = applyTaskMutation(seed, "delete", { id: 2 });
    expect(deleted.op).toEqual({ kind: "delete", id: 2, subject: "Second" });
    expect(deleted.state.tasks[1]?.status).toBe("deleted");
    expect(applyTaskMutation(deleted.state, "delete", { id: 2 }).op).toEqual({ kind: "error", message: "#2 is already deleted" });
    expect(applyTaskMutation(seed, "delete", { id: 9 }).op).toEqual({ kind: "error", message: "#9 not found" });
    expect(applyTaskMutation(seed, "delete", {}).op).toEqual({ kind: "error", message: "id required for delete" });
    const cleared = applyTaskMutation(deleted.state, "clear", {});
    expect(cleared.op).toEqual({ kind: "clear", count: 2 });
    expect(cleared.state).toEqual(EMPTY_STATE);
  });
});

describe("todo dependency graph", () => {
  it("inverts blockedBy into blocks edges and previews cycles without mutating", () => {
    const tasks: TodoTask[] = [
      { id: 1, subject: "A", status: "pending", blockedBy: [2] },
      { id: 2, subject: "B", status: "pending" }
    ];
    expect(deriveBlocks(tasks).get(2)).toEqual([1]);
    expect(detectCycle(tasks, 2, [1])).toBe(true);
    expect(detectCycle(tasks, 2, [])).toBe(false);
  });
});

describe("todo response envelope", () => {
  it("formats upstream-identical strings and embeds the replay snapshot in details", () => {
    let state = EMPTY_STATE;
    let result = applyTaskMutation(state, "create", { subject: "Write the parser" });
    state = result.state;
    const created = buildToolResult("create", state, result.op);
    expect(created.content[0]?.text).toBe("Created #1: Write the parser (pending)");
    expect(created.details).toEqual({ version: TODO_DETAILS_VERSION, action: "create", tasks: state.tasks, nextId: 2 });

    result = applyTaskMutation(state, "update", { id: 1, status: "in_progress", activeForm: "writing the parser" });
    state = result.state;
    expect(buildToolResult("update", state, result.op).content[0]?.text).toBe("Updated #1 (pending → in_progress)");

    const noop = applyTaskMutation(state, "update", { id: 1, status: "in_progress" });
    expect(buildToolResult("update", noop.state, noop.op).content[0]?.text)
      .toBe("No change: #1 already matches the requested values (status: in_progress)");

    // Errors are in-band but still carry the (unchanged) state so replay stays consistent.
    const failed = applyTaskMutation(state, "get", { id: 9 });
    const error = buildToolResult("get", failed.state, failed.op);
    expect(error.content[0]?.text).toBe("Error: #9 not found");
    expect(error.details.error).toBe("#9 not found");
    expect(error.details.tasks).toEqual(state.tasks);
  });
});

describe("todo state replay", () => {
  const snapshot = (tasks: unknown[], nextId: number) => ({ version: TODO_DETAILS_VERSION, action: "create", tasks, nextId });
  const toolResult = (details: unknown) => ({ type: "message", message: { role: "toolResult", toolName: "todo", details } });

  it("takes the last snapshot, skips foreign shapes, and falls back to empty", () => {
    const branch: unknown[] = [
      toolResult(snapshot([{ id: 1, subject: "Old", status: "pending" }], 2)),
      toolResult({ tasks: "corrupt" }),
      { type: "message", message: { role: "toolResult", toolName: "other_tool", details: snapshot([{ id: 5, subject: "Foreign", status: "pending" }], 9) } },
      toolResult({ version: 99, tasks: [{ id: 5, subject: "Foreign", status: "pending" }], nextId: 6 }),
      toolResult(snapshot([{ id: 1, subject: "New", status: "completed" }], 3))
    ];
    expect(replayFromBranch(branch)).toEqual({ tasks: [{ id: 1, subject: "New", status: "completed" }], nextId: 3 });
    expect(replayFromBranch([])).toEqual(EMPTY_STATE);
  });

  it("normalizes away malformed task entries", () => {
    const branch: unknown[] = [
      toolResult(snapshot([{ id: 1, subject: "Fine", status: "pending" }, { id: "x" }, null], 2))
    ];
    expect(replayFromBranch(branch).tasks).toEqual([{ id: 1, subject: "Fine", status: "pending" }]);
  });
});

describe("sub-agent parameters", () => {
  it("accepts exactly one mode and trims the inputs", () => {
    expect(normalizeSubagentParams({ agent: " scout ", task: " look " })).toEqual({ ok: true, mode: "single", tasks: [{ agent: "scout", task: "look" }] });
    expect(normalizeSubagentParams({ tasks: [{ agent: "scout", task: "a" }, { agent: "reviewer", task: "b" }] })).toEqual({
      ok: true, mode: "parallel", tasks: [{ agent: "scout", task: "a" }, { agent: "reviewer", task: "b" }],
    });
    expect(normalizeSubagentParams({ agent: "scout", task: "a", tasks: [{ agent: "scout", task: "b" }] }).ok).toBe(false);
    expect(normalizeSubagentParams({ agent: "scout" }).ok).toBe(false);
    expect(normalizeSubagentParams({ tasks: [] }).ok).toBe(false);
    expect(normalizeSubagentParams({ tasks: [{ agent: "scout" }] }).ok).toBe(false);
    expect(normalizeSubagentParams({ tasks: Array.from({ length: 9 }, () => ({ agent: "scout", task: "x" })) }).ok).toBe(false);
    expect(normalizeSubagentParams("scout").ok).toBe(false);
  });

  it("offers only switched-on agents by name", () => {
    const params = subagentParams(["scout", "worker"]) as { properties: { agent: { enum?: string[] }; tasks: { items: { properties: { agent: { enum?: string[] } } } } } };
    expect(params.properties.agent.enum).toEqual(["scout", "worker"]);
    expect(params.properties.tasks.items.properties.agent.enum).toEqual(["scout", "worker"]);
    expect((subagentParams([]) as { properties: { agent: { enum?: string[] } } }).properties.agent.enum).toBeUndefined();
  });

  it("names the tool in every guideline and only delegates on request by default", () => {
    for (const trigger of ["on_request", "auto"] as const) {
      for (const line of subagentGuidelines(trigger)) expect(line).toContain("subagent");
    }
    expect(subagentGuidelines("on_request")[0]).toContain("Only call subagent when the user explicitly asks");
    expect(subagentGuidelines("auto")[0]).toContain("clearly benefits from delegation");
  });
});

describe("sub-agent tools and policy", () => {
  it("gives a child its role's Pi tools, minus unavailable, switched-off and (when read-only) mutating ones", () => {
    const available = ["read", "grep", "ls", "bash", "edit", "write"];
    expect(resolveChildTools(["read", "grep", "find", "ls", "bash"], true, available)).toEqual(["read", "grep", "ls", "bash"]);
    expect(resolveChildTools(["read", "edit", "write", "todo", "subagent"], false, available)).toEqual(["read", "edit", "write"]);
    expect(resolveChildTools(["read", "edit", "write"], true, available)).toEqual(["read"]);
    expect(resolveChildTools(["read", "read"], false, ["ls"])).toEqual([]);
  });

  it("keeps a read-only child on inspection, with Plan mode's shell policy", () => {
    expect(readOnlyDecision("read", { path: "a.ts" }, "/repo")).toBeUndefined();
    expect(readOnlyDecision("bash", { command: "git status && rg TODO" }, "/repo")).toBeUndefined();
    expect(readOnlyDecision("bash", { command: "rm -rf build" }, "/repo")?.reason).toContain("Blocked command: rm -rf build");
    expect(readOnlyDecision("bash", { command: "cat x > y" }, "/repo")?.block).toBe(true);
    expect(readOnlyDecision("write", { path: "a.ts" }, "/repo")?.reason).toContain("cannot use 'write'");
    expect(readOnlyDecision("edit", {}, "/repo")?.block).toBe(true);
  });
});

describe("sub-agent scheduling", () => {
  const tick = () => new Promise((wake) => setTimeout(wake, 5));

  it("caps how many run at once, lets only one writer in at a time, and keeps input order", async () => {
    let running = 0;
    let peak = 0;
    let writers = 0;
    let peakWriters = 0;
    const order: string[] = [];
    const items = [
      { id: "w1", writer: true }, { id: "w2", writer: true }, { id: "r1", writer: false },
      { id: "r2", writer: false }, { id: "r3", writer: false }, { id: "w3", writer: true },
    ];
    const results = await runScheduled(items, 3, (item) => item.writer, async (item) => {
      running += 1;
      peak = Math.max(peak, running);
      if (item.writer) { writers += 1; peakWriters = Math.max(peakWriters, writers); }
      order.push(item.id);
      await tick();
      running -= 1;
      if (item.writer) writers -= 1;
      return item.id.toUpperCase();
    });
    expect(results).toEqual(["W1", "W2", "R1", "R2", "R3", "W3"]);
    expect(peak).toBe(3);
    expect(peakWriters).toBe(1);
    // Read-only tasks queued behind the waiting writer started without waiting for it.
    expect(order.slice(0, 3)).toEqual(["w1", "r1", "r2"]);
  });

  it("finishes every task before reporting a failure, and handles an empty call", async () => {
    const finished: number[] = [];
    await expect(runScheduled([1, 2, 3], 2, () => false, async (item) => {
      await tick();
      finished.push(item);
      if (item === 1) throw new Error("boom");
      return item;
    })).rejects.toThrow("boom");
    expect(finished.sort()).toEqual([1, 2, 3]);
    await expect(runScheduled([], 4, () => false, async () => 1)).resolves.toEqual([]);
  });
});

describe("sub-agent results", () => {
  it("summarizes tool calls for the card and keeps only the latest", () => {
    expect(summarizeActivity("bash", { command: "npm   test\n--run" })).toEqual({ tool: "bash", subject: "npm test --run" });
    expect(summarizeActivity("grep", { pattern: "TODO" })).toEqual({ tool: "grep", subject: "TODO" });
    expect(summarizeActivity("ls", {})).toEqual({ tool: "ls", subject: "." });
    expect(summarizeActivity("read", { path: "src/a.ts" })).toEqual({ tool: "read", subject: "src/a.ts" });
    expect(summarizeActivity("bash", { command: "x".repeat(400) }).subject.length).toBe(160);
    const [result] = createDetails("single", [{ input: { agent: "scout", task: "t" }, readOnly: true }]).results;
    for (let index = 0; index < 20; index += 1) recordActivity(result, { tool: "read", subject: String(index) });
    expect(result.activity).toHaveLength(12);
    expect(result.activity[0].subject).toBe("8");
  });

  it("adds usage into the shape Pi totals tool results with", () => {
    const total = addUsage(emptyUsage(), { input: 10, output: 5, cacheRead: 2, cacheWrite: 1, totalTokens: 18, cost: { input: 0.1, output: 0.2, cacheRead: 0, cacheWrite: 0, total: 0.3 } });
    addUsage(total, { input: 1, output: 1, cost: { total: 0.1 } } as never);
    addUsage(total, undefined);
    expect(total.input).toBe(11);
    expect(total.output).toBe(6);
    expect(total.cost.total).toBeCloseTo(0.4);
  });

  it("gives the model one answer, or a section per child when parallel", () => {
    const single = createDetails("single", [{ input: { agent: "scout", task: "t" }, readOnly: true }]);
    single.results[0].status = "done";
    expect(modelContent(single, ["Found it."])).toBe("Found it.");
    single.results[0].status = "failed";
    single.results[0].error = "The model request failed.";
    expect(modelContent(single, [""])).toBe("scout failed: The model request failed.");

    const parallel = createDetails("parallel", [
      { input: { agent: "scout", task: "a" }, readOnly: true },
      { input: { agent: "worker", task: "b" }, readOnly: false },
    ]);
    parallel.results[0].status = "done";
    parallel.results[1].status = "aborted";
    const text = modelContent(parallel, ["A report", "half"]);
    expect(text).toContain("1/2 sub-agents completed.");
    expect(text).toContain("### [scout] completed\nA report");
    expect(text).toContain("### [worker] stopped\nworker was stopped before it finished.\n\nPartial output:\nhalf");
    expect(truncate("abcdef", 3)).toEqual({ text: "abc\n\n[… 3 more characters]", truncated: true });
  });
});
