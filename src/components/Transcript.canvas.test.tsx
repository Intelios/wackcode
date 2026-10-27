import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedMessage } from "../types";
import type { CanvasBoard } from "../canvas-state";
import { canvasAnchor } from "../canvas-placement";
import { createCanvasSession } from "../canvas-session";
import type { CanvasScope } from "./canvas/CanvasCard";
import { Transcript } from "./Transcript";

// The surface's gestures/staging have their own component tests. Here a lightweight host
// makes turn ownership, scroll geometry and frame multiplicity observable independently.
vi.mock("./canvas/CanvasInline", () => ({
  CanvasInline: ({ touch, onResizeStart, onResizeEnd }: { touch: { title: string }; onResizeStart?: () => void; onResizeEnd?: () => void }) => <section aria-label={`Live canvas: ${touch.title}`}>
    <div className="canvas-viewport" tabIndex={0} />
    <button onClick={() => { onResizeStart?.(); onResizeEnd?.(); }}>Resize canvas</button>
  </section>,
}));

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const board: CanvasBoard = { id: "ideas", title: "Ideas", lastChanged: ["base"], artboards: [{ id: "base", name: "Base", width: 800, height: 600, html: "<main>Base</main>" }] };
function canvasTurn(userKey: string, canvas = "ideas", title = "Ideas"): NormalizedMessage[] {
  return [
    { id: userKey, entryId: userKey, role: "user", blocks: [{ type: "text", text: `Draw ${userKey}` }] },
    { id: `${userKey}:call`, role: "assistant", stopReason: "toolUse", blocks: [{ type: "tool-call", toolName: "canvas", toolCallId: `${userKey}:canvas`, arguments: { canvas, artboards: [{ name: "Base", width: 800, height: 600, html: "<main>Base</main>" }] } }] },
    { id: `${userKey}:result`, role: "tool", blocks: [{ type: "tool-result", toolName: "canvas", toolCallId: `${userKey}:canvas`, text: "Drew canvas", details: { v: 1, canvas, title, applied: [{ index: 0, id: "base", op: "add" }], removed: [] } }] },
    { id: `${userKey}:answer`, entryId: `${userKey}:answer`, role: "assistant", stopReason: "stop", blocks: [{ type: "text", text: `Finished ${userKey}` }], turn: { userEntryId: userKey, endEntryId: `${userKey}:answer` } },
  ];
}
function scope(): CanvasScope {
  const session = createCanvasSession();
  session.ensure("chat", board.id);
  return { taskId: "chat", session, boards: new Map([[board.id, board]]), onOpen: vi.fn() };
}
const slot = (container: HTMLElement, owner: string, canvas = "ideas") => [...container.querySelectorAll<HTMLElement>("[data-canvas-slot]")]
  .find((element) => element.dataset.transcriptAnchor === canvasAnchor(owner, canvas))!;

describe("Transcript inline canvases", () => {
  it("shows accepted artboards during a live run, while pending and rejected calls stay as work", () => {
    const all = canvasTurn("u1");
    const canvases = scope();
    const view = render(<Transcript messages={all.slice(0, 2)} running taskId="chat" canvases={canvases} />);
    expect(screen.queryByRole("region", { name: "Live canvas: Ideas" })).toBeNull();
    const rejected = { ...all[2], blocks: [{ ...all[2].blocks[0], isError: true }] };
    view.rerender(<Transcript messages={[...all.slice(0, 2), rejected]} running taskId="chat" canvases={canvases} />);
    expect(screen.queryByRole("region", { name: "Live canvas: Ideas" })).toBeNull();
    view.rerender(<Transcript messages={all.slice(0, 3)} running taskId="chat" canvases={canvases} />);
    expect(screen.getByRole("region", { name: "Live canvas: Ideas" })).toBeInTheDocument();
    expect(screen.queryByText("Finished u1")).toBeNull();
    expect(slot(view.container, "u1")).toHaveAttribute("data-canvas-owner", "ideas");
  });

  it("keeps one live board at the latest modifying turn and compact earlier references outside folded work", () => {
    const first = canvasTurn("u1");
    const second = canvasTurn("u2");
    const canvases = scope();
    const view = render(<Transcript messages={first} running={false} taskId="chat" canvases={canvases} />);
    const old = slot(view.container, "u1");
    view.rerender(<Transcript messages={[...first, ...second]} running={false} taskId="chat" canvases={canvases} />);
    expect(screen.getAllByRole("region", { name: "Live canvas: Ideas" })).toHaveLength(1);
    expect(slot(view.container, "u1")).toBe(old);
    expect(old).not.toHaveAttribute("data-canvas-owner");
    expect(slot(view.container, "u2")).toHaveAttribute("data-canvas-owner", "ideas");
    expect(old.closest(".work-transcript-region")).toBeNull();
    expect(slot(view.container, "u2").closest(".work-transcript-region")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "View latest" }));
    expect(canvases.onOpen).toHaveBeenCalledWith("ideas", ["base"]);
  });

  it("defers a locked board's relocation and applies it immediately when expansion ends", () => {
    const first = canvasTurn("u1");
    const canvases = scope();
    const view = render(<Transcript messages={first} running={false} taskId="chat" canvases={canvases} />);
    act(() => canvases.session!.setExpanded("ideas"));
    view.rerender(<Transcript messages={[...first, ...canvasTurn("u2")]} running={false} taskId="chat" canvases={canvases} />);
    expect(slot(view.container, "u1")).toHaveAttribute("data-canvas-owner", "ideas");
    expect(slot(view.container, "u2")).not.toHaveAttribute("data-canvas-owner");
    act(() => canvases.session!.setExpanded(undefined));
    expect(slot(view.container, "u1")).not.toHaveAttribute("data-canvas-owner");
    expect(slot(view.container, "u2")).toHaveAttribute("data-canvas-owner", "ideas");
  });

  it.each(["viewport", "control"])("recovers %s focus at the local reference when Done releases a deferred relocation", (target) => {
    const first = canvasTurn("u1");
    const canvases = scope();
    const view = render(<Transcript messages={first} running={false} taskId="chat" canvases={canvases} />);
    act(() => canvases.session!.activate("ideas"));
    const old = target === "viewport" ? slot(view.container, "u1").querySelector<HTMLElement>(".canvas-viewport")!
      : screen.getByRole("button", { name: "Resize canvas" });
    act(() => old.focus());
    view.rerender(<Transcript messages={[...first, ...canvasTurn("u2")]} running={false} taskId="chat" canvases={canvases} />);
    expect(old).toHaveFocus();
    act(() => canvases.session!.deactivate());
    expect(old.isConnected).toBe(false);
    expect(screen.getByRole("button", { name: "View latest" })).toHaveFocus();
  });

  it("keeps composer focus when outside interaction releases a deferred relocation", () => {
    const first = canvasTurn("u1");
    const canvases = scope();
    const draw = (messages: NormalizedMessage[]) => <><input aria-label="Composer" /><Transcript messages={messages} running={false} taskId="chat" canvases={canvases} /></>;
    const view = render(draw(first));
    act(() => canvases.session!.activate("ideas"));
    act(() => slot(view.container, "u1").querySelector<HTMLElement>(".canvas-viewport")!.focus());
    view.rerender(draw([...first, ...canvasTurn("u2")]));
    const composer = screen.getByRole("textbox", { name: "Composer" });
    act(() => composer.focus());
    act(() => canvases.session!.deactivate());
    expect(slot(view.container, "u2")).toHaveAttribute("data-canvas-owner", "ideas");
    expect(composer).toHaveFocus();
  });

  it("retains pane identity across calls in one turn, and recomputes ownership after rewind", () => {
    const first = canvasTurn("u1");
    const canvases = scope();
    const view = render(<Transcript messages={first.slice(0, 3)} running taskId="chat" canvases={canvases} />);
    const host = screen.getByRole("region", { name: "Live canvas: Ideas" });
    const extra = canvasTurn("ignored").slice(1, 3).map((message) => ({ ...message, id: `extra:${message.id}` }));
    view.rerender(<Transcript messages={[...first.slice(0, 3), ...extra]} running taskId="chat" canvases={canvases} />);
    expect(screen.getByRole("region", { name: "Live canvas: Ideas" })).toBe(host);
    view.rerender(<Transcript messages={[...first, ...canvasTurn("u2")]} running={false} taskId="chat" canvases={canvases} />);
    expect(slot(view.container, "u2")).toHaveAttribute("data-canvas-owner", "ideas");
    view.rerender(<Transcript messages={first} running={false} taskId="chat" canvases={canvases} />);
    expect(slot(view.container, "u1")).toHaveAttribute("data-canvas-owner", "ideas");
    expect(screen.queryByRole("button", { name: "View latest" })).toBeNull();
    view.rerender(<Transcript messages={first.slice(0, 1)} running={false} taskId="chat" canvases={canvases} />);
    expect(screen.queryByRole("region", { name: "Live canvas: Ideas" })).toBeNull();
  });

  it("gives multiple canvases independent live occurrences", () => {
    const canvases = scope();
    canvases.boards = new Map([...canvases.boards, ["other", { ...board, id: "other", title: "Other" }]]);
    const view = render(<Transcript messages={[...canvasTurn("u1"), ...canvasTurn("u2", "other", "Other"), ...canvasTurn("u3")]} running={false} taskId="chat" canvases={canvases} />);
    expect(slot(view.container, "u1")).not.toHaveAttribute("data-canvas-owner");
    expect(slot(view.container, "u2", "other")).toHaveAttribute("data-canvas-owner", "other");
    expect(slot(view.container, "u3")).toHaveAttribute("data-canvas-owner", "ideas");
    expect(screen.getAllByRole("region", { name: /^Live canvas:/ })).toHaveLength(2);
  });

  it("preserves detached reading below an earlier pane when that pane becomes a compact reference", () => {
    const first = canvasTurn("u1");
    const second = canvasTurn("u2");
    const canvases = scope();
    const view = render(<Transcript messages={[...first, second[0]]} running={false} taskId="chat" canvases={canvases} />);
    const scroller = view.container.querySelector<HTMLElement>(".conversation-scroll")!;
    Object.defineProperties(scroller, { clientHeight: { value: 400 }, scrollHeight: { value: 3_000 } });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this === scroller) return { top: 0, bottom: 400, height: 400 } as DOMRect;
      const oldOwner = slot(view.container, "u1").hasAttribute("data-canvas-owner");
      const key = this.dataset.transcriptAnchor;
      const y = key === "message:u2" ? (oldOwner ? 600 : 240)
        : key === canvasAnchor("u1", "ideas") ? 100 : 0;
      const height = key === canvasAnchor("u1", "ideas") ? (oldOwner ? 420 : 60) : key === "message:u2" ? 60 : 0;
      return { top: y - scroller.scrollTop, bottom: y - scroller.scrollTop + height, height } as DOMRect;
    });
    scroller.scrollTop = 550;
    fireEvent.wheel(scroller, { deltaY: -4 });
    fireEvent.scroll(scroller);
    view.rerender(<Transcript messages={[...first, ...second]} running={false} taskId="chat" canvases={canvases} />);
    expect(scroller.scrollTop).toBe(190);
    expect(view.container.querySelector<HTMLElement>('[data-transcript-anchor="message:u2"]')!.getBoundingClientRect().top).toBe(50);
    view.rerender(<Transcript messages={[...first, ...second, { id: "later", role: "assistant", blocks: [{ type: "text", text: "More explanation" }] }]} running={false} taskId="chat" canvases={canvases} />);
    expect(scroller.scrollTop).toBe(190);
  });

  it("navigates a focus request once and lets later background revisions stay quiet", () => {
    const canvases = scope();
    const first = canvasTurn("u1");
    const view = render(<Transcript messages={first} running={false} taskId="chat" canvases={canvases} />);
    const scroller = view.container.querySelector<HTMLElement>(".conversation-scroll")!;
    const scrollTo = vi.fn();
    Object.defineProperty(scroller, "scrollTo", { value: scrollTo });
    const request = { ...canvases, focus: { canvasId: "ideas", ids: ["base"], nonce: 1 } };
    view.rerender(<Transcript messages={first} running={false} taskId="chat" canvases={request} />);
    expect(scrollTo).toHaveBeenCalledTimes(1);
    expect(slot(view.container, "u1").querySelector(".canvas-viewport")).toHaveFocus();
    view.rerender(<Transcript messages={[...first, ...canvasTurn("u2")]} running={false} taskId="chat" canvases={request} />);
    expect(scrollTo).toHaveBeenCalledTimes(1);
  });

  it("pauses bottom-following during a manual resize and holds the pane's top through later frames", () => {
    const canvases = scope();
    const messages = canvasTurn("u1");
    const view = render(<Transcript messages={messages} running={false} taskId="chat" canvases={canvases} />);
    const scroller = view.container.querySelector<HTMLElement>(".conversation-scroll")!;
    const pane = slot(view.container, "u1");
    Object.defineProperties(scroller, { clientHeight: { value: 400 }, scrollHeight: { value: 3_000 } });
    scroller.getBoundingClientRect = () => ({ top: 0, bottom: 400 } as DOMRect);
    let y = 300;
    pane.getBoundingClientRect = () => ({ top: y - scroller.scrollTop, bottom: y - scroller.scrollTop + 420 } as DOMRect);
    scroller.scrollTop = 250;
    fireEvent.click(screen.getByRole("button", { name: "Resize canvas" }));
    expect(scroller.scrollTop).toBe(250);
    y += 40;
    view.rerender(<Transcript messages={messages} running={false} taskId="chat" canvases={canvases} />);
    expect(scroller.scrollTop).toBe(290);
    expect(pane.getBoundingClientRect().top).toBe(50);
  });
});
