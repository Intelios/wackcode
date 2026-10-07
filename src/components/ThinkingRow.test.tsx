import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ThinkingPreviewEnabled, ThinkingRow, ThinkingTimerPrecision } from "./ThinkingRow";
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

// Reduced motion makes useSmoothText reveal deltas at once, so each rerender updates synchronously.
function stubReducedMotion() {
  vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
}

describe("ThinkingRow live timer", () => {
  it("ticks even without new reasoning deltas and keeps its start across rerenders and remounts", () => {
    // Previews off: the stream's enter/exit animation holds its own timers, which would
    // muddy the count this test asserts. The timer beside a stream is covered below.
    const row = (text: string) => (
      <ThinkingPreviewEnabled.Provider value={false}>
        <ThinkingRow text={text} live startedAt={10_000} />
      </ThinkingPreviewEnabled.Provider>
    );
    const view = render(row(""));
    expect(screen.getByRole("button", { name: "Thinking… <1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3_000));
    expect(screen.getByRole("button", { name: "Thinking… 3s" })).toBeInTheDocument();

    view.rerender(row("Still reasoning"));
    act(() => vi.advanceTimersByTime(2_000));
    expect(screen.getByRole("button", { name: "Thinking… 5s" })).toBeInTheDocument();

    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    act(() => vi.advanceTimersByTime(4_000));
    render(row("Still reasoning"));
    expect(screen.getByRole("button", { name: "Thinking… 9s" })).toBeInTheDocument();
  });

  it("counts in tenths when the timer is set to them (Settings → Appearance)", () => {
    const tenths = (
      <ThinkingPreviewEnabled.Provider value={false}>
        <ThinkingTimerPrecision.Provider value="tenth">
          <ThinkingRow text="" live startedAt={10_000} />
        </ThinkingTimerPrecision.Provider>
      </ThinkingPreviewEnabled.Provider>
    );
    const view = render(tenths);
    expect(screen.getByRole("button", { name: "Thinking… <0.1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(150));
    expect(screen.getByRole("button", { name: "Thinking… 0.1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.getByRole("button", { name: "Thinking… 1.1s" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(64_050));
    expect(screen.getByRole("button", { name: "Thinking… 65.2s" })).toBeInTheDocument();
    // The worker's final duration honours the same choice.
    view.rerender(
      <ThinkingPreviewEnabled.Provider value={false}>
        <ThinkingTimerPrecision.Provider value="tenth">
          <ThinkingRow text="" durationMs={4_200} />
        </ThinkingTimerPrecision.Provider>
      </ThinkingPreviewEnabled.Provider>
    );
    expect(screen.getByRole("button", { name: "Thought for 4.2s" })).toBeInTheDocument();
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

  it("keeps the timer beside the collapsed stream, outside the accessible name", () => {
    render(<ThinkingRow text="Checking both approaches. Now I compare" live startedAt={8_000} />);
    const row = screen.getByRole("button", { name: "Thinking… 2s" });
    expect(within(row).getByText(/Checking both approaches\./)).toBeInTheDocument();
    expect(row.querySelector(".thinking-stream")).toHaveAttribute("aria-hidden", "true");
  });

  it("flows the reasoning tail past headings instead of latching them", () => {
    // Reduced motion reveals each rerender's new text at once instead of via rAF frames.
    stubReducedMotion();
    const view = render(<ThinkingRow text="**Exploring the code**" live startedAt={8_000} />);
    expect(view.container.querySelector(".thinking-stream-text")).toHaveTextContent("Exploring the code");
    view.rerender(<ThinkingRow text={"**Exploring the code**\n\nI read App.tsx. Now I check"} live startedAt={8_000} />);
    expect(view.container.querySelector(".thinking-stream-text")).toHaveTextContent("Exploring the code · I read App.tsx. Now I check");
  });

  it("hides the stream when opened, finished or switched off, and drops the streaming class", async () => {
    // Real timers: the live elapsed clock starts ticking, harmless here, and the exit
    // animation can actually finish (the file's fake timers stall it).
    vi.useRealTimers();
    const view = render(<ThinkingRow text="Checking both approaches." live startedAt={8_000} />);
    expect(view.container.querySelector(".thinking-row")).toHaveClass("streaming");
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    // Opening hides the stream at once; its exit animation then removes the element.
    await waitFor(() => expect(view.container.querySelector(".thinking-stream")).toBeNull());
    expect(view.container.querySelector(".thinking-row")).not.toHaveClass("streaming");
    view.unmount();

    const done = render(<ThinkingRow text="Checking both approaches." durationMs={4_200} />);
    expect(done.container.querySelector(".thinking-stream")).toBeNull();
    done.unmount();

    const off = render(
      <ThinkingPreviewEnabled.Provider value={false}>
        <ThinkingRow text="Checking both approaches." live startedAt={8_000} />
      </ThinkingPreviewEnabled.Provider>
    );
    expect(off.container.querySelector(".thinking-stream")).toBeNull();
    expect(off.container.querySelector(".thinking-row")).not.toHaveClass("streaming");
  });

  it("unfurls open, then holds the body through its closing animation", async () => {
    // Real timers so framer's exit can actually finish; motion allowed so the reveal wrapper runs
    // (elsewhere the still path mounts and unmounts at once, as every other test here asserts).
    vi.useRealTimers();
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    const view = render(<ThinkingRow text="Weighing both approaches." durationMs={4_200} />);
    const head = screen.getByRole("button", { name: "Thought for 4s" });
    fireEvent.click(head);
    const region = screen.getByRole("region", { name: "Reasoning" });
    expect(view.container.querySelector(".thinking-reveal .thinking-body")).toBe(region);

    fireEvent.click(head);
    // The body lingers while its exit animation runs, then leaves.
    expect(screen.getByRole("region", { name: "Reasoning" })).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("region", { name: "Reasoning" })).toBeNull());
    expect(head).toHaveAttribute("aria-expanded", "false");
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

  it("animates its duck mark only while live", () => {
    const view = render(<ThinkingRow text="" live startedAt={8_000} />);
    const duck = () => document.querySelector(".ponder-duck");
    expect(duck()).toHaveClass("live");
    view.rerender(<ThinkingRow text="" durationMs={4_200} />);
    expect(duck()).not.toHaveClass("live");
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

describe("ThinkingRow ink and rail", () => {
  // Motion positively allowed: matchMedia exists and reports no reduced preference, so the
  // body takes the fresh-ink path. Fake timers are fine here — useSmoothText shows a same-length
  // rerender at once, and the body mounts with its full first text.
  function stubMotion() {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  }

  it("frames a live body with a living rail and fresh ink, then settles when the block finishes", () => {
    stubMotion();
    const view = render(<ThinkingRow text="Weighing both approaches" live startedAt={8_000} />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    expect(view.container.querySelector(".thinking-frame")).toHaveClass("live");
    const region = screen.getByRole("region", { name: "Reasoning" });
    expect(region.querySelectorAll(".ink-fresh").length).toBeGreaterThan(0);
    expect(region.querySelector(".ink-caret")).not.toBeNull();

    view.rerender(<ThinkingRow text="Weighing both approaches" durationMs={4_200} />);
    const frame = view.container.querySelector(".thinking-frame");
    expect(frame).not.toHaveClass("live");
    expect(frame).toHaveClass("settling");
    // Finished means settled ink: the word spans and caret drop out of the tree at once.
    expect(screen.getByRole("region", { name: "Reasoning" }).querySelectorAll(".ink-fresh, .ink-caret")).toHaveLength(0);
    act(() => vi.advanceTimersByTime(900));
    expect(view.container.querySelector(".thinking-frame")).not.toHaveClass("settling");
    view.unmount();
    // No bare getTimerCount here: with motion allowed, motion/react schedules its own
    // animation timers under fake timers, which the settle assertions above already cover.
  });

  it("never settles a row that mounts already finished", () => {
    stubMotion();
    const view = render(<ThinkingRow text="Earlier reasoning" durationMs={4_200} />);
    fireEvent.click(screen.getByRole("button", { name: "Thought for 4s" }));
    expect(view.container.querySelector(".thinking-frame")).not.toHaveClass("settling");
    act(() => vi.advanceTimersByTime(5_000));
    expect(view.container.querySelector(".thinking-frame")).not.toHaveClass("settling");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps the body plain where motion is not positively allowed", () => {
    // Reduced motion: the tinted rail and wet colour still mark the live row, but words render
    // as plain text with no caret.
    stubReducedMotion();
    const view = render(<ThinkingRow text="Plain reasoning here" live startedAt={8_000} />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking…/ }));
    expect(view.container.querySelector(".thinking-frame")).toHaveClass("live");
    const region = screen.getByRole("region", { name: "Reasoning" });
    expect(region.querySelectorAll(".ink-fresh, .ink-caret")).toHaveLength(0);
  });
});
