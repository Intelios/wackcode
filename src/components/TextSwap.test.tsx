import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useReducedMotion } from "motion/react";
import { TextSwap } from "./TextSwap";

vi.mock("motion/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("motion/react")>();
  return { ...actual, useReducedMotion: vi.fn(() => false) };
});

afterEach(cleanup);
beforeEach(() => { vi.mocked(useReducedMotion).mockReturnValue(false); });

describe("TextSwap", () => {
  it("updates text instantly while swapKey is unchanged", () => {
    const { rerender } = render(<TextSwap text="Old name" swapKey={0} variant="title" swappedClassName="title-glint" />);
    rerender(<TextSwap text="New name" swapKey={0} variant="title" swappedClassName="title-glint" />);
    expect(screen.getByText("New name")).toBeInTheDocument();
    expect(screen.queryByText("Old name")).toBeNull();
    expect(document.querySelector(".title-glint")).toBeNull();
  });

  it("slides the old text out and glints the new one when swapKey bumps", async () => {
    const { rerender } = render(<TextSwap text="Old name" swapKey={0} variant="title" swappedClassName="title-glint" />);
    rerender(<TextSwap text="New name" swapKey={1} variant="title" swappedClassName="title-glint" />);
    const incoming = screen.getByText("New name");
    expect(incoming).toHaveClass("title-glint");
    await waitFor(() => expect(screen.queryByText("Old name")).toBeNull());
  });

  it("keeps a remount with an already-bumped key unmarked", () => {
    const { unmount } = render(<TextSwap text="New name" swapKey={1} variant="title" swappedClassName="title-glint" />);
    unmount();
    render(<TextSwap text="New name" swapKey={1} variant="title" swappedClassName="title-glint" />);
    expect(document.querySelector(".title-glint")).toBeNull();
  });

  it("crossfades without the glint under reduced motion", async () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    const { rerender } = render(<TextSwap text="Old name" swapKey={0} variant="row" swappedClassName="title-glint" />);
    rerender(<TextSwap text="New name" swapKey={1} variant="row" swappedClassName="title-glint" />);
    expect(document.querySelector(".title-glint")).toBeNull();
    await waitFor(() => expect(screen.getByText("New name")).toBeInTheDocument());
    await waitFor(() => expect(screen.queryByText("Old name")).toBeNull());
  });
});
