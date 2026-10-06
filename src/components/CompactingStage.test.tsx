import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CompactingStage, type CompactionReason } from "./CompactingStage";

let reducedMotion = false;
vi.mock("motion/react", async (importOriginal) => {
  const mod = await importOriginal<typeof import("motion/react")>();
  return { ...mod, useReducedMotion: () => reducedMotion };
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  reducedMotion = false;
});

describe("CompactingStage", () => {
  it("announces the status with the trigger reason and a ticking clock", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const reasons: [CompactionReason, string][] = [
      ["manual", "you asked"],
      ["threshold", "context was getting full"],
      ["overflow", "context overflowed"],
    ];
    for (const [reason, text] of reasons) {
      const view = render(<CompactingStage reason={reason} />);
      const status = screen.getByRole("status");
      expect(status).toHaveTextContent("Compacting context…");
      expect(status).toHaveTextContent(text);
      expect(status).toHaveTextContent("0:00");
      view.unmount();
      vi.setSystemTime(10_000);
    }
    render(<CompactingStage reason="manual" />);
    act(() => { vi.advanceTimersByTime(84_000); });
    expect(screen.getByRole("status")).toHaveTextContent("1:24");
  });

  it("escalates through calm, busy and dramatic acts", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    render(<CompactingStage reason="manual" />);
    expect(screen.getByRole("status")).toHaveAttribute("data-act", "calm");
    act(() => { vi.advanceTimersByTime(20_100); });
    expect(screen.getByRole("status")).toHaveAttribute("data-act", "busy");
    act(() => { vi.advanceTimersByTime(40_100); });
    expect(screen.getByRole("status")).toHaveAttribute("data-act", "dramatic");
  });

  it("runs the rAF sim while live", () => {
    vi.useFakeTimers();
    const raf = vi.spyOn(window, "requestAnimationFrame");
    render(<CompactingStage reason="manual" />);
    expect(raf).toHaveBeenCalled();
    raf.mockRestore();
  });

  it("renders a still frame under reduced motion: no rAF, clock still ticks", () => {
    reducedMotion = true;
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const raf = vi.spyOn(window, "requestAnimationFrame");
    render(<CompactingStage reason="manual" />);
    expect(screen.getByRole("status")).toHaveAttribute("data-still", "true");
    expect(raf).not.toHaveBeenCalled();
    act(() => { vi.advanceTimersByTime(5_000); });
    expect(screen.getByRole("status")).toHaveTextContent("0:05");
    raf.mockRestore();
  });

  it("calls onEnded after the finale once ending, and not before", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const onEnded = vi.fn();
    render(<CompactingStage reason="manual" onEnded={onEnded} />);
    act(() => { vi.advanceTimersByTime(700); });
    expect(onEnded).not.toHaveBeenCalled();
  });

  it("calls onEnded after the finale once ending", () => {
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const onEnded = vi.fn();
    const view = render(<CompactingStage reason="manual" onEnded={onEnded} />);
    view.rerender(<CompactingStage reason="manual" ending onEnded={onEnded} />);
    expect(screen.getByRole("status")).toHaveClass("ending");
    act(() => { vi.advanceTimersByTime(700); });
    expect(onEnded).toHaveBeenCalledTimes(1);
    // A second pass must not refire.
    act(() => { vi.advanceTimersByTime(2_000); });
    expect(onEnded).toHaveBeenCalledTimes(1);
  });

  it("under reduced motion the finale resolves immediately", () => {
    reducedMotion = true;
    vi.useFakeTimers();
    vi.setSystemTime(10_000);
    const onEnded = vi.fn();
    render(<CompactingStage reason="manual" ending onEnded={onEnded} />);
    act(() => { vi.advanceTimersByTime(10); });
    expect(onEnded).toHaveBeenCalledTimes(1);
  });
});
