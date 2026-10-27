import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NormalizedBlock } from "../../types";
import { CanvasProgress } from "./CanvasProgress";
import { CanvasScopeContext } from "./CanvasCard";

afterEach(() => { cleanup(); vi.useRealTimers(); });
const call: NormalizedBlock = { type: "tool-call", toolName: "canvas", toolCallId: "draw", arguments: { title: "Billing", artboards: [{ name: "Desktop", html: "<main>" }] } };

describe("CanvasProgress", () => {
  it("shows actual streamed work, identifies quiet streams, and resumes when content arrives", () => {
    vi.useFakeTimers();
    const { rerender, container } = render(<CanvasProgress call={call} />);
    expect(screen.getByText("Writing Desktop")).toBeInTheDocument();
    expect(screen.getByText("6 characters received")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(15_000));
    expect(screen.getByText("No new canvas content")).toHaveTextContent("No new canvas content for 15s");
    expect(container.querySelector(".quill-mark.live")).toBeNull();
    // Unrelated snapshots must not reset a stalled stream's clock.
    rerender(<CanvasProgress call={{ ...call }} />);
    expect(screen.getByText("No new canvas content")).toHaveTextContent("No new canvas content for 15s");
    rerender(<CanvasProgress call={{ ...call, arguments: { title: "Billing", artboards: [{ name: "Desktop", html: "<main>Hi</main>" }, { name: "Phone" }] } }} />);
    expect(screen.queryByText(/No new canvas content/)).toBeNull();
    expect(screen.getByText("Writing Phone")).toBeInTheDocument();
    expect(screen.getByText("15 characters received · 2 artboards")).toBeInTheDocument();
    expect(container.querySelector(".quill-mark.live")).not.toBeNull();
  });

  it("resolves revised artboards by id and opens the existing canvas", () => {
    const onOpen = vi.fn();
    render(<CanvasScopeContext.Provider value={{ boards: new Map([["ideas", { id: "ideas", title: "Ideas", lastChanged: [], artboards: [{ id: "base", name: "Desktop", width: 800 }] }]]), onOpen }}>
      <CanvasProgress call={{ ...call, arguments: { canvas: "ideas", artboards: [{ id: "base", css: "main{}" }] } }} />
    </CanvasScopeContext.Provider>);
    expect(screen.getByText("Writing Desktop")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Open canvas: Ideas" }));
    expect(onOpen).toHaveBeenCalledWith("ideas", ["base"]);
  });
});
