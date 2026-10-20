import { describe, expect, it } from "vitest";
import foldCases from "../worker/src/builtin/canvas/fold-cases.json";
import type { NormalizedMessage } from "./types";
import { artboardMention, canvasActivity, canvasTouch, canvasesFromMessages, latestCanvasId, mergeTouches, resolveArtboardHtml, type CanvasBoard, type CanvasDetails } from "./canvas-state";

type Call = { args: unknown; details: CanvasDetails | Record<string, unknown> };

/** Accepted calls shaped as the snapshot carries them: the call, then its result. */
function messages(calls: Call[], options: { refuse?: number } = {}): NormalizedMessage[] {
  return calls.flatMap((call, index) => [
    { id: `a-${index}`, role: "assistant", blocks: [{ type: "tool-call", toolName: "canvas", toolCallId: `call-${index}`, arguments: call.args }] },
    { id: `t-${index}`, role: "tool", blocks: [{ type: "tool-result", toolName: "canvas", toolCallId: `call-${index}`, isError: options.refuse === index, details: call.details, text: "" }] },
  ] as NormalizedMessage[]);
}

function board(canvas: CanvasBoard) {
  return {
    title: canvas.title,
    lastChanged: canvas.lastChanged,
    artboards: canvas.artboards.map((artboard) => ({
      id: artboard.id,
      name: artboard.name,
      group: artboard.group ?? null,
      width: artboard.width,
      height: artboard.height ?? null,
      html: resolveArtboardHtml(canvas, artboard),
    })),
  };
}

describe("canvasesFromMessages (shared with the worker)", () => {
  for (const testCase of foldCases.cases) {
    it(testCase.name, () => {
      const boards = canvasesFromMessages(messages(testCase.calls as Call[]));
      expect(Object.fromEntries([...boards].map(([id, canvas]) => [id, board(canvas)]))).toEqual(testCase.expect);
    });
  }

  it("skips a refused call and never mutates an earlier board", () => {
    const calls = foldCases.cases[0].calls as Call[];
    const first = canvasesFromMessages(messages(calls.slice(0, 1))).get("themes")!;
    const both = canvasesFromMessages(messages(calls)).get("themes")!;
    expect(first.artboards[0].html).toBe("<main>v1</main>");
    expect(both.artboards[0].html).toBe("<main>v2</main>");
    const refused = canvasesFromMessages(messages(calls, { refuse: 1 })).get("themes")!;
    expect(refused.artboards[0].html).toBe("<main>v1</main>");
  });
});

describe("canvas touches", () => {
  const details = (applied: CanvasDetails["applied"], removed: string[] = []): CanvasDetails => ({ v: 1, canvas: "c", title: "C", applied, removed });
  const call = { type: "tool-call" as const, toolName: "canvas", toolCallId: "x" };

  it("reads what one accepted call did", () => {
    expect(canvasTouch(call, { type: "tool-result", details: details([{ index: 0, id: "a", op: "add" }, { index: 1, id: "b", op: "update" }], ["z"]) }))
      .toEqual({ toolCallId: "x", canvas: "c", title: "C", added: ["a"], updated: ["b"], removed: ["z"] });
    expect(canvasTouch(call, { type: "tool-result", isError: true, details: details([]) })).toBeUndefined();
    expect(canvasTouch(call, undefined)).toBeUndefined();
  });

  it("sums a turn's calls per canvas", () => {
    const touch = (added: string[], updated: string[], removed: string[] = [], canvas = "c") => ({ toolCallId: "x", canvas, title: "C", added, updated, removed });
    expect(mergeTouches([touch(["a", "b"], []), touch([], ["a", "c"]), touch([], [], ["b"]), touch(["q"], [], [], "d")])).toEqual([
      { toolCallId: "x", canvas: "c", title: "C", added: ["a"], updated: ["c"], removed: ["b"] },
      { toolCallId: "x", canvas: "d", title: "C", added: ["q"], updated: [], removed: [] },
    ]);
  });

  it("mentions an artboard by its names", () => {
    const canvas: CanvasBoard = { id: "c", title: "Billing", artboards: [], lastChanged: [] };
    expect(artboardMention(canvas, { id: "ember", name: "Ember", width: 1280 })).toBe("“Ember” (canvas “Billing”)");
  });
});

describe("latestCanvasId and canvasActivity", () => {
  const call = (id: string, name = "canvas"): NormalizedMessage => ({ id: `a-${id}`, role: "assistant", blocks: [{ type: "tool-call", toolName: name, toolCallId: id }] } as NormalizedMessage);
  const result = (id: string, canvas: string, name = "canvas"): NormalizedMessage => ({ id: `t-${id}`, role: "tool", blocks: [{ type: "tool-result", toolName: name, toolCallId: id, details: { v: 1, canvas, title: canvas, applied: [], removed: [] } }] } as NormalizedMessage);
  const user: NormalizedMessage = { id: "u", role: "user", blocks: [{ type: "text", text: "go" }] } as NormalizedMessage;

  it("names the canvas drawn on last", () => {
    expect(latestCanvasId([call("1"), result("1", "one"), call("2"), result("2", "two"), call("3"), result("3", "one")])).toBe("one");
    expect(latestCanvasId([user])).toBeUndefined();
  });

  it("reports a call still in flight on the current turn", () => {
    expect(canvasActivity([user, call("1")])).toBe("drawing");
    expect(canvasActivity([user, call("1"), result("1", "one")])).toBeUndefined();
    expect(canvasActivity([user, call("1"), result("1", "one"), call("2", "canvas_look")])).toBe("looking");
    expect(canvasActivity([call("1"), user])).toBeUndefined();
    expect(canvasActivity([user], call("9", "canvas_look"))).toBe("looking");
  });
});
