import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GoalState } from "../types";
import { GoalBanner } from "./GoalBanner";

afterEach(cleanup);

function goal(phase: GoalState["phase"], patch: Partial<GoalState> = {}): GoalState {
  return { objective: "Ship the feature", phase, iteration: 2, maxIterations: 25, noProgress: 0, ...patch };
}

describe("GoalBanner", () => {
  it("renders nothing without a goal", () => {
    const { container } = render(<GoalBanner onAction={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the round, next action and pause control while running", () => {
    const onAction = vi.fn();
    render(<GoalBanner goal={goal("verifying", { lastNextAction: "run the tests" })} onAction={onAction} />);
    const banner = screen.getByRole("region", { name: "Goal" });
    expect(banner).toHaveTextContent("2/25");
    expect(banner).toHaveTextContent("run the tests");
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(onAction).toHaveBeenCalledWith("pause");
  });

  it("offers resume and clear while paused", () => {
    const onAction = vi.fn();
    render(<GoalBanner goal={goal("paused", { note: "Stopped by user." })} onAction={onAction} />);
    const banner = screen.getByRole("region", { name: "Goal" });
    expect(banner).toHaveTextContent("Goal paused");
    expect(banner).toHaveTextContent("Stopped by user.");
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(onAction).toHaveBeenCalledWith("resume");
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(onAction).toHaveBeenCalledWith("clear");
  });

  it("shows a dismiss button once the goal ends", () => {
    const onAction = vi.fn();
    render(<GoalBanner goal={goal("complete", { lastReason: "all verified" })} onAction={onAction} />);
    const banner = screen.getByRole("region", { name: "Goal" });
    expect(banner).toHaveTextContent("Goal complete");
    expect(banner).toHaveTextContent("all verified");
    expect(screen.queryByRole("button", { name: "Pause" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onAction).toHaveBeenCalledWith("clear");
  });
});
