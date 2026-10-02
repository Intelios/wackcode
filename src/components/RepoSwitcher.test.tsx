import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord } from "../types";
import { RepoSwitcher } from "./RepoSwitcher";

afterEach(cleanup);

function project(id: string, name: string, gitRoot: string | null = `/code/${name}`): ProjectRecord {
  return { id, name, path: `/code/${name}`, gitRoot, gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" };
}

const projects = [project("a", "alpha"), project("b", "beta"), project("c", "gamma"), project("d", "notes", null)];

function open(pinned: string[] = ["c"]) {
  const callbacks = { onSelect: vi.fn(), onSetPinned: vi.fn(), onAddProject: vi.fn() };
  render(<RepoSwitcher projects={projects} currentId="a" branch="main" pinned={new Set(pinned)} {...callbacks} />);
  fireEvent.click(screen.getByRole("button", { name: /Current repository: alpha/ }));
  return callbacks;
}

describe("RepoSwitcher", () => {
  it("lists pinned projects first and switches repository", () => {
    const callbacks = open();
    const dialog = screen.getByRole("dialog", { name: "Switch repository" });
    const names = within(dialog).getAllByRole("button", { name: /^(alpha|beta|gamma|notes)/ }).map((row) => row.textContent);
    expect(names[0]).toContain("gamma");
    expect(within(dialog).getByText("Pinned")).toBeInTheDocument();
    expect(within(dialog).getByText("not Git")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: /^beta/ }));
    expect(callbacks.onSelect).toHaveBeenCalledWith("b");
  });

  it("pins and unpins from each row", () => {
    const callbacks = open();
    fireEvent.click(screen.getByRole("button", { name: "Pin beta" }));
    expect(callbacks.onSetPinned).toHaveBeenCalledWith("b", true);
    fireEvent.click(screen.getByRole("button", { name: "Unpin gamma" }));
    expect(callbacks.onSetPinned).toHaveBeenCalledWith("c", false);
  });

  it("filters by name and picks the highlighted row with Enter", () => {
    const callbacks = open([]);
    const filter = screen.getByRole("textbox", { name: "Filter repositories" });
    fireEvent.change(filter, { target: { value: "gam" } });
    expect(screen.queryByRole("button", { name: /^beta/ })).toBeNull();
    fireEvent.keyDown(filter, { key: "Enter" });
    expect(callbacks.onSelect).toHaveBeenCalledWith("c");
  });
});
