import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useFollowScroll } from "./useFollowScroll";

afterEach(cleanup);

function TranscriptScroller({ chunk = 0 }: { chunk?: number }) {
  const { ref, onScroll, onWheel, detached, jumpToLatest } = useFollowScroll();
  return <>
    <div ref={ref} onScroll={onScroll} onWheel={onWheel} data-testid="scroller">Chunk {chunk}</div>
    <output>{detached ? "Reading history" : "Following"}</output>
    <button onClick={jumpToLatest}>Jump to latest</button>
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
  return { ...view, el, stream, scroll, scrollTo };
}

describe("useFollowScroll", () => {
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

  it("ignores horizontal wheels and zoom gestures", () => {
    const { el, stream } = setup();
    fireEvent.wheel(el, { deltaX: -10 });
    fireEvent.wheel(el, { deltaY: -10, ctrlKey: true });
    act(() => stream(30));
    expect(el.scrollTop).toBe(730);
    expect(screen.getByText("Following")).toBeInTheDocument();
  });
});
