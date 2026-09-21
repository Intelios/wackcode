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
