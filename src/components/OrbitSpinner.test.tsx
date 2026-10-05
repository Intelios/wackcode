import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useReducedMotion } from "motion/react";
import { OrbitSpinner } from "./OrbitSpinner";

vi.mock("motion/react", () => ({ useReducedMotion: vi.fn(() => false) }));

afterEach(cleanup);

describe("OrbitSpinner", () => {
  it("orbits while active, naming itself Running for the transcript", () => {
    const { container } = render(<OrbitSpinner active />);
    const svg = container.querySelector("svg.orbit-spinner")!;
    expect(svg).toHaveClass("live");
    expect(svg).toHaveAttribute("role", "img");
    expect(svg).toHaveAttribute("aria-label", "Running");
    expect(svg).not.toHaveAttribute("aria-hidden");
    expect(container.querySelectorAll(".orbit-bead")).toHaveLength(3);
  });

  it("renders nothing for a tool that was never watched running", () => {
    const { container } = render(<OrbitSpinner active={false} />);
    expect(container.querySelector("svg")).toBeNull();
  });

  it("lands once the tool finishes, then leaves", () => {
    vi.useFakeTimers();
    const view = render(<OrbitSpinner active />);
    view.rerender(<OrbitSpinner active={false} />);
    const svg = view.container.querySelector("svg")!;
    expect(svg).toHaveClass("landing");
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).not.toHaveAttribute("aria-label");
    act(() => vi.advanceTimersByTime(320));
    expect(view.container.querySelector("svg")).toBeNull();
    vi.useRealTimers();
  });

  it("tints a failed landing and revives if the tool restarts mid-beat", () => {
    vi.useFakeTimers();
    const view = render(<OrbitSpinner active />);
    view.rerender(<OrbitSpinner active={false} failed />);
    expect(view.container.querySelector("svg")).toHaveClass("landing", "failed");
    act(() => vi.advanceTimersByTime(200));
    view.rerender(<OrbitSpinner active />);
    expect(view.container.querySelector("svg")).toHaveClass("live");
    // The beat's timer is gone, so the revived mark keeps orbiting.
    act(() => vi.advanceTimersByTime(1_000));
    expect(view.container.querySelector("svg")).not.toBeNull();
    vi.useRealTimers();
  });

  it("skips the landing beat under reduced motion", () => {
    vi.mocked(useReducedMotion).mockReturnValue(true);
    const view = render(<OrbitSpinner active />);
    view.rerender(<OrbitSpinner active={false} />);
    expect(view.container.querySelector("svg")).toBeNull();
    vi.mocked(useReducedMotion).mockReturnValue(false);
  });
});
