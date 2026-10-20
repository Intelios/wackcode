import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../../api";
import type { CanvasBoard } from "../../canvas-state";
import { CanvasPanel } from "./CanvasPanel";
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

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: () => void) {}
    observe() { this.callback(); }
    disconnect() {}
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
  vi.mocked(api.stageVisual).mockClear();
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function panel(overrides: Partial<Parameters<typeof CanvasPanel>[0]> = {}) {
  const props = {
    taskId: "t1", board, boards: [board], activity: undefined, expanded: false,
    onSelectCanvas: vi.fn(), onExpand: vi.fn(), onClose: vi.fn(), onMention: vi.fn(), ...overrides,
  };
  return { props, view: render(<CanvasPanel {...props} />) };
}

describe("CanvasPanel", () => {
  it("lists every artboard by name and size, and stages each as an artboard document", async () => {
    panel();
    expect(screen.getByText("Canvas · 3 artboards")).toBeInTheDocument();
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

  it("zooms from its controls and offers the switcher only with several canvases", () => {
    const other: CanvasBoard = { ...board, id: "other", title: "Onboarding" };
    const { props } = panel({ boards: [board, other] });
    fireEvent.click(screen.getByRole("button", { name: /^Canvas: Billing themes\. Switch canvas$/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /Onboarding/ }));
    expect(props.onSelectCanvas).toHaveBeenCalledWith("other");
    expect(screen.getByRole("button", { name: /^Zoom \d+%\. Zoom options$/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Expand canvas" }));
    expect(props.onExpand).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Close canvas panel" }));
    expect(props.onClose).toHaveBeenCalled();
  });

  it("says the agent is at work on it", () => {
    panel({ activity: "looking" });
    expect(screen.getByText("Checking artboards")).toHaveAttribute("role", "status");
  });

  it("offers to fly to new artboards once the user has panned away", () => {
    const { view, props } = panel();
    const region = screen.getByRole("region", { name: "Canvas: Billing themes" });
    act(() => {
      region.dispatchEvent(new WheelEvent("wheel", { deltaX: 40, deltaY: 40, bubbles: true, cancelable: true }));
    });
    const next: CanvasBoard = { ...board, artboards: [...board.artboards, { id: "pine", name: "Pine", group: "Desktop", width: 1280, height: 800, basedOn: "base" }] };
    view.rerender(<CanvasPanel {...props} board={next} boards={[next]} />);
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
    fireEvent.click(screen.getByRole("button", { name: "Show Base, Ember, Phone on the canvas" }));
    expect(onOpen).toHaveBeenCalledWith("themes", ["base", "ember", "phone"]);
    fireEvent.click(screen.getByRole("button", { name: "Open canvas" }));
    expect(onOpen).toHaveBeenCalledTimes(2);
  });

  it("says when its canvas is gone", () => {
    render(<CanvasCard touch={{ toolCallId: "c1", canvas: "gone", title: "Old", added: ["a"], updated: [], removed: [] }} />);
    expect(screen.getByText("This canvas is gone")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open canvas" })).toBeDisabled();
  });
});
