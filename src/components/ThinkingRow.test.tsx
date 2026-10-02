import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ThinkingPreviewEnabled, ThinkingRow } from "./ThinkingRow";
import { ExploreGroup } from "./ExploreGroup";

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(10_000);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("ThinkingRow live timer", () => {
  it("ticks even without new reasoning deltas and keeps its start across rerenders and remounts", () => {
    const view = render(<ThinkingRow text="" live startedAt={10_000} />);
    expect(screen.getByRole("button", { name: "Thinking… <1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.getByRole("button", { name: "Thinking… 3s" })).toBeInTheDocument();

    view.rerender(<ThinkingRow text="Still reasoning" live startedAt={10_000} />);
    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByRole("button", { name: "Thinking… 5s" })).toBeInTheDocument();

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    act(() => vi.advanceTimersByTime(4_000));
    render(<ThinkingRow text="Still reasoning" live startedAt={10_000} />);
    expect(screen.getByRole("button", { name: "Thinking… 9s" })).toBeInTheDocument();
  });

  it("stays visible while expanded and when thinking previews are disabled", () => {
    render(<ThinkingPreviewEnabled.Provider value={false}>
      <ThinkingRow text="Checking both approaches." live startedAt={8_000} />
    </ThinkingPreviewEnabled.Provider>);
    const row = screen.getByRole("button", { name: "Thinking… 2s" });
    fireEvent.click(row);
    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Checking both approaches.")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByRole("button", { name: "Thinking… 3s" })).toHaveAttribute("aria-expanded", "true");
  });

  it("keeps the timer beside the collapsed preview", () => {
    render(<ThinkingRow text="Checking both approaches. Now I compare" live startedAt={8_000} />);
    expect(screen.getByRole("button", { name: /Thinking… 2s.*Checking both approaches\./ })).toBeInTheDocument();
  });

  it("resets for a new block and formats minutes and hours", () => {
    const view = render(<ThinkingRow text="" live startedAt={8_000} />);
    view.rerender(<ThinkingRow text="" live startedAt={10_000} />);
    expect(screen.getByRole("button", { name: "Thinking… <1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(65_000));
    expect(screen.getByRole("button", { name: "Thinking… 1m 5s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_600_000));
    expect(screen.getByRole("button", { name: "Thinking… 1h 1m 5s" })).toBeInTheDocument();
  });

  it("replaces the timer with the worker's final duration and clears the interval", () => {
    const view = render(<ThinkingRow text="" live startedAt={8_000} />);
    expect(vi.getTimerCount()).toBe(1);
    view.rerender(<ThinkingRow text="" durationMs={4_200} />);
    expect(screen.getByRole("button", { name: "Thought for 4s" })).toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
    act(() => vi.advanceTimersByTime(5_000));
    expect(screen.getByRole("button", { name: "Thought for 4s" })).toBeInTheDocument();
  });

  it("uses the block's original start when an exploration group is opened later", () => {
    render(<ExploreGroup group={{ key: "read-a", items: [
      { key: "a", block: { type: "tool-call", toolName: "read", arguments: { path: "a.ts" } }, live: false, streaming: false },
      { key: "thinking", block: { type: "thinking", text: "", startedAt: 8_000 }, live: true, streaming: true },
      { key: "b", block: { type: "tool-call", toolName: "read", arguments: { path: "b.ts" } }, live: false, streaming: false }
    ] }} results={new Map()} running />);
    act(() => vi.advanceTimersByTime(3_000));
    fireEvent.click(screen.getByRole("button", { name: /Explored.*2 files/ }));
    expect(screen.getByRole("button", { name: "Thinking… 5s" })).toBeInTheDocument();
  });

  it("never runs a timer for saved or unclocked reasoning, and clamps a future start", () => {
    const view = render(<ThinkingRow text="" />);
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
    view.rerender(<ThinkingRow text="" live />);
    expect(screen.getByRole("button", { name: "Thinking…" })).toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
    view.rerender(<ThinkingRow text="" live startedAt={11_000} />);
    expect(screen.getByRole("button", { name: "Thinking… <1s" })).toBeInTheDocument();
    view.rerender(<ThinkingRow text="" startedAt={11_000} />);
    expect(screen.getByRole("button", { name: "Reasoning" })).toBeInTheDocument();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("ThinkingRow body scroll", () => {
  const VIEWPORT = 120;

  /** jsdom does no layout: model a scroll region whose content is `height` tall. */
  function rigScrollRegion(el: HTMLElement, height: number) {
    let contentHeight = height;
    let top = 0;
    Object.defineProperties(el, {
      clientHeight: { configurable: true, get: () => VIEWPORT },
      scrollHeight: { configurable: true, get: () => contentHeight },
      scrollTop: {
        configurable: true,
        get: () => Math.max(0, Math.min(top, contentHeight - VIEWPORT)),
        set: (value: number) => { top = Math.max(0, Math.min(value, contentHeight - VIEWPORT)); }
      }
    });
    return {
      grow: (by: number) => { contentHeight += by; },
      scroll: (to: number) => { el.scrollTop = to; fireEvent.scroll(el); }
    };
  }

  // Reduced motion makes useSmoothText reveal deltas at once, so each rerender pins synchronously.
  function stubReducedMotion() {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  }

  const reasoning = (length: number) => `Reasoning. ${"and more ".repeat(length)}`;

  it("opens at the newest reasoning, live or finished, and follows the stream", () => {
    stubReducedMotion();
    const view = render(<ThinkingRow text={reasoning(40)} live startedAt={8_000} />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    const region = screen.getByRole("region", { name: "Reasoning" });
    const rig = rigScrollRegion(region, 500);
    view.rerender(<ThinkingRow text={reasoning(40)} live startedAt={8_000} />);
    expect(region.scrollTop).toBe(380);

    rig.grow(200);
    view.rerender(<ThinkingRow text={reasoning(56)} live startedAt={8_000} />);
    expect(region.scrollTop).toBe(580);

    view.unmount();
    const done = render(<ThinkingRow text={reasoning(40)} durationMs={4_200} />);
    fireEvent.click(screen.getByRole("button", { name: "Thought for 4s" }));
    const finished = screen.getByRole("region", { name: "Reasoning" });
    rigScrollRegion(finished, 500);
    done.rerender(<ThinkingRow text={reasoning(40)} durationMs={4_200} />);
    expect(finished.scrollTop).toBe(380);
  });

  it("holds the reader's place after they scroll up, until they return to the bottom", () => {
    stubReducedMotion();
    const view = render(<ThinkingRow text={reasoning(40)} live startedAt={8_000} />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    const region = screen.getByRole("region", { name: "Reasoning" });
    const rig = rigScrollRegion(region, 500);
    view.rerender(<ThinkingRow text={reasoning(40)} live startedAt={8_000} />);
    expect(region.scrollTop).toBe(380);

    rig.scroll(300);
    rig.grow(100);
    view.rerender(<ThinkingRow text={reasoning(48)} live startedAt={8_000} />);
    expect(region.scrollTop).toBe(300);

    rig.scroll(480);
    rig.grow(60);
    view.rerender(<ThinkingRow text={reasoning(52)} live startedAt={8_000} />);
    expect(region.scrollTop).toBe(540);
  });
});
