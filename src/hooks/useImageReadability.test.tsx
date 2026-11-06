import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CENTRED } from "../backdrop-crop";
import { analyseImage, backdropSamples, type ImageBounds } from "../image-analysis";
import { useImageReadability } from "./useImageReadability";
import type { BackgroundImageMotion } from "../types";

vi.mock("../image-analysis", async (original) => ({
  ...await original<typeof import("../image-analysis")>(),
  analyseImage: vi.fn(),
  backdropSamples: vi.fn((await original<typeof import("../image-analysis")>()).backdropSamples)
}));

class TestImage {
  static instances: TestImage[] = [];
  naturalWidth = 64;
  naturalHeight = 64;
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  crossOrigin = "";
  private url = "";
  corsAtLoad = "";
  constructor() { TestImage.instances.push(this); }
  set src(value: string) { this.url = value; if (value) this.corsAtLoad = this.crossOrigin; }
  get src() { return this.url; }
}

function whiteBounds(): ImageBounds {
  return { width: 64, height: 64, columns: 2, rows: 2, cells: new Uint8ClampedArray(32).fill(255) };
}

const inspectMotion = vi.fn<(name: string) => Promise<BackgroundImageMotion>>();

function Harness({ url = "asset://one.png", enabled = true, visual = false, crop = CENTRED }: { url?: string; enabled?: boolean; visual?: boolean; crop?: typeof CENTRED }) {
  const root = useRef<HTMLDivElement>(null);
  const { result, ready } = useImageReadability({ root, enabled, imageUrl: url, imageName: url.split("/").pop(), inspectMotion,
    crop, dim: 0, blur: 0, conversationKey: "code:one", area: "code", visual, warning: false });
  return <div ref={root} className="app-shell" data-testid="shell" data-ready={String(ready)} data-polarity={result?.polarity} data-extra-dim={result?.extraDim}>
    <div className="workspace"><div className="chat-view"><div className="conversation-scroll" /></div></div>
  </div>;
}

beforeEach(() => {
  TestImage.instances = [];
  vi.stubGlobal("Image", TestImage);
  inspectMotion.mockReset().mockResolvedValue("static");
  vi.mocked(analyseImage).mockReset().mockResolvedValue(whiteBounds());
  vi.mocked(backdropSamples).mockClear();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, toJSON: () => ({}) });
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("image readability lifecycle", () => {
  it("waits for a safe result, loads with CORS, then uses dark text for a bright image", async () => {
    render(<Harness />);
    expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "false");
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "light");
    expect(TestImage.instances[0].corsAtLoad).toBe("anonymous");
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "true"));
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "dark");
  });

  it("never inspects or samples images when protection is off", () => {
    render(<Harness enabled={false} />);
    expect(inspectMotion).not.toHaveBeenCalled();
    expect(analyseImage).not.toHaveBeenCalled();
    expect(TestImage.instances).toHaveLength(0);
    expect(screen.getByTestId("shell")).not.toHaveAttribute("data-polarity");
  });

  it.each(["animated", "unknown"] as const)("protects %s images conservatively without sampling a single frame", async (motion) => {
    inspectMotion.mockResolvedValue(motion);
    render(<Harness />);
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "true"));
    expect(analyseImage).not.toHaveBeenCalled();
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "light");
    expect(Number(screen.getByTestId("shell").dataset.extraDim)).toBeGreaterThan(0);
  });

  it("uses the safe fallback when pixel access fails", async () => {
    vi.mocked(analyseImage).mockRejectedValue(new Error("Tainted canvas"));
    render(<Harness />);
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "true"));
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "light");
    expect(Number(screen.getByTestId("shell").dataset.extraDim)).toBeGreaterThan(0);
  });

  it("discards a late analysis after switching backgrounds", async () => {
    let finishOld!: (bounds: ImageBounds) => void;
    vi.mocked(analyseImage).mockImplementationOnce(() => new Promise((resolve) => { finishOld = resolve; }));
    const { rerender } = render(<Harness />);
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(analyseImage).toHaveBeenCalledTimes(1));
    rerender(<Harness url="asset://two.png" />);
    expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "false");
    act(() => TestImage.instances[1].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "dark"));
    const black = whiteBounds(); black.cells.fill(0); for (let at = 0; at < black.cells.length; at += 8) black.cells[at + 6] = black.cells[at + 7] = 255;
    await act(async () => finishOld(black));
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "dark");
  });

  it("reuses pixel bounds for crop and viewport changes, and ignores scrolling", async () => {
    const { rerender } = render(<Harness />);
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-ready", "true"));
    const samples = vi.mocked(backdropSamples).mock.calls.length;
    fireEvent.scroll(document.querySelector(".conversation-scroll")!);
    rerender(<Harness />);
    expect(backdropSamples).toHaveBeenCalledTimes(samples);
    rerender(<Harness crop={{ zoom: 200, x: 100, y: 900 }} />);
    expect(backdropSamples).toHaveBeenCalledTimes(samples + 1);
    vi.mocked(HTMLElement.prototype.getBoundingClientRect).mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 700, right: 1000, bottom: 700, toJSON: () => ({}) });
    fireEvent(window, new Event("resize"));
    await waitFor(() => expect(backdropSamples).toHaveBeenCalledTimes(samples + 2));
    expect(analyseImage).toHaveBeenCalledTimes(1);
    expect(inspectMotion).toHaveBeenCalledTimes(1);
  });

  it("protects a visual reply while preserving the sampled image and saved dim preference", async () => {
    const { rerender } = render(<Harness />);
    act(() => TestImage.instances[0].onload?.());
    await waitFor(() => expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "dark"));
    rerender(<Harness visual />);
    expect(screen.getByTestId("shell")).toHaveAttribute("data-polarity", "light");
    expect(Number(screen.getByTestId("shell").dataset.extraDim)).toBeGreaterThan(0);
    expect(analyseImage).toHaveBeenCalledTimes(1);
  });
});
