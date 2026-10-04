import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_EXECUTION_POLICY } from "../execution-policy";
import { ExecutionPolicyNotice } from "./ExecutionPolicyNotice";

afterEach(() => { cleanup(); vi.useRealTimers(); });
const unlocked = { unrestrictedPlanning: true, unrestrictedSubagents: true };
describe("active access indicator", () => {
  it("keeps an active child's warning when disabling sub-agents is queued", () => {
    render(<ExecutionPolicyNotice saved={unlocked} applied={unlocked} running mode="build" subagentsEnabled={false} appliedSubagentsEnabled />);
    expect(screen.getByRole("status")).toHaveTextContent("Read-only off");
  });
  it("reveals the access explanation on keyboard focus", () => {
    vi.useFakeTimers();
    render(<ExecutionPolicyNotice saved={unlocked} running={false} mode="ultraplan" subagentsEnabled />);
    fireEvent.mouseEnter(screen.getByRole("status").parentElement!);
    fireEvent.focus(screen.getByRole("status"));
    act(() => vi.advanceTimersByTime(400));
    expect(screen.getByRole("tooltip")).toHaveTextContent("Dangerous and not recommended");
    fireEvent.blur(screen.getByRole("status"));
    act(() => vi.advanceTimersByTime(400));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
  it("retains the active warning while disabling waits for the current run", () => {
    render(<ExecutionPolicyNotice saved={DEFAULT_EXECUTION_POLICY} applied={unlocked} running mode="plan" subagentsEnabled />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("Read-only off");
    expect(notice).toHaveTextContent("Applies next turn");
    expect(notice).toHaveAccessibleName(/Plan \/ Ultra Plan and normally read-only sub-agents/);
  });
  it("shows a pending enable without claiming the running turn is unrestricted", () => {
    render(<ExecutionPolicyNotice saved={unlocked} applied={DEFAULT_EXECUTION_POLICY} running mode="ultraplan" subagentsEnabled />);
    expect(screen.getByRole("status")).toHaveTextContent("Applies next turn");
    expect(screen.queryByText("Read-only off")).not.toBeInTheDocument();
  });
  it("keeps the planning ceiling and does not warn about disabled sub-agents", () => {
    const { rerender } = render(<ExecutionPolicyNotice saved={{ ...DEFAULT_EXECUTION_POLICY, unrestrictedSubagents: true }} running={false} mode="plan" subagentsEnabled />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    rerender(<ExecutionPolicyNotice saved={unlocked} running={false} mode="build" subagentsEnabled={false} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});
