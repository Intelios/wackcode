import { StrictMode } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { BrowserContextAttachment, BrowserState } from "../types";
import { BROWSER_VIEWPORTS } from "../browser-context";
import { BrowserPanel } from "./BrowserPanel";
import { SidePanel } from "./SidePanel";

vi.mock("../api", () => ({ api: { browserState: vi.fn(), browserPresent: vi.fn(), browserRetirePresentation: vi.fn().mockResolvedValue(undefined), browserCapture: vi.fn(), browserSetViewport: vi.fn(), browserCancelCapture: vi.fn().mockResolvedValue(undefined) } }));

const state: BrowserState = {
  taskId: "chat-1", exists: true, url: "https://www.google.com/", title: "Google",
  loading: false, canGoBack: false, canGoForward: false,
  agentActive: false, userControl: false, popup: false
};
const actions = { onState: vi.fn(), onReset: vi.fn() };
let bounds: DOMRect;
const observers: { targets: Set<Element>; notify: () => void }[] = [];

function panel(hidden = false, shown: BrowserState = state) {
  return (
    <aside className="side-panel">
      <BrowserPanel taskId={state.taskId} state={shown} visible={!hidden} expanded={false} {...actions} />
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
    expect(presentations()).toEqual([{ taskId: state.taskId, presentationId: expect.any(String), visible: true, x: 1000, y: 100, width: 430, height: 700 }]);
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
    expect(presentations()).toEqual([{ taskId: state.taskId, presentationId: expect.any(String), visible: true, x: 1000, y: 100, width: 430, height: 700 }]);
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
    expect(presentations().at(-1)).toEqual({ taskId: state.taskId, presentationId: expect.any(String), visible: true, x: 1200, y: 100, width: 430, height: 700 });
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
  it("heads the panel with the page, and says whose hands are on it", () => {
    const view = render(panel());
    expect(screen.getByRole("heading", { name: "Google" })).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("button", { name: "Take control" })).toHaveAttribute("aria-pressed", "false");

    view.rerender(panel(false, { ...state, title: "", userControl: true }));
    expect(screen.getByRole("heading", { name: "www.google.com" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("You have control");
    expect(screen.getByRole("button", { name: "Resume agent control" })).toHaveAttribute("aria-pressed", "true");

    view.rerender(panel(false, { ...state, url: "", title: "", agentActive: true }));
    expect(screen.getByRole("heading", { name: "No page open" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Agent is using the page");
  });
  it("picks the viewport from a radio group of devices", async () => {
    vi.mocked(api.browserSetViewport).mockResolvedValue({ ...state, viewport: BROWSER_VIEWPORTS.phone });
    render(panel(false, { ...state, viewport: BROWSER_VIEWPORTS.tablet }));
    const group = screen.getByRole("radiogroup", { name: "Browser viewport" });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Tablet, 768 × 1024" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: "Tablet, 768 × 1024" }));
    expect(api.browserSetViewport).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(screen.getByRole("radio", { name: "Phone, 390 × 844" })); });
    expect(api.browserSetViewport).toHaveBeenCalledWith(state.taskId, "phone");
    expect(actions.onState).toHaveBeenCalledWith(expect.objectContaining({ viewport: BROWSER_VIEWPORTS.phone }));
  });
  it("retires the first StrictMode presentation without reusing it", async () => {
    render(<StrictMode>{panel()}</StrictMode>);
    await advance(160);
    const retired = vi.mocked(api.browserRetirePresentation).mock.calls.map(([, owner]) => owner);
    expect(retired).toHaveLength(1);
    expect(presentations()).toHaveLength(1);
    expect(retired).not.toContain(presentations()[0].presentationId);
  });
  it("cancels a hidden capture and ignores its late result before unmount", async () => {
    let complete!: (capture: BrowserContextAttachment) => void;
    vi.mocked(api.browserCapture).mockReturnValue(new Promise((resolve) => { complete = resolve; }));
    const onAttach = vi.fn();
    const view = render(<BrowserPanel taskId={state.taskId} state={state} visible expanded={false} {...actions} onAttach={onAttach} />);
    await advance(160);
    fireEvent.click(screen.getByRole("button", { name: "Mark up screenshot" }));
    const [task, capture, owner] = vi.mocked(api.browserCapture).mock.calls[0];
    expect(owner).toBe(presentations()[0].presentationId);
    view.rerender(<BrowserPanel taskId={state.taskId} state={state} visible={false} expanded={false} {...actions} onAttach={onAttach} />);
    expect(api.browserCancelCapture).toHaveBeenCalledWith(task, capture);
    await act(async () => complete({ id: capture, kind: "markup", capturedAt: 1, url: state.url, title: state.title, viewport: BROWSER_VIEWPORTS.desktop,
      image: { type: "image", mimeType: "image/png", data: "AAAA" }, targets: [] }));
    expect(screen.queryByRole("dialog", { name: "Mark up browser screenshot" })).not.toBeInTheDocument();
    expect(onAttach).not.toHaveBeenCalled();
  });
  it("releases a capture reservation as its drawer starts closing", () => {
    vi.mocked(api.browserCapture).mockReturnValue(new Promise(() => {}));
    const drawer = (open: boolean) => <SidePanel view={open ? { kind: "browser", taskId: state.taskId } : null} width={430} onWidthChange={vi.fn()} onResetWidth={vi.fn()} label="Browser"
      tabs={open ? [{ view: { kind: "browser", taskId: state.taskId }, label: "Browser", icon: "browser" }] : []} addable={[]} expanded={false}
      onSelectTab={vi.fn()} onCloseTab={vi.fn()} onAddTab={vi.fn()} onToggleExpand={vi.fn()} onHide={vi.fn()}>
      {() => <BrowserPanel taskId={state.taskId} state={state} visible expanded={false} {...actions} onAttach={vi.fn()} />}
    </SidePanel>;
    const view = render(drawer(true));
    fireEvent.click(screen.getByRole("button", { name: "Mark up screenshot" }));
    const [task, capture] = vi.mocked(api.browserCapture).mock.calls[0];
    view.rerender(drawer(false));
    // The exit animation is still running: cancellation must not wait for unmount.
    expect(api.browserCancelCapture).toHaveBeenCalledWith(task, capture);
    expect(api.browserRetirePresentation).toHaveBeenCalled();
  });
});
