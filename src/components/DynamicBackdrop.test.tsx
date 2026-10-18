import { createRef } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DynamicBackdrop, type DynamicBackdropHandle } from "./DynamicBackdrop";

const motion = vi.hoisted(() => ({ reduced: false }));
vi.mock("motion/react", () => ({ useReducedMotion: () => motion.reduced }));
const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
const disconnect = vi.fn();
const context = { setTransform: vi.fn(), scale: vi.fn(), fillText: vi.fn(), clearRect: vi.fn(), drawImage: vi.fn() };
const props = { style: "colourDrift", palette: "theme", intensity: 50, scene: "hero", phase: "idle", focused: true } as const;
function advance(now: number) {
  act(() => {
    const pending = [...frames.values()]; frames.clear();
    pending.forEach((frame) => frame(now));
  });
}
beforeEach(() => {
  motion.reduced = false; frames.clear(); nextFrame = 0; vi.clearAllMocks();
  vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) => { frames.set(++nextFrame, fn); return nextFrame; });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect = disconnect; });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => context as unknown as CanvasRenderingContext2D);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("dynamic animation lifecycle", () => {
  it("cancels frames on blur, hidden documents, zero intensity and disposal", () => {
    const view = render(<DynamicBackdrop {...props} />);
    expect(frames.size).toBe(1);
    advance(10); advance(50);
    const bloom = view.container.querySelector<HTMLElement>(".dynamic-bloom")!;
    const before = bloom.style.transform;
    view.rerender(<DynamicBackdrop {...props} focused={false} />);
    expect(frames.size).toBe(0);
    advance(10_000);
    expect(bloom.style.transform).toBe(before);
    view.rerender(<DynamicBackdrop {...props} />);
    advance(10_100); // Rebase the clock; no catch-up after a long pause.
    expect(bloom.style.transform).toBe(before);
    vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    act(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(frames.size).toBe(0);
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    act(() => document.dispatchEvent(new Event("visibilitychange")));
    expect(frames.size).toBe(1);
    view.rerender(<DynamicBackdrop {...props} intensity={0} />);
    expect(frames.size).toBe(0);
    view.unmount();
    expect(disconnect).toHaveBeenCalledOnce();
    expect(frames.size).toBe(0);
  });
  it("keeps reduced-motion compositions still across activity and pointer input", () => {
    motion.reduced = true;
    const ref = createRef<DynamicBackdropHandle>();
    const view = render(<DynamicBackdrop {...props} ref={ref} />);
    const bloom = view.container.querySelector<HTMLElement>(".dynamic-bloom")!;
    const initial = bloom.style.cssText;
    act(() => { ref.current!.pulse(); ref.current!.ripple(20, 20); });
    view.rerender(<DynamicBackdrop {...props} ref={ref} phase="thinking" />);
    expect(bloom.style.cssText).toBe(initial);
    expect(frames.size).toBe(0);
    view.rerender(<DynamicBackdrop {...props} ref={ref} palette="sunset" />);
    expect(frames.size).toBe(0);
    expect(view.container.querySelector(".dynamic-backdrop")).toHaveAttribute("aria-hidden", "true");
  });
  it("reuses the glyph atlas across activity and scene updates and redraws for a new palette", () => {
    const view = render(<DynamicBackdrop {...props} style="glyphFlow" />);
    const glyphs = context.fillText.mock.calls.length;
    expect(glyphs).toBeGreaterThan(0);
    advance(10); advance(50);
    view.rerender(<DynamicBackdrop {...props} style="glyphFlow" phase="thinking" scene="chat" />);
    advance(90);
    expect(context.fillText).toHaveBeenCalledTimes(glyphs);
    expect(context.drawImage).toHaveBeenCalled();
    view.rerender(<DynamicBackdrop {...props} style="glyphFlow" palette="aurora" />);
    expect(context.fillText).toHaveBeenCalledTimes(glyphs * 2);
    expect(frames.size).toBe(1);
    view.unmount();
    expect(frames.size).toBe(0);
  });
  it("falls back to the base background when a canvas context is unavailable", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const view = render(<DynamicBackdrop {...props} style="glyphFlow" />);
    expect(view.container.querySelector(".dynamic-backdrop")).not.toBeNull();
    expect(frames.size).toBe(0);
  });
});
