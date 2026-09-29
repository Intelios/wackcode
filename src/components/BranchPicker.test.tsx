import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GitBranches } from "../types";
import { BranchPicker } from "./BranchPicker";

afterEach(cleanup);

const listed: GitBranches = {
  current: "main",
  branches: [
    { name: "main", remote: false, worktree: null },
    { name: "feature/login", remote: false, worktree: null },
    { name: "wackcode/chat-1234", remote: false, worktree: "/tmp/worktrees/1234" },
    { name: "origin/release", remote: true, worktree: null }
  ]
};

function renderPicker(onCheckout = vi.fn(async () => undefined), branch: string | null = "main") {
  render(<BranchPicker branch={branch} variant="meta" onLoad={async () => listed} onCheckout={onCheckout} />);
  fireEvent.click(screen.getByRole("button", { name: /Switch branch/ }));
  return onCheckout;
}

describe("BranchPicker", () => {
  it("lists local and remote branches and switches to one", async () => {
    const onCheckout = renderPicker();
    fireEvent.click(await screen.findByRole("button", { name: "feature/login" }));
    await waitFor(() => expect(onCheckout).toHaveBeenCalledWith("feature/login", "local"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Switch branch" })).toBeNull());
  });

  it("tracks a remote branch and leaves ones checked out elsewhere disabled", async () => {
    const onCheckout = renderPicker();
    expect(await screen.findByRole("button", { name: /wackcode\/chat-1234/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "origin/release" }));
    await waitFor(() => expect(onCheckout).toHaveBeenCalledWith("origin/release", "remote"));
  });

  it("filters by the search and offers to create a missing branch", async () => {
    const onCheckout = renderPicker();
    await screen.findByRole("button", { name: "feature/login" });
    const search = screen.getByRole("textbox", { name: "Find or create a branch" });
    fireEvent.change(search, { target: { value: "fix/typo" } });
    expect(screen.queryByRole("button", { name: "feature/login" })).toBeNull();
    expect(screen.getByRole("button", { name: "Create fix/typo" })).toBeInTheDocument();
    fireEvent.keyDown(search, { key: "Enter" });
    await waitFor(() => expect(onCheckout).toHaveBeenCalledWith("fix/typo", "create"));
  });

  it("picks the first match on Enter and keeps the popover open on failure", async () => {
    const onCheckout = vi.fn(async () => { throw "Commit or discard your changes first: switching to feature/login would overwrite them"; });
    renderPicker(onCheckout);
    await screen.findByRole("button", { name: "feature/login" });
    const search = screen.getByRole("textbox", { name: "Find or create a branch" });
    fireEvent.change(search, { target: { value: "feat" } });
    fireEvent.keyDown(search, { key: "Enter" });
    expect(await screen.findByRole("alert")).toHaveTextContent("Commit or discard");
    expect(onCheckout).toHaveBeenCalledWith("feature/login", "local");
    expect(screen.getByRole("dialog", { name: "Switch branch" })).toBeInTheDocument();
  });

  it("labels a detached HEAD", () => {
    render(<BranchPicker branch={null} variant="pill" onLoad={async () => listed} onCheckout={async () => undefined} />);
    expect(screen.getByRole("button", { name: /Detached HEAD/ })).toBeInTheDocument();
  });
});
