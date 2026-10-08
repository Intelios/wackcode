import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useFollowScroll } from "./useFollowScroll";
import type { TranscriptViewState } from "../transcript-view";

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function TranscriptScroller({ chunk = 0 }: { chunk?: number }) {
  const { ref, onScroll, onWheel, detached, jumpToLatest, pauseFollowing, keepAnchor } = useFollowScroll();
  return <>
    <div ref={ref} onScroll={onScroll} onWheel={onWheel} data-testid="scroller"><span data-testid="anchor">Chunk {chunk}</span></div>
    <output>{detached ? "Reading history" : "Following"}</output>
    <button onClick={jumpToLatest}>Jump to latest</button>
    <button onClick={() => { pauseFollowing(); keepAnchor(ref.current!.firstElementChild as HTMLElement, 25); }}>Hold anchor</button>
  </>;
}

function setup(initialHeight = 1_000) {
  const view = render(<TranscriptScroller />);
  const el = screen.getByTestId("scroller");
  let height = initialHeight;
  let top = 0;
  // Model native clamping; jsdom otherwise lets scrollTop exceed the bottom.
  Object.defineProperties(el, {
    clientHeight: { configurable: true, get: () => 300 },
    scrollHeight: { configurable: true, get: () => height },
    scrollTop: {
      configurable: true,
      get: () => Math.max(0, Math.min(top, height - 300)),
      set: (value: number) => { top = Math.max(0, Math.min(value, height - 300)); }
    }
  });
  const scrollTo = vi.fn(({ top: next }: ScrollToOptions) => { el.scrollTop = next ?? 0; });
  Object.defineProperty(el, "scrollTo", { configurable: true, value: scrollTo });
  let chunk = 0;
  const stream = (growth = 0) => {
    height += growth;
    view.rerender(<TranscriptScroller chunk={++chunk} />);
  };
  const scroll = (next: number) => {
    el.scrollTop = next;
    fireEvent.scroll(el);
  };
  stream();
  return { ...view, el, stream, scroll, scrollTo, resize: (growth: number) => { height += growth; } };
}

describe("useFollowScroll", () => {
  it("waits for cold history, restores before paint and keeps the reading anchor through streaming", () => {
    const saved = { following: false, top: 675, anchors: [{ key: "reading", offset: 25 }] };
    const memory: TranscriptViewState = { position: saved };
    function Remembered({ ready = false, y = 400 }: { ready?: boolean; y?: number }) {
      const scroll = useFollowScroll(memory, ready);
      return <div ref={scroll.ref} onScroll={scroll.onScroll} data-testid="memory-scroller">
        {ready && <div data-transcript-anchor="reading" data-y={y}>History</div>}
      </div>;
    }
    const view = render(<Remembered />);
    const el = screen.getByTestId("memory-scroller");
    Object.defineProperties(el, { clientHeight: { value: 300 }, scrollHeight: { value: 2_000 } });
    el.getBoundingClientRect = () => ({ top: 50 } as DOMRect);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return { top: 50 + Number(this.dataset.y ?? 0) - el.scrollTop } as DOMRect;
    });
    view.rerender(<Remembered />);
    expect(memory.position).toBe(saved);
    expect(el.scrollTop).toBe(0);
    view.rerender(<Remembered ready />);
    expect(el.scrollTop).toBe(375);
    view.rerender(<Remembered ready y={350} />);
    expect(el.scrollTop).toBe(325);
    expect(memory.position).toMatchObject({ following: false, top: 325 });
  });

  it("uses the nearest surviving anchor after a branch removes the held row, then clamps an empty branch", () => {
    const memory: TranscriptViewState = { position: {
      following: false, top: 490, anchors: [{ key: "removed", offset: 10 }, { key: "next", offset: 210 }, { key: "previous", offset: -290 }]
    } };
    function Branch({ ready = false, rows = [["removed", 500], ["next", 700], ["previous", 200]] as [string, number][] }: { ready?: boolean; rows?: [string, number][] }) {
      const scroll = useFollowScroll(memory, ready);
      return <div ref={scroll.ref} data-testid="branch-scroller">{ready && rows.map(([key, y]) => <div key={key} data-transcript-anchor={key} data-y={y}>{key}</div>)}</div>;
    }
    const view = render(<Branch />);
    const el = screen.getByTestId("branch-scroller");
    let height = 2_000;
    Object.defineProperties(el, { clientHeight: { value: 300 }, scrollHeight: { get: () => height } });
    el.getBoundingClientRect = () => ({ top: 50 } as DOMRect);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      return { top: 50 + Number(this.dataset.y ?? 0) - el.scrollTop } as DOMRect;
    });
    view.rerender(<Branch ready />);
    expect(el.scrollTop).toBe(490);
    view.rerender(<Branch ready rows={[["next", 400], ["previous", 200]]} />);
    expect(el.scrollTop).toBe(190);
    height = 400;
    view.rerender(<Branch ready rows={[]} />);
    expect(el.scrollTop).toBe(100);
  });

  it("follows growing output while the user stays at the bottom", () => {
    const { el, stream } = setup();
    expect(el.scrollTop).toBe(700);
    stream(40);
    expect(el.scrollTop).toBe(740);
    fireEvent.scroll(el);
    stream(120);
    expect(el.scrollTop).toBe(860);
    expect(screen.getByText("Following")).toBeInTheDocument();
  });

  it("releases on the first upward wheel input, before a scroll or streaming render", () => {
    const { el, stream, scroll } = setup();
    fireEvent.wheel(el, { deltaY: -2 });
    expect(screen.getByText("Reading history")).toBeInTheDocument();
    // A queued scroll event from the previous auto-pin must not reattach us.
    fireEvent.scroll(el);
    stream(20);
    expect(el.scrollTop).toBe(700);
    scroll(698);
    expect(screen.getByText("Reading history")).toBeInTheDocument();
    for (let i = 0; i < 5; i++) stream(10);
    expect(el.scrollTop).toBe(698);
  });

  it("detaches for a small upward scroll even without a wheel event", () => {
    const { el, stream, scroll } = setup();
    scroll(692);
    expect(screen.getByText("Reading history")).toBeInTheDocument();
    stream(30);
    expect(el.scrollTop).toBe(692);
  });

  it("only resumes following when scrolling down all the way to the bottom", () => {
    const { el, stream, scroll } = setup();
    scroll(400);
    scroll(450);
    stream(20);
    expect(el.scrollTop).toBe(450);
    expect(screen.getByText("Reading history")).toBeInTheDocument();
    scroll(710);
    expect(screen.getByText("Reading history")).toBeInTheDocument();
    expect(el.scrollTop).toBe(710);
    scroll(720);
    expect(screen.getByText("Following")).toBeInTheDocument();
    stream(20);
    expect(el.scrollTop).toBe(740);
  });

  it("lets Jump to latest re-pin streaming output", () => {
    const { el, stream, scroll, scrollTo } = setup();
    scroll(400);
    stream(80);
    fireEvent.click(screen.getByRole("button", { name: "Jump to latest" }));
    expect(scrollTo).toHaveBeenCalledWith({ top: 1_080, behavior: "smooth" });
    fireEvent.scroll(el);
    stream(40);
    expect(el.scrollTop).toBe(820);
    expect(screen.getByText("Following")).toBeInTheDocument();
  });

  it("does not mistake bottom clamping after content shrinks for scrolling up", () => {
    const { el, stream } = setup();
    stream(-200);
    fireEvent.scroll(el);
    stream(40);
    expect(el.scrollTop).toBe(540);
    expect(screen.getByText("Following")).toBeInTheDocument();
  });

  it("keeps following when an upward wheel cannot scroll a short transcript", () => {
    const { el, stream } = setup(200);
    fireEvent.wheel(el, { deltaY: -4 });
    expect(screen.getByText("Following")).toBeInTheDocument();
    stream(300);
    expect(el.scrollTop).toBe(200);
  });

  it("ignores upward wheels consumed by nested output, but pauses when they reach the transcript", () => {
    const { el, stream } = setup();
    const output = document.createElement("pre");
    output.style.overflowY = "auto";
    output.scrollTop = 20;
    el.append(output);
    fireEvent.wheel(output, { deltaY: -4 });
    stream(20);
    expect(el.scrollTop).toBe(720);
    expect(screen.getByText("Following")).toBeInTheDocument();
    output.scrollTop = 0;
    fireEvent.wheel(output, { deltaY: -4 });
    stream(20);
    expect(el.scrollTop).toBe(720);
    expect(screen.getByText("Reading history")).toBeInTheDocument();
  });

  it("keeps following size changes between React commits and disconnects its observer", () => {
    const callbacks: Array<() => void> = [];
    const disconnect = vi.fn();
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { callbacks.push(callback); }
      observe() {}
      disconnect = disconnect;
    });
    const { el, unmount, resize } = setup();
    resize(100);
    act(() => callbacks[callbacks.length - 1]());
    expect(el.scrollTop).toBe(800);
    fireEvent.scroll(el);
    expect(screen.getByText("Following")).toBeInTheDocument();
    unmount();
    expect(disconnect).toHaveBeenCalled();
  });

  it("holds a reading anchor through resize frames, but releases it on real transcript wheel intent", () => {
    const callbacks: Array<() => void> = [];
    vi.stubGlobal("ResizeObserver", class {
      constructor(callback: () => void) { callbacks.push(callback); }
      observe() {}
      disconnect() {}
    });
    const { el, scroll } = setup();
    scroll(400);
    const anchor = screen.getByTestId("anchor");
    let y = 425;
    vi.spyOn(anchor, "getBoundingClientRect").mockImplementation(() => ({ top: y - el.scrollTop } as DOMRect));
    fireEvent.click(screen.getByRole("button", { name: "Hold anchor" }));
    const frame = () => act(() => callbacks[callbacks.length - 1]());
    y += 30;
    frame();
    expect(el.scrollTop).toBe(430);
    fireEvent.scroll(el);
    expect(screen.getByText("Reading history")).toBeInTheDocument();

    const output = document.createElement("pre");
    output.style.overflowY = "auto";
    output.scrollTop = 20;
    el.append(output);
    fireEvent.wheel(output, { deltaY: -4 });
    y += 30;
    frame();
    expect(el.scrollTop).toBe(460);
    Object.defineProperties(output, { clientHeight: { value: 100 }, scrollHeight: { value: 200 } });
    fireEvent.wheel(output, { deltaY: 4 });
    y += 30;
    frame();
    expect(el.scrollTop).toBe(490);
    fireEvent.wheel(el, { deltaY: 4 });
    y += 30;
    frame();
    expect(el.scrollTop).toBe(490);
  });

  it("ignores horizontal wheels and zoom gestures", () => {
    const { el, stream } = setup();
    fireEvent.wheel(el, { deltaX: -10 });
    fireEvent.wheel(el, { deltaY: -10, ctrlKey: true });
    act(() => stream(30));
    expect(el.scrollTop).toBe(730);
    expect(screen.getByText("Following")).toBeInTheDocument();
  });
});
