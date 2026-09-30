import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GitCommitForm } from "./GitCommitForm";

afterEach(cleanup);

function props() {
  return {
    branch: "main", summary: "Add thing", description: "", stale: false, checkedCount: 3,
    generate: { available: true, running: false }, agentName: "WackCode", committing: false, commitPulse: 0,
    onSummary: vi.fn(), onDescription: vi.fn(), onGenerate: vi.fn(), onCommit: vi.fn()
  };
}

describe("GitCommitForm", () => {
  it("labels the button with the ticked count and branch, and commits on click or ⌘↩", () => {
    const callbacks = props();
    render(<GitCommitForm {...callbacks} />);
    fireEvent.click(screen.getByRole("button", { name: "Commit 3 files to main" }));
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Commit description" }), { key: "Enter", metaKey: true });
    expect(callbacks.onCommit).toHaveBeenCalledTimes(2);
  });

  it("needs a summary and at least one ticked file", () => {
    const callbacks = props();
    const { rerender } = render(<GitCommitForm {...callbacks} summary="  " />);
    expect(screen.getByRole("button", { name: "Commit 3 files to main" })).toBeDisabled();
    rerender(<GitCommitForm {...callbacks} checkedCount={0} />);
    // The label swaps with a short crossfade, so both texts share the button for a moment.
    expect(screen.getByRole("button", { name: /Commit to main/ })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Commit summary" }), { key: "Enter", metaKey: true });
    expect(callbacks.onCommit).not.toHaveBeenCalled();
  });

  it("keeps the summary to one line: Return moves on to the description", () => {
    const callbacks = props();
    render(<GitCommitForm {...callbacks} />);
    const summary = screen.getByRole("textbox", { name: "Commit summary" });
    fireEvent.change(summary, { target: { value: "Add thing\nand more" } });
    expect(callbacks.onSummary).toHaveBeenCalledWith("Add thing and more");
    fireEvent.keyDown(summary, { key: "Enter" });
    expect(screen.getByRole("textbox", { name: "Commit description" })).toHaveFocus();
    expect(callbacks.onCommit).not.toHaveBeenCalled();
  });

  it("counts down past 50 characters and warns past 72", () => {
    const callbacks = props();
    const { rerender } = render(<GitCommitForm {...callbacks} summary={"x".repeat(60)} />);
    expect(screen.getByText("12")).not.toHaveClass("over");
    rerender(<GitCommitForm {...callbacks} summary={"x".repeat(80)} />);
    expect(screen.getByText("-8")).toHaveClass("over");
  });

  it("generates through the agent, and says why it can't", () => {
    const callbacks = props();
    const { rerender } = render(<GitCommitForm {...callbacks} />);
    fireEvent.click(screen.getByRole("button", { name: "Generate commit message with WackCode" }));
    expect(callbacks.onGenerate).toHaveBeenCalled();
    rerender(<GitCommitForm {...callbacks} generate={{ available: false, reason: "Start a chat in this project to generate messages", running: false }} />);
    expect(screen.getByRole("button", { name: "Generate commit message with WackCode" })).toBeDisabled();
  });

  it("locks the commit while a chat is running and notes a stale generated message", () => {
    const callbacks = props();
    render(<GitCommitForm {...callbacks} stale blockedReason="Wait for Fix auth to finish before changing Git files" />);
    expect(screen.getByRole("button", { name: "Commit 3 files to main" })).toBeDisabled();
    expect(screen.getByText("Your changes moved since this message was generated.")).toBeInTheDocument();
  });
});
