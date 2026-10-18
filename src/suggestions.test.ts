import { describe, expect, it } from "vitest";
import type { NormalizedBlock, NormalizedMessage } from "./types";
import {
  SUGGEST_TASK_TOOL_NAME,
  SUGGESTED_TASK_CLOSE,
  SUGGESTED_TASK_OPEN,
  composeSuggestedTask,
  parseSuggestionDetails,
  splitSuggestedTask,
  startedHere,
  suggestionFromCall,
  suggestionLinks,
  turnSuggestions,
} from "./suggestions";

const brief = "**Where:** `src/components/Composer.tsx:88`\n\nThe stop button disables itself after a failed save.";
const card = { id: "call-1", kind: "bug" as const, title: "Dismiss button stays disabled after a failed save", description: brief };

describe("composeSuggestedTask", () => {
  it("puts the title, the brief and one guidance tail in the message", () => {
    const text = composeSuggestedTask(card);
    expect(text.startsWith(`${card.title}\n\n${brief}\n\n${SUGGESTED_TASK_OPEN}\n`)).toBe(true);
    expect(text.endsWith(`\n${SUGGESTED_TASK_CLOSE}`)).toBe(true);
    expect(text).toContain(`Started from a suggested task "${card.title}".`);
    expect(text).toContain('"id":"call-1"');
    expect(text).not.toContain("read_chat");
  });

  it("names the origin chat for a new chat, so the new agent can read the context", () => {
    const text = composeSuggestedTask(card, { chatId: "task-9", chatName: "Settings cleanup" });
    expect(text).toContain("in the chat \"Settings cleanup\"");
    expect(text).toContain("Call read_chat with its id");
    expect(text).toContain('"origin":{"chatId":"task-9","chatName":"Settings cleanup"}');
  });

  it("cannot have its tail ended early by quotes or newlines in a title", () => {
    const text = composeSuggestedTask({ ...card, title: 'He said "stop"\nnow' });
    const { suggestion } = splitSuggestedTask(text);
    expect(suggestion?.title).toBe("He said stop now");
  });
});

describe("splitSuggestedTask", () => {
  it("round-trips the words and the suggestion, with and without an origin", () => {
    const plain = splitSuggestedTask(composeSuggestedTask(card));
    expect(plain.text).toBe(`${card.title}\n\n${brief}`);
    expect(plain.suggestion).toEqual({ id: "call-1", kind: "bug", title: card.title });

    const withOrigin = splitSuggestedTask(composeSuggestedTask(card, { chatId: "task-9", chatName: "Settings cleanup" }));
    expect(withOrigin.suggestion?.origin).toEqual({ chatId: "task-9", chatName: "Settings cleanup" });
  });

  it("leaves text alone when the section is malformed, has no words, or has trailing text", () => {
    const untrusted = `${card.title}\n\n<suggested-task-v1>\nnot json\n</suggested-task-v1>`;
    expect(splitSuggestedTask(untrusted)).toEqual({ text: untrusted });
    const empty = `<suggested-task-v1>\n{"version":1,"id":"x","kind":"bug","title":"T"}\n</suggested-task-v1>`;
    expect(splitSuggestedTask(empty)).toEqual({ text: empty });
    const trailing = `${composeSuggestedTask(card)}\nand more words`;
    expect(splitSuggestedTask(trailing)).toEqual({ text: trailing });
    const foreignKind = composeSuggestedTask(card).replace('"kind":"bug"', '"kind":"chore"');
    expect(splitSuggestedTask(foreignKind).suggestion).toBeUndefined();
  });
});

const text = (value: string): NormalizedBlock => ({ type: "text", text: value });
const call = (id: string, args: Record<string, unknown>): NormalizedBlock => ({ type: "tool-call", toolCallId: id, toolName: SUGGEST_TASK_TOOL_NAME, arguments: args });
const callResult = (id: string, extra: Partial<NormalizedBlock> = {}): NormalizedBlock => ({
  type: "tool-result", toolCallId: id, text: "Suggested", toolName: SUGGEST_TASK_TOOL_NAME,
  details: { v: 1, kind: "bug", title: "T" }, ...extra,
});

describe("suggestion cards", () => {
  it("draws a card only for a settled, accepted call whose arguments parse", () => {
    const ok = call("call-1", { title: "T", kind: "polish", description: brief });
    expect(suggestionFromCall(ok, callResult("call-1"))).toEqual({ id: "call-1", title: "T", kind: "polish", description: brief });
    expect(suggestionFromCall(ok, undefined)).toBeUndefined();
    expect(suggestionFromCall(ok, callResult("call-1", { isError: true }))).toBeUndefined();
    expect(suggestionFromCall(call("call-2", { title: "T", kind: "chore", description: brief }), callResult("call-2"))).toBeUndefined();
    expect(suggestionFromCall(call("call-3", { title: "T", kind: "bug", description: " " }), callResult("call-3"))).toBeUndefined();
  });

  it("collects one turn's suggestions in filing order and skips other tools", () => {
    const messages: NormalizedMessage[] = [
      { id: "u", role: "user", blocks: [text("Fix the layout")] },
      { id: "a1", role: "assistant", blocks: [call("call-1", { title: "One", kind: "bug", description: brief }), call("other", { command: "ls" })] },
      { id: "r1", role: "tool", blocks: [callResult("call-1"), callResult("other")] },
      { id: "a2", role: "assistant", blocks: [call("call-2", { title: "Two", kind: "improvement", description: brief }), call("call-3", { title: "Three", kind: "bug", description: brief })] },
      { id: "r2", role: "tool", blocks: [callResult("call-2"), callResult("call-3", { isError: true })] },
      { id: "a3", role: "assistant", blocks: [text("Done")] },
    ];
    const results = new Map<string, NormalizedBlock>();
    for (const message of messages) for (const block of message.blocks) if (block.type === "tool-result") results.set(block.toolCallId!, block);
    expect(turnSuggestions(messages, results, 1, messages.length).map((entry) => entry.title)).toEqual(["One", "Two"]);
  });

  it("reads a suggestion's versioned details, and refuses anything else", () => {
    expect(parseSuggestionDetails({ v: 1, kind: "improvement", title: "T" })).toEqual({ v: 1, kind: "improvement", title: "T" });
    expect(parseSuggestionDetails({ v: 2, kind: "improvement", title: "T" })).toBeUndefined();
    expect(parseSuggestionDetails({ v: 1, kind: "chore", title: "T" })).toBeUndefined();
    expect(parseSuggestionDetails(undefined)).toBeUndefined();
  });
});

describe("started suggestions", () => {
  it("finds a suggestion started in this chat, and follows a rewind by simply not finding it", () => {
    const started = { id: "u", role: "user" as const, blocks: [text(composeSuggestedTask(card))] };
    expect([...startedHere([started]).keys()]).toEqual(["call-1"]);
    expect(startedHere([]).size).toBe(0);
    const ordinary = { id: "u2", role: "user" as const, blocks: [text("Fix the layout")] };
    expect(startedHere([ordinary]).size).toBe(0);
  });

  it("maps each chat started from a suggestion back to the suggestion it came from", () => {
    const links = suggestionLinks([
      { id: "origin", name: "Settings cleanup", originSuggestion: null },
      { id: "new-1", name: "Dismiss button", originSuggestion: { taskId: "origin", suggestionId: "call-1" } },
      { id: "fork", name: "Dismiss button (fork)", originSuggestion: null },
    ]);
    expect(links.get("origin")?.get("call-1")).toEqual({ taskId: "new-1", name: "Dismiss button" });
    expect(links.get("origin")?.get("call-2")).toBeUndefined();
    expect(links.has("fork")).toBe(false);
  });
});
