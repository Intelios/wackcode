import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import type { CanvasBoard } from "../canvas-state";
import type { CanvasScope } from "./canvas/CanvasCard";
import { Transcript } from "./Transcript";

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const board: CanvasBoard = { id: "ideas", title: "Ideas", lastChanged: ["base"], artboards: [{ id: "base", name: "Base", width: 800, height: 600, html: "<main>Base</main>" }] };
function canvasTurn(userKey: string, canvas = "ideas", title = "Ideas"): NormalizedMessage[] {
  return [
    { id: userKey, entryId: userKey, role: "user", blocks: [{ type: "text", text: `Draw ${userKey}` }] },
    { id: `${userKey}:call`, role: "assistant", stopReason: "toolUse", blocks: [{ type: "tool-call", toolName: "canvas", toolCallId: `${userKey}:canvas`, arguments: { canvas, artboards: [{ name: "Base", width: 800, height: 600, html: "<main>Base</main>" }] } }] },
    { id: `${userKey}:result`, role: "tool", blocks: [{ type: "tool-result", toolName: "canvas", toolCallId: `${userKey}:canvas`, text: "Drew canvas", details: { v: 1, canvas, title, applied: [{ index: 0, id: "base", op: "add" }], removed: [] } }] },
    { id: `${userKey}:answer`, entryId: `${userKey}:answer`, role: "assistant", stopReason: "stop", blocks: [{ type: "text", text: `Finished ${userKey}` }], turn: { userEntryId: userKey, endEntryId: `${userKey}:answer` } },
  ];
}
const scope = (): CanvasScope => ({ boards: new Map([[board.id, board]]), onOpen: vi.fn() });
const cards = () => screen.queryAllByRole("button", { name: /^Canvas: Ideas\./ });

// The board itself lives in the side panel (`CanvasPanel`); the transcript only keeps cards.
describe("Transcript canvas cards", () => {
  it("shows a card as soon as a call is accepted during a live run, while pending and rejected calls stay as work", () => {
    const all = canvasTurn("u1");
    const canvases = scope();
    const view = render(<Transcript messages={all.slice(0, 2)} running taskId="chat" canvases={canvases} />);
    expect(cards()).toHaveLength(0);
    const rejected = { ...all[2], blocks: [{ ...all[2].blocks[0], isError: true }] };
    view.rerender(<Transcript messages={[...all.slice(0, 2), rejected]} running taskId="chat" canvases={canvases} />);
    expect(cards()).toHaveLength(0);
    view.rerender(<Transcript messages={all.slice(0, 3)} running taskId="chat" canvases={canvases} />);
    expect(cards()).toHaveLength(1);
    expect(screen.queryByRole("region", { name: /Canvas/ })).toBeNull();
  });

  it("keeps one card per turn outside folded work, each opening the panel on what its turn drew", () => {
    const canvases = scope();
    const view = render(<Transcript messages={[...canvasTurn("u1"), ...canvasTurn("u2")]} running={false} taskId="chat" canvases={canvases} />);
    expect(cards()).toHaveLength(2);
    for (const card of cards()) expect(card.closest(".work-transcript-region")).toBeNull();
    fireEvent.click(cards()[0]);
    expect(canvases.onOpen).toHaveBeenCalledWith("ideas", ["base"]);
    view.rerender(<Transcript messages={[...canvasTurn("u1"), ...canvasTurn("u2", "other", "Other")]} running={false} taskId="chat" canvases={canvases} />);
    expect(cards()).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Canvas: Other. This canvas is gone" })).toBeDisabled();
  });
});
