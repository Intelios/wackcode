import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { CanvasBoard } from "../../canvas-state";
import { CanvasPanel } from "./CanvasPanel";
import { createCanvasSession } from "../../canvas-session";
import { CanvasCard, CanvasScopeContext } from "./CanvasCard";

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
    taskId: "t1", boards: [board], board, session: createCanvasSession(), activity: undefined,
    onSelectCanvas: vi.fn(), onMention: vi.fn(), onExport: vi.fn(), ...overrides,
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

  it("plays an artboard with Enter or its Play button, and Esc stops, then clears the selection", () => {
    panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    fireEvent.click(screen.getByRole("button", { name: "Base, 1280 by 800" }));
    fireEvent.keyDown(region, { key: "Enter" });
    expect(screen.getByText(/Playing “Base”/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop playing “Base”" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(region, { key: "Escape" });
    expect(screen.queryByText(/Playing/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Base, 1280 by 800" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.keyDown(region, { key: "Escape" });
    expect(screen.getByRole("button", { name: "Base, 1280 by 800" })).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(screen.getByRole("button", { name: "Play “Phone”" }));
    expect(screen.getByText(/Playing “Phone”/)).toBeInTheDocument();
  });

  it("always owns the wheel: it pans the board and never scrolls what's behind it", () => {
    const onWheel = vi.fn();
    const session = createCanvasSession();
    render(<div onWheel={onWheel}><CanvasPanel taskId="t1" boards={[board]} board={board} session={session} onSelectCanvas={vi.fn()} onMention={vi.fn()} onExport={vi.fn()} /></div>);
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    const before = session.get(board.id)!.camera;
    const wheel = new WheelEvent("wheel", { deltaY: -50, bubbles: true, cancelable: true });
    act(() => { region.dispatchEvent(wheel); });
    expect(wheel.defaultPrevented).toBe(true);
    expect(onWheel).not.toHaveBeenCalled();
    expect(session.get(board.id)!.camera.y).toBe(before.y + 50);
  });

  it("switches between Select and Hand from the toolbar or with V and H; Hand never selects", () => {
    const { props } = panel();
    const select = screen.getByRole("button", { name: "Select" });
    const hand = screen.getByRole("button", { name: "Hand" });
    expect(select).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(hand);
    expect(hand).toHaveAttribute("aria-pressed", "true");
    expect(props.session.get(board.id)!.tool).toBe("hand");
    const base = screen.getByRole("button", { name: "Base, 1280 by 800" });
    const frame = base.closest<HTMLElement>("[data-artboard]")!;
    fireEvent.pointerDown(frame, { button: 0, pointerId: 1 });
    fireEvent.pointerUp(frame, { button: 0, pointerId: 1 });
    expect(base).toHaveAttribute("aria-pressed", "false");
    fireEvent.keyDown(screen.getByRole("region", { name: "Canvas: Billing themes" }), { key: "v" });
    expect(select).toHaveAttribute("aria-pressed", "true");
  });

  it("zooms from the view toolbar and its menu", () => {
    const { props } = panel();
    const level = () => screen.getByRole("button", { name: /^Zoom \d+%/ });
    const before = props.session.get(board.id)!.camera.zoom;
    expect(level()).toHaveAccessibleName(`Zoom ${Math.round(before * 100)}%. Zoom options`);
    fireEvent.click(level());
    expect(screen.getByRole("menuitem", { name: /Zoom to selection/ })).toBeDisabled();
  });

  it("swaps canvases from the switcher only when the chat has more than one", () => {
    const other: CanvasBoard = { id: "onboarding", title: "Onboarding", lastChanged: [], artboards: [] };
    const { view, props } = panel();
    expect(screen.queryByRole("button", { name: /Switch canvas/ })).toBeNull();
    view.rerender(<CanvasPanel {...props} boards={[board, other]} />);
    fireEvent.click(screen.getByRole("button", { name: "Canvas: Billing themes. Switch canvas" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Onboarding/ }));
    expect(props.onSelectCanvas).toHaveBeenCalledWith("onboarding");
  });

  it("saves an artboard's resolved document at its real size", () => {
    const { props } = panel();
    fireEvent.click(screen.getByRole("button", { name: "Download “Ember”" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Save as PNG…" }));
    expect(props.onExport).toHaveBeenCalledWith({
      name: "Ember", html: "<main>base</main>\n<style>:root{--bg:#000}</style>", width: 1280, height: 800, format: "png",
    });
  });

  it("presents artboards one at a time in board order, stepping with the arrows and leaving with Esc", () => {
    panel();
    fireEvent.click(screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" }));
    fireEvent.click(screen.getByRole("button", { name: "Present" }));
    const dialog = screen.getByRole("dialog", { name: "Presenting Billing themes" });
    expect(within(dialog).getByText("2 / 3")).toBeInTheDocument();
    fireEvent.keyDown(dialog, { key: "ArrowRight" });
    expect(within(dialog).getByText("3 / 3")).toBeInTheDocument();
    fireEvent.keyDown(dialog, { key: "ArrowRight" });
    expect(within(dialog).getByText("1 / 3")).toBeInTheDocument();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    // Leaving lands on the artboard last shown.
    expect(screen.getByRole("button", { name: "Base, 1280 by 800" })).toHaveAttribute("aria-pressed", "true");
  });

  it("focuses the artboards a transcript card asked for, once per request", () => {
    const { view, props } = panel({ focus: { ids: ["phone"], nonce: 1 } });
    expect(screen.getByRole("button", { name: "Phone, 390 by auto" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "Base, 1280 by 800" }));
    view.rerender(<CanvasPanel {...props} focus={{ ids: ["phone"], nonce: 1 }} board={{ ...board }} />);
    expect(screen.getByRole("button", { name: "Base, 1280 by 800" })).toHaveAttribute("aria-pressed", "true");
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

  it("reports streamed revisions while keeping the accepted artboard visible", async () => {
    const { view, props } = panel({ activity: "drawing", progress: { type: "tool-call", toolName: "canvas", toolCallId: "revise", arguments: { canvas: board.id, artboards: [{ id: "base", html: "<main>Draft" }] } } });
    expect(screen.getByText("Writing Base")).toBeInTheDocument();
    expect(screen.getByText("11 characters received")).toBeInTheDocument();
    await waitFor(() => expect(api.stageVisual).toHaveBeenCalledTimes(3));
    expect(vi.mocked(api.stageVisual).mock.calls.some(([html]) => html.includes("Draft"))).toBe(false);
    view.rerender(<CanvasPanel {...props} progress={undefined} activity={undefined} />);
    expect(screen.queryByText("Writing Base")).toBeNull();
  });

  it("offers to fly to new artboards once the user has panned away", () => {
    const { view, props } = panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    act(() => {
      region.dispatchEvent(new WheelEvent("wheel", { deltaX: 40, deltaY: 40, bubbles: true, cancelable: true }));
    });
    const next: CanvasBoard = { ...board, artboards: [...board.artboards, { id: "pine", name: "Pine", group: "Desktop", width: 1280, height: 800, basedOn: "base" }] };
    view.rerender(<CanvasPanel {...props} board={next} />);
    expect(screen.getByRole("button", { name: "1 new or revised artboard" })).toBeInTheDocument();
  });

  it("keeps its viewer state when the panel page remounts", () => {
    const session = createCanvasSession();
    const { view } = panel({ session });
    fireEvent.click(screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" }));
    fireEvent.click(screen.getByRole("button", { name: "Hand" }));
    view.unmount();
    panel({ session });
    expect(screen.getByRole("button", { name: "Ember, 1280 by 800. Warm" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Hand" })).toHaveAttribute("aria-pressed", "true");
  });
});

describe("CanvasCard", () => {
  const touch = { toolCallId: "c1", canvas: "themes", title: "Billing themes", added: ["ember", "phone"], updated: ["base"], removed: [] };

  it("names what a turn drew and opens the canvas in the side panel at those artboards", () => {
    const onOpen = vi.fn();
    render(
      <CanvasScopeContext.Provider value={{ boards: new Map([[board.id, board]]), onOpen }}>
        <CanvasCard touch={touch} />
      </CanvasScopeContext.Provider>
    );
    expect(screen.getByText("Added 2 · Revised 1")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Canvas: Billing themes. Added 2 · Revised 1. Open in the side panel" }));
    expect(onOpen).toHaveBeenCalledWith("themes", ["base", "ember", "phone"]);
  });

  it("marks the canvas the panel is showing, and says when the agent is drawing on it", () => {
    render(
      <CanvasScopeContext.Provider value={{ boards: new Map([[board.id, board]]), shownId: "themes", activity: new Map([["themes", "drawing"]]), onOpen: vi.fn() }}>
        <CanvasCard touch={touch} />
      </CanvasScopeContext.Provider>
    );
    expect(screen.getByRole("button", { name: /Canvas: Billing themes/ })).toHaveAttribute("aria-current", "true");
    expect(screen.getByText("Drawing…")).toBeInTheDocument();
  });

  it("says when its canvas is gone", () => {
    render(<CanvasCard touch={{ toolCallId: "c1", canvas: "gone", title: "Old", added: ["a"], updated: [], removed: [] }} />);
    expect(screen.getByRole("button", { name: "Canvas: Old. This canvas is gone" })).toBeDisabled();
  });
});
