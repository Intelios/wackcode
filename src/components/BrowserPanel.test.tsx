import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { BrowserState } from "../types";
import { BrowserPanel } from "./BrowserPanel";

vi.mock("../api", () => ({ api: { browserState: vi.fn(), browserPresent: vi.fn() } }));

const state: BrowserState = {
  taskId: "chat-1", exists: true, url: "https://www.google.com/", title: "Google",
  loading: false, canGoBack: false, canGoForward: false,
  agentActive: false, userControl: false, popup: false
};
const actions = { onState: vi.fn(), onExpand: vi.fn(), onReset: vi.fn(), onClose: vi.fn() };
let bounds: DOMRect;
const observers: { targets: Set<Element>; notify: () => void }[] = [];

function panel(hidden = false) {
  return (
    <aside className="side-panel">
      <BrowserPanel taskId={state.taskId} state={state} visible={!hidden} expanded={false} {...actions} />
    </aside>
  );
}

const presentations = () => vi.mocked(api.browserPresent).mock.calls.map(([input]) => input).filter((input) => input.visible);
const advance = async (milliseconds: number) => { await act(async () => { vi.advanceTimersByTime(milliseconds); }); };

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
  vi.clearAllMocks();
  bounds = new DOMRect(1000, 100, 430, 700);
  observers.length = 0;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => bounds);
  vi.mocked(api.browserState).mockResolvedValue(state);
  vi.mocked(api.browserPresent).mockResolvedValue(state);
  vi.stubGlobal("ResizeObserver", class {
    targets = new Set<Element>();
    constructor(readonly notify: () => void) { observers.push(this); }
    observe(target: Element) { this.targets.add(target); }
    disconnect() { this.targets.clear(); }
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("BrowserPanel positioning", () => {
  it("reopens at the final drawer position when the surface moves without resizing", async () => {
    const first = render(panel());
    await advance(160);
    expect(presentations()).toEqual([{ taskId: state.taskId, visible: true, x: 1000, y: 100, width: 430, height: 700 }]);
    first.unmount();
    vi.mocked(api.browserPresent).mockClear();

    bounds = new DOMRect(1400, 100, 430, 700);
    render(panel());
    // The drawer opens for 380 ms with a fixed-size surface. No surface resize notification
    // arrives, so a one-off measurement partway through the slide would leave a permanent gap.
    for (let frame = 1; frame <= 24; frame++) {
      bounds = new DOMRect(1400 - 400 * frame / 24, 100, 430, 700);
      await advance(16);
    }
    expect(presentations()).toEqual([]);
    await advance(160);
    expect(presentations()).toEqual([{ taskId: state.taskId, visible: true, x: 1000, y: 100, width: 430, height: 700 }]);
  });

  it("repositions when the drawer changes size while the surface keeps its size", async () => {
    render(panel());
    await advance(160);
    const drawer = screen.getByRole("complementary");
    bounds = new DOMRect(1200, 100, 430, 700);
    act(() => {
      for (const observer of observers) if (observer.targets.has(drawer)) observer.notify();
    });
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    await advance(160);
    expect(presentations().at(-1)).toEqual({ taskId: state.taskId, visible: true, x: 1200, y: 100, width: 430, height: 700 });
  });

  it("cancels pending placement when hidden or unmounted, and restores it when shown", async () => {
    const view = render(panel());
    await advance(80);
    view.rerender(panel(true));
    await advance(500);
    expect(presentations()).toEqual([]);

    view.rerender(panel());
    await advance(160);
    expect(presentations()).toHaveLength(1);
    view.rerender(panel(true));
    view.rerender(panel());
    await advance(80);
    view.unmount();
    await advance(500);
    expect(presentations()).toHaveLength(1);
    expect(api.browserPresent).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
  });
});
