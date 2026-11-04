import { describe, expect, it } from "vitest";
import { canvasDocuments } from "./canvas-documents.js";
import { CANVAS_TOOL_NAME, type CanvasDetails } from "./builtin/canvas/state.js";

/** One accepted call shaped as Pi saves it: the assistant's call, then its result. */
function branch(calls: Array<{ args: unknown; details: CanvasDetails }>) {
  return calls.flatMap((call, index) => [
    { type: "message", message: { role: "assistant", content: [{ type: "toolCall", id: `call-${index}`, name: CANVAS_TOOL_NAME, arguments: call.args }] } },
    { type: "message", message: { role: "toolResult", toolName: CANVAS_TOOL_NAME, toolCallId: `call-${index}`, details: call.details } },
  ]);
}

/** A call that added artboards and saved inlined markup for some of them. */
function call(entries: Array<{ id: string; saved?: { html?: string; css?: string } }>): { args: unknown; details: CanvasDetails } {
  return {
    args: { title: "One", artboards: entries.map((entry) => ({ name: entry.id.toUpperCase(), width: 400, html: `<main>${entry.id}</main>` })) },
    details: {
      v: 1,
      canvas: "one",
      title: "One",
      applied: entries.map((entry, index) => ({ index, id: entry.id, op: "add" as const })),
      removed: [],
      documents: entries.flatMap((entry) => entry.saved ? [{ id: entry.id, ...entry.saved }] : []),
    },
  };
}

describe("saved canvas documents", () => {
  it("returns the current fields of every flagged artboard, and only those", () => {
    const calls = [call([
      { id: "hero", saved: { html: "<main>inlined</main>" } },
      { id: "plain" },
    ])];
    expect(canvasDocuments(branch(calls), "one")).toEqual({ v: 1, canvas: "one", artboards: [{ id: "hero", html: "<main>inlined</main>" }] });
  });

  it("carries a variant's saved css beside its base's saved html", () => {
    const calls = [{
      args: {
        title: "One",
        artboards: [
          { name: "Base", width: 400, html: "<main>b</main>" },
          { name: "Dusk", width: 400, basedOn: "base", css: "body{background:url('/tmp/t.png')}" },
        ],
      },
      details: {
        v: 1 as const,
        canvas: "one",
        title: "One",
        applied: [
          { index: 0, id: "base", op: "add" as const },
          { index: 1, id: "dusk", op: "add" as const, basedOn: "base" },
        ],
        removed: [],
        documents: [{ id: "base", html: "<main>inlined</main>" }, { id: "dusk", css: "body{background:url('data:image/png;base64,eA==')}" }],
      },
    }];
    expect(canvasDocuments(branch(calls), "one")).toEqual({
      v: 1,
      canvas: "one",
      artboards: [
        { id: "base", html: "<main>inlined</main>" },
        { id: "dusk", css: "body{background:url('data:image/png;base64,eA==')}" },
      ],
    });
  });

  it("drops a field a later plain revision replaced", () => {
    const calls = [
      call([{ id: "hero", saved: { html: "<main>inlined</main>" } }]),
      {
        args: { canvas: "one", artboards: [{ id: "hero", html: "<main>plain now</main>" }] },
        details: { v: 1 as const, canvas: "one", title: "One", applied: [{ index: 0, id: "hero", op: "update" as const }], removed: [], documents: undefined },
      },
    ];
    expect(canvasDocuments(branch(calls), "one")).toEqual({ v: 1, canvas: "one", artboards: [] });
  });

  it("refuses a canvas that is not on the branch", () => {
    const calls = [call([{ id: "hero" }])];
    expect(() => canvasDocuments(branch(calls), "nope")).toThrow("That canvas is no longer in this chat.");
    expect(() => canvasDocuments([], "one")).toThrow("That canvas is no longer in this chat.");
  });
});
