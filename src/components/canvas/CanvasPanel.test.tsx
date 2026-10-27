import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { CanvasBoard } from "../../canvas-state";
import { CanvasPanel, CanvasExpanded } from "./CanvasPanel";
import { CanvasInline } from "./CanvasInline";
import { createCanvasSession, canvasHeightStorageKey } from "../../canvas-session";
import { CanvasCard, CanvasScopeContext, type CanvasScope } from "./CanvasCard";

vi.mock("../../api", () => ({
  api: { stageVisual: vi.fn().mockResolvedValue("wcvisual://localhost/abc123") }
}));

const board: CanvasBoard = {
  id: "themes",
  title: "Billing themes",
  lastChanged: ["ember"],
  artboards: [
    { id: "base", name: "Base", group: "Desktop", width: 1280, height: 800, html: "<main>base</main>" },
    { id: "ember", name: "Ember", group: "Desktop", width: 1280, height: 800, basedOn: "base", css: ":root{--bg:#000}", notes: "Warm" },
    { id: "phone", name: "Phone", group: "Phone", width: 390, basedOn: "base" },
  ],
};

let viewportWidth = 800;
let viewportHeight = 600;
beforeEach(() => {
  viewportWidth = 800;
  viewportHeight = 600;
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: () => void) {}
    observe() { this.callback(); }
    disconnect() {}
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => viewportWidth });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => viewportHeight });
  vi.mocked(api.stageVisual).mockClear();
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function panel(overrides: Partial<Parameters<typeof CanvasPanel>[0]> = {}) {
  const props = {
    taskId: "t1", board, session: createCanvasSession(), activity: undefined, expanded: false,
    onExpand: vi.fn(), onMention: vi.fn(), ...overrides,
  };
  return { props, view: render(<CanvasPanel {...props} />) };
}

describe("CanvasPanel", () => {
  it("lists every artboard by name and size, and stages each as an artboard document", async () => {
    panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    const list = within(region).getByRole("list", { name: "Artboards" });
    expect(within(list).getByRole("button", { name: "Base, 1280 by 800" })).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: "Ember, 1280 by 800. Warm" })).toBeInTheDocument();
    expect(within(list).getByRole("button", { name: "Phone, 390 by auto" })).toBeInTheDocument();
    await waitFor(() => expect(api.stageVisual).toHaveBeenCalledTimes(3));
    const calls = vi.mocked(api.stageVisual).mock.calls;
    expect(calls.every((call) => call[2] === "artboard")).toBe(true);
    // A variant is its base's markup with its own styles after it.
    expect(calls.map((call) => call[0])).toContain("<main>base</main>\n<style>:root{--bg:#000}</style>");
  });

  it("selects with a click or the arrow keys, and mentions an artboard in the draft", () => {
    const { props } = panel();
    const ember = screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" });
    fireEvent.click(ember);
    expect(ember).toHaveAttribute("aria-pressed", "true");
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    fireEvent.keyDown(region, { key: "ArrowLeft" });
    expect(screen.getByRole("button", { name: "Base, 1280 by 800" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(region, { key: "ArrowDown" });
    expect(screen.getByRole("button", { name: "Phone, 390 by auto" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Mention “Ember” in chat" }));
    expect(props.onMention).toHaveBeenCalledWith("“Ember” (canvas “Billing themes”)");
  });

  it("steps into an artboard with Enter and back out with Esc", () => {
    panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    fireEvent.click(screen.getByRole("button", { name: "Base, 1280 by 800" }));
    fireEvent.keyDown(region, { key: "Enter" });
    expect(screen.getByText(/Interacting with “Base”/)).toBeInTheDocument();
    fireEvent.keyDown(region, { key: "Escape" });
    expect(screen.queryByText(/Interacting with/)).not.toBeInTheDocument();
  });

  it("condenses to a floating expand control, with no toolbar", () => {
    const { props } = panel();
    expect(screen.queryByText("Canvas · 3 artboards")).toBeNull();
    expect(screen.queryByRole("group", { name: "Zoom" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Interact with canvas" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
    expect(props.onExpand).toHaveBeenCalledOnce();
  });

  it("floats a restore control over the expanded board", () => {
    const { props } = panel({ expanded: true });
    fireEvent.click(screen.getByRole("button", { name: "Restore canvas" }));
    expect(props.onExpand).toHaveBeenCalledOnce();
  });

  it("finishes artboard effects even when another snapshot repeats the board", () => {
    vi.useFakeTimers();
    const { view, props } = panel();
    const next: CanvasBoard = { ...board, artboards: [...board.artboards, { id: "new", name: "New", width: 400, height: 300, html: "new" }] };
    view.rerender(<CanvasPanel {...props} board={next} />);
    expect(view.container.querySelector(".canvas-artboard.pop")).not.toBeNull();
    view.rerender(<CanvasPanel {...props} board={{ ...next }} />);
    act(() => { vi.advanceTimersByTime(1500); });
    expect(view.container.querySelector(".canvas-artboard.pop")).toBeNull();
  });

  it("says the agent is at work on it", () => {
    panel({ activity: "looking" });
    expect(screen.getByText("Checking artboards")).toHaveAttribute("role", "status");
  });

  it("offers to fly to new artboards once the user has panned away", () => {
    const { view, props } = panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    fireEvent.keyDown(region, { key: "Enter" });
    act(() => {
      region.dispatchEvent(new WheelEvent("wheel", { deltaX: 40, deltaY: 40, bubbles: true, cancelable: true }));
    });
    const next: CanvasBoard = { ...board, artboards: [...board.artboards, { id: "pine", name: "Pine", group: "Desktop", width: 1280, height: 800, basedOn: "base" }] };
    view.rerender(<CanvasPanel {...props} board={next} />);
    expect(screen.getByRole("button", { name: "1 new or revised artboard" })).toBeInTheDocument();
  });
});

describe("CanvasCard", () => {
  it("names what a turn drew and opens the canvas at those artboards", () => {
    const onOpen = vi.fn();
    render(
      <CanvasScopeContext.Provider value={{ boards: new Map([[board.id, board]]), onOpen }}>
        <CanvasCard touch={{ toolCallId: "c1", canvas: "themes", title: "Billing themes", added: ["ember", "phone"], updated: ["base"], removed: [] }} />
      </CanvasScopeContext.Provider>
    );
    expect(screen.getByRole("figure", { name: "Canvas: Billing themes" })).toBeInTheDocument();
    expect(screen.getByText("Added 2 · Revised 1")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "View latest" }));
    expect(onOpen).toHaveBeenCalledWith("themes", ["base", "ember", "phone"]);
  });

  it("says when its canvas is gone", () => {
    render(<CanvasCard touch={{ toolCallId: "c1", canvas: "gone", title: "Old", added: ["a"], updated: [], removed: [] }} />);
    expect(screen.getByText("This canvas is gone")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "View latest" })).toBeDisabled();
  });
});

describe("inline Canvas interaction and rehosting", () => {
  it("leaves idle wheel events to the chat and consumes active wheel without bubbling", () => {
    const onWheel = vi.fn();
    const session = createCanvasSession();
    render(<div onWheel={onWheel}><CanvasPanel taskId="t1" board={board} session={session} onExpand={() => {}} onMention={() => {}} /></div>);
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    const before = session.get(board.id)!.camera;
    const idle = new WheelEvent("wheel", { deltaY: -50, bubbles: true, cancelable: true });
    act(() => { region.dispatchEvent(idle); });
    expect(idle.defaultPrevented).toBe(false);
    expect(onWheel).toHaveBeenCalledOnce();
    expect(session.get(board.id)!.camera).toEqual(before);
    fireEvent.keyDown(region, { key: "Enter" });
    expect(session.isLocked(board.id)).toBe(true);
    const active = new WheelEvent("wheel", { deltaY: -50, bubbles: true, cancelable: true });
    act(() => { region.dispatchEvent(active); });
    expect(active.defaultPrevented).toBe(true);
    expect(onWheel).toHaveBeenCalledOnce();
    expect(session.get(board.id)!.camera.y).toBe(before.y + 50);
    fireEvent.keyDown(region, { key: "Escape" });
    expect(session.isLocked(board.id)).toBe(false);
  });

  it("ends inline interaction when focus or a click moves outside", () => {
    const { props } = panel();
    const enter = () => fireEvent.keyDown(screen.getByRole("region", { name: "Canvas: Billing themes" }), { key: "Enter" });
    enter();
    fireEvent.pointerDown(document.body);
    expect(props.session.isLocked(board.id)).toBe(false);
    enter();
    fireEvent.focusIn(document.body);
    expect(props.session.isLocked(board.id)).toBe(false);
  });

  it("unlocks when the app loses focus even if the active element is an artboard frame", async () => {
    const { props } = panel();
    fireEvent.keyDown(screen.getByRole("region", { name: "Canvas: Billing themes" }), { key: "Enter" });
    const frame = await waitFor(() => {
      const element = screen.getAllByTitle("Base").find((item) => item instanceof HTMLIFrameElement);
      expect(element).toBeDefined();
      return element!;
    });
    frame.focus();
    expect(document.activeElement).toBe(frame);
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    vi.useFakeTimers();
    fireEvent(window, new Event("blur"));
    act(() => { vi.advanceTimersByTime(0); });
    expect(props.session.isLocked(board.id)).toBe(true);
    hasFocus.mockReturnValue(false);
    fireEvent(window, new Event("blur"));
    act(() => { vi.advanceTimersByTime(0); });
    expect(props.session.isLocked(board.id)).toBe(false);
    hasFocus.mockRestore();
  });

  it("preserves selection, arrivals, zoom and world center when an expanded host replaces the inline host", () => {
    const session = createCanvasSession();
    const { view } = panel({ session, focus: { ids: ["base"], nonce: 1 } });
    fireEvent.click(screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" }));
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    act(() => { region.dispatchEvent(new WheelEvent("wheel", { deltaX: 30, deltaY: 50, bubbles: true, cancelable: true })); });
    act(() => { session.update(board.id, { arrivals: ["base"] }); });
    const before = session.get(board.id)!.camera;
    view.unmount();
    viewportWidth = 1200;
    viewportHeight = 800;
    const onRestore = vi.fn();
    render(<CanvasExpanded canvasId={board.id} scope={{ taskId: "t1", boards: new Map([[board.id, board]]), session, onRestore, focus: { canvasId: board.id, ids: ["base"], nonce: 1 } }} />);
    expect(session.get(board.id)!.camera).toEqual({ zoom: before.zoom, x: before.x + 200, y: before.y + 100 });
    expect(screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "1 new or revised artboard" })).toBeInTheDocument();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onRestore).toHaveBeenCalledOnce();
  });

  it("keeps only a placeholder inline while expanded and remembers keyboard-resized height", async () => {
    const session = createCanvasSession();
    const onResizeStart = vi.fn();
    const onResizeEnd = vi.fn();
    const scope: CanvasScope = { taskId: "t1", boards: new Map([[board.id, board]]), session };
    const touch = { toolCallId: "c1", canvas: board.id, title: board.title, added: ["base"], updated: [], removed: [] };
    const draw = (expandedId?: string) => <CanvasScopeContext.Provider value={{ ...scope, expandedId }}>
      <div className="conversation-scroll"><CanvasInline touch={touch} ownerKey="u1" onResizeStart={onResizeStart} onResizeEnd={onResizeEnd} /></div>
    </CanvasScopeContext.Provider>;
    const view = render(draw());
    const handle = screen.getByRole("separator", { name: "Canvas height" });
    expect(handle).toHaveAttribute("aria-valuenow", "420");
    fireEvent.keyDown(handle, { key: "ArrowDown" });
    expect(handle).toHaveAttribute("aria-valuenow", "444");
    expect(localStorage.getItem(canvasHeightStorageKey("t1", board.id))).toBe("444");
    expect(onResizeStart).toHaveBeenCalledOnce();
    expect(onResizeEnd).toHaveBeenCalledOnce();
    await waitFor(() => expect(api.stageVisual).toHaveBeenCalled());
    view.rerender(draw(board.id));
    expect(screen.queryByRole("region", { name: "Canvas: Billing themes" })).not.toBeInTheDocument();
    expect(screen.getByText("Expanded above the composer")).toBeInTheDocument();
    view.rerender(draw());
    expect(screen.getByRole("separator", { name: "Canvas height" })).toHaveAttribute("aria-valuenow", "444");
    view.unmount();
    const restored = createCanvasSession();
    expect(restored.ensure("t1", board.id).height).toBe(444);
  });

  it("does not stage artboards until its transcript pane approaches the viewport", async () => {
    let show: IntersectionObserverCallback | undefined;
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { show = callback; }
      observe() {}
      disconnect() {}
    });
    const session = createCanvasSession();
    render(<CanvasScopeContext.Provider value={{ taskId: "t1", boards: new Map([[board.id, board]]), session }}>
      <CanvasInline touch={{ toolCallId: "c1", canvas: board.id, title: board.title, added: ["base"], updated: [], removed: [] }} ownerKey="u1" />
    </CanvasScopeContext.Provider>);
    expect(api.stageVisual).not.toHaveBeenCalled();
    act(() => { show?.([{ isIntersecting: true }] as IntersectionObserverEntry[], {} as IntersectionObserver); });
    await waitFor(() => expect(api.stageVisual).toHaveBeenCalledTimes(3));
  });
});
