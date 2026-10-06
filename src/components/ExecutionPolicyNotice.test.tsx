import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_EXECUTION_POLICY } from "../execution-policy";
import { ExecutionPolicyNotice } from "./ExecutionPolicyNotice";

afterEach(() => { cleanup(); vi.useRealTimers(); });
const unlocked = { unrestrictedPlanning: true, unrestrictedSubagents: true };
describe("execution policy notice", () => {
  it("shows nothing while idle, however restrictions are set", () => {
    render(<ExecutionPolicyNotice saved={unlocked} running={false} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("shows nothing while a run already applies the saved policy", () => {
    render(<ExecutionPolicyNotice saved={unlocked} applied={unlocked} running />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("announces a saved change the running turn has not applied", () => {
    render(<ExecutionPolicyNotice saved={DEFAULT_EXECUTION_POLICY} applied={unlocked} running />);
    const notice = screen.getByRole("status");
    expect(notice).toHaveTextContent("Applies next turn");
    expect(notice).not.toHaveTextContent("Read-only off");
  });

  it("reveals the explanation on keyboard focus", () => {
    vi.useFakeTimers();
    render(<ExecutionPolicyNotice saved={unlocked} applied={DEFAULT_EXECUTION_POLICY} running />);
    fireEvent.mouseEnter(screen.getByRole("status").parentElement!);
    fireEvent.focus(screen.getByRole("status"));
    act(() => vi.advanceTimersByTime(400));
    expect(screen.getByRole("tooltip")).toHaveTextContent("apply next turn");
    fireEvent.blur(screen.getByRole("status"));
    act(() => vi.advanceTimersByTime(400));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
});
