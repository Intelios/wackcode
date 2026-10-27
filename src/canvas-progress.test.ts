import { describe, expect, it } from "vitest";
import type { NormalizedBlock, NormalizedMessage } from "./types";
import { canvasProgress, pendingCanvasCalls } from "./canvas-progress";

const call: NormalizedBlock = { type: "tool-call", toolName: "canvas", toolCallId: "draw", arguments: { canvas: "ideas", artboards: [{ id: "base", html: "<main>Hi", css: "main{}" }, { name: "Phone", width: 390 }] } };
const assistant = (block: NormalizedBlock): NormalizedMessage => ({ id: "a", role: "assistant", blocks: [block] });
const user: NormalizedMessage = { id: "u", role: "user", blocks: [] };

describe("canvas progress", () => {
  it("counts received markup while JSON and artboards are incomplete", () => {
    expect(canvasProgress({ ...call, arguments: undefined })).toMatchObject({ artboards: [], characters: 0 });
    expect(canvasProgress({ ...call, arguments: { artboards: [null, {}, { html: "<" }] } })).toMatchObject({ characters: 1 });
    expect(canvasProgress(call)).toMatchObject({ canvasId: "ideas", characters: 14, artboards: [{ id: "base", name: "base", characters: 14 }, { name: "Phone", characters: 0 }] });
  });

  it("uses the freshest partial once per call and drops accepted, refused and past-turn calls", () => {
    const fresh = { ...call, arguments: { title: "Fresh" } };
    expect(pendingCanvasCalls([user, assistant(call)], assistant(fresh))).toEqual([fresh]);
    for (const isError of [false, true]) {
      const result: NormalizedMessage = { id: "r", role: "tool", blocks: [{ type: "tool-result", toolCallId: "draw", isError }] };
      expect(pendingCanvasCalls([user, assistant(call), result], assistant(fresh))).toEqual([]);
    }
    expect(pendingCanvasCalls([assistant(call), user])).toEqual([]);
    expect(pendingCanvasCalls([user, { ...assistant(call), stopReason: "aborted" }])).toEqual([]);
    expect(pendingCanvasCalls([user, { ...assistant(call), stopReason: "error" }])).toEqual([]);
  });
});
