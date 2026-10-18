import { describe, expect, it } from "vitest";
import {
  MAX_SUGGESTION_DESCRIPTION,
  MAX_SUGGESTION_TITLE,
  MAX_SUGGESTIONS_PER_RUN,
  SUGGEST_TASK_GUIDE_LINES,
  SUGGEST_TASK_TOOL_NAME,
  SUGGEST_TASK_TOOL_NAMES,
  createSuggestTaskExtension,
  normalizeSuggestTaskParams,
  suggestionKey,
  suggestedTitlesFromBranch,
} from "./builtin/suggest-task.js";
import { createBuiltinExtensions, SWITCHABLE_BUILTIN_TOOLS } from "./builtin/index.js";
import { CHAT_BUILTIN_TOOLS, chatToolAllowed } from "./builtin/chat-mode/policy.js";
import { stripTailSections } from "./chat-history.js";

const brief = "**Where:** `src/components/Composer.tsx:88`\n\nThe stop button disables itself after a failed save.";

describe("suggest_task params", () => {
  it("accepts a title, a kind and a brief, normalising the title's whitespace", () => {
    expect(normalizeSuggestTaskParams({ title: "  Dismiss   stays disabled ", kind: "bug", description: brief })).toEqual({
      ok: true,
      suggestion: { title: "Dismiss stays disabled", kind: "bug", description: brief },
    });
  });

  it("refuses a missing or over-long title, a foreign kind, and an empty or over-long brief", () => {
    expect(normalizeSuggestTaskParams({ kind: "bug", description: brief })).toEqual({ ok: false, error: "title must be a short non-empty string." });
    expect(normalizeSuggestTaskParams({ title: "   ", kind: "bug", description: brief }).ok).toBe(false);
    expect(normalizeSuggestTaskParams({ title: "x".repeat(MAX_SUGGESTION_TITLE + 1), kind: "bug", description: brief })).toEqual({
      ok: false,
      error: `title must be at most ${MAX_SUGGESTION_TITLE} characters.`,
    });
    expect(normalizeSuggestTaskParams({ title: "T", kind: "chore", description: brief })).toEqual({
      ok: false,
      error: "kind must be one of bug, polish, improvement.",
    });
    expect(normalizeSuggestTaskParams({ title: "T", kind: "bug", description: "  " })).toEqual({
      ok: false,
      error: "description must be a non-empty Markdown brief.",
    });
    const tooLong = normalizeSuggestTaskParams({ title: "T", kind: "bug", description: "x".repeat(MAX_SUGGESTION_DESCRIPTION + 1) });
    expect(tooLong.ok).toBe(false);
    expect(tooLong.ok === false && tooLong.error).toContain(`over the ${MAX_SUGGESTION_DESCRIPTION} limit`);
    expect(normalizeSuggestTaskParams({ title: "T", kind: "bug", description: "x".repeat(MAX_SUGGESTION_DESCRIPTION) }).ok).toBe(true);
    expect(normalizeSuggestTaskParams(null)).toEqual({ ok: false, error: "Expected an object with a title, a kind and a description." });
  });

  it("treats case and spacing as the same title", () => {
    expect(suggestionKey("Dismiss  STAYS disabled")).toBe(suggestionKey("dismiss stays disabled"));
  });
});

/** A minimal ExtensionAPI stand-in: the extension registers one tool and one lifecycle hook. */
function register() {
  const tools: Array<{ name: string; description: string; parameters: unknown; execute: (id: string, params: unknown, signal?: unknown, onUpdate?: unknown, ctx?: unknown) => Promise<unknown> }> = [];
  const hooks: Record<string, () => void> = {};
  const pi = {
    registerTool: (tool: (typeof tools)[number]) => tools.push(tool),
    on: (name: string, handler: () => void) => { hooks[name] = handler; },
  };
  createSuggestTaskExtension()(pi as never);
  return { tool: tools[0], hooks };
}

/** A branch holding accepted suggestions, shaped as Pi saves them. */
function branch(titles: string[]) {
  return titles.map((title, index) => ({
    type: "message",
    message: { role: "toolResult", toolName: SUGGEST_TASK_TOOL_NAME, details: { v: 1, kind: "bug", title }, toolCallId: `call-${index}` },
  }));
}

const context = (titles: string[]) => ({ sessionManager: { getBranch: () => branch(titles) } });

describe("suggest_task tool", () => {
  it("registers one tool that records the card's identity, not its brief", async () => {
    const { tool } = register();
    expect(tool.name).toBe(SUGGEST_TASK_TOOL_NAME);
    expect(SUGGEST_TASK_TOOL_NAMES).toEqual([SUGGEST_TASK_TOOL_NAME]);
    const result = (await tool.execute("call-1", { title: "Dismiss stays disabled", kind: "bug", description: brief }, undefined, undefined, context([]))) as {
      content: Array<{ type: string; text: string }>;
      details: { v: number; kind: string; title: string };
    };
    expect(result.details).toEqual({ v: 1, kind: "bug", title: "Dismiss stays disabled" });
    expect(result.content[0].text).toContain("Suggested “Dismiss stays disabled”");
    expect(result.content[0].text).toContain("Don't work on it now");
    // The brief stays in the call's arguments, which Pi saves; repeating it would double it.
    expect(JSON.stringify(result)).not.toContain("Composer.tsx");
  });

  it("refuses a refused call, so the transcript shows the error instead of a card", async () => {
    const { tool } = register();
    await expect(tool.execute("call-1", { title: "", kind: "bug", description: brief })).rejects.toThrow("title must be a short non-empty string.");
  });

  it("refuses a title already suggested on this branch, whatever its case", async () => {
    const { tool } = register();
    await expect(tool.execute("call-1", { title: "DISMISS stays disabled", kind: "polish", description: brief }, undefined, undefined, context(["Dismiss stays disabled"])))
      .rejects.toThrow("is already suggested in this chat");
  });

  it("caps one run at five suggestions and starts counting again after the run settles", async () => {
    const { tool, hooks } = register();
    for (let index = 0; index < MAX_SUGGESTIONS_PER_RUN; index += 1) {
      await tool.execute(`call-${index}`, { title: `Task ${index}`, kind: "improvement", description: brief });
    }
    await expect(tool.execute("call-six", { title: "Task six", kind: "improvement", description: brief }))
      .rejects.toThrow(`This run already filed ${MAX_SUGGESTIONS_PER_RUN} suggested tasks`);
    expect(hooks.agent_end).toBeTypeOf("function");
    hooks.agent_end();
    await expect(tool.execute("call-seven", { title: "Task seven", kind: "improvement", description: brief })).resolves.toBeDefined();
  });

  it("reads the titles the branch already recorded, ignoring refused calls", () => {
    const accepted = branch(["One", "Two"]);
    const refused = { type: "message", message: { role: "toolResult", toolName: SUGGEST_TASK_TOOL_NAME, isError: true, details: undefined } };
    const other = { type: "message", message: { role: "toolResult", toolName: "todo", details: { v: 1, kind: "bug", title: "Not ours" } } };
    expect(suggestedTitlesFromBranch([...accepted, refused, other, { type: "session" }])).toEqual(["One", "Two"]);
  });
});

describe("suggest_task policy wiring", () => {
  it("is switchable, so Settings › Tools can take it away", () => {
    expect(SWITCHABLE_BUILTIN_TOOLS.has(SUGGEST_TASK_TOOL_NAME)).toBe(true);
  });

  it("loads in a code chat but never in a Chat mode chat", () => {
    const builtins = createBuiltinExtensions({} as never);
    const names = (list: { name: string }[]) => list.map((extension) => extension.name);
    expect(names(builtins.factories)).toContain("wackcode-suggest-task");
    expect(names(builtins.chatFactories)).not.toContain("wackcode-suggest-task");
    // Belt and braces: the Chat mode guard would refuse it even if it were registered.
    expect(CHAT_BUILTIN_TOOLS.has(SUGGEST_TASK_TOOL_NAME)).toBe(false);
    expect(chatToolAllowed(SUGGEST_TASK_TOOL_NAME, "wackcode")).toBe(false);
  });

  it("says what it is for, and what it is not for, in its own words", () => {
    const text = SUGGEST_TASK_GUIDE_LINES.join(" ");
    expect(text).toContain("outside the task you were asked to do");
    expect(text).toContain("Never use it for work you are going to do in this task");
    expect(text).toContain("Prefer no suggestion to a weak one");
  });
});

describe("read_chat rendering", () => {
  it("collapses a started suggested task's section to a marker", () => {
    const text = "Fix the labels\n\n<suggested-task-v1>\nStarted from a suggested task \"Checkout labels\".\n{\"version\":1}\n</suggested-task-v1>";
    expect(stripTailSections(text)).toBe("Fix the labels\n\n[started suggested task: Checkout labels]");
  });

  it("leaves an ordinary message alone", () => {
    expect(stripTailSections("just words")).toBe("just words");
  });
});
