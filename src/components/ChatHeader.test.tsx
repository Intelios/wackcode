import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TaskRecord } from "../types";
import { ChatHeader } from "./ChatHeader";

afterEach(cleanup);

function task(id: string, name: string): TaskRecord {
  return {
    id, projectId: "p1", name, autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/tmp", worktreePath: null,
    branch: null, usesWorktree: false, providerId: "p", modelId: "m", thinkingLevel: "off", sessionFile: null,
    status: "idle", mode: "build", archived: false, archivedAt: null, lastError: null, kind: "code", lastActivityAt: null, createdAt: "now", updatedAt: "now"
  };
}

function props(t: TaskRecord, titlePulse = 0) {
  return {
    task: t,
    onSaveRunCommand: vi.fn(async () => {}),
    onRun: vi.fn(async () => {}),
    onStopRun: vi.fn(async () => {}),
    onShowRunOutput: vi.fn(),
    changesOpen: false,
    browserOpen: false,
    onToggleChanges: vi.fn(),
    onToggleBrowser: vi.fn(),
    terminalOpen: false,
    onToggleTerminal: vi.fn(),
    gamesOpen: false,
    onToggleGames: vi.fn(),
    onRename: vi.fn(),
    onTaskAction: vi.fn(),
    onListBranches: vi.fn(async () => ({ current: "main", branches: [] })),
    onCheckoutBranch: vi.fn(async () => {}),
    titlePulse
  };
}

describe("ChatHeader title", () => {
  it("renders the chat name and renames inline on double-click", () => {
    const p = props(task("t1", "Old name"));
    render(<ChatHeader {...p} />);
    expect(screen.getByRole("heading", { name: "Old name" })).toBeInTheDocument();
    fireEvent.doubleClick(screen.getByRole("heading", { name: "Old name" }));
    const input = screen.getByDisplayValue("Old name");
    fireEvent.change(input, { target: { value: "Typed name" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(p.onRename).toHaveBeenCalledWith("Typed name");
  });

  it("swipes and glints the title when the auto-title pulse bumps", async () => {
    const t = task("t1", "Old name");
    const { rerender } = render(<ChatHeader {...props(t)} />);
    rerender(<ChatHeader {...props(task("t1", "Generated title"), 1)} />);
    expect(document.querySelector(".title-glint")).not.toBeNull();
    await waitFor(() => expect(screen.queryByText("Old name")).toBeNull());
    expect(screen.getByRole("heading", { name: "Generated title" })).toBeInTheDocument();
  });

  it("swaps the title silently when only the name changed", () => {
    const t = task("t1", "Old name");
    const { rerender } = render(<ChatHeader {...props(t)} />);
    rerender(<ChatHeader {...props(task("t1", "Renamed chat"))} />);
    expect(screen.getByRole("heading", { name: "Renamed chat" })).toBeInTheDocument();
    expect(screen.queryByText("Old name")).toBeNull();
    expect(document.querySelector(".title-glint")).toBeNull();
  });

  it("never animates a pulse belonging to another chat", () => {
    const { rerender } = render(<ChatHeader {...props(task("t1", "First chat"), 1)} />);
    // A title that arrived while a different chat was open must not replay on switch.
    rerender(<ChatHeader {...props(task("t2", "Second chat"), 2)} />);
    expect(screen.getByRole("heading", { name: "Second chat" })).toBeInTheDocument();
    expect(screen.queryByText("First chat")).toBeNull();
    expect(document.querySelector(".title-glint")).toBeNull();
  });
});

describe("ChatHeader panel buttons", () => {
  it("toggles the Games panel and shows it pressed while open", () => {
    const p = props(task("t1", "Chat"));
    const { rerender } = render(<ChatHeader {...p} />);
    const games = screen.getByRole("button", { name: "Games" });
    expect(games).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(games);
    expect(p.onToggleGames).toHaveBeenCalledOnce();
    rerender(<ChatHeader {...p} gamesOpen />);
    expect(screen.getByRole("button", { name: "Games" })).toHaveAttribute("aria-pressed", "true");
  });
});

describe("ChatHeader in Chat mode", () => {
  it("offers the scratchpad and the browser, and none of a project's chrome", () => {
    const p = props({ ...task("c1", "Trip plan"), kind: "chat", projectId: null, workspacePath: "/data/scratch/c1" });
    render(<ChatHeader {...p} git={{ branch: "main" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Reveal scratchpad" }));
    expect(p.onTaskAction).toHaveBeenCalledWith(p.task, "reveal");
    expect(screen.getByRole("button", { name: "Browser" })).toBeInTheDocument();
    for (const name of ["Changes", "Terminal", "Games"]) expect(screen.queryByRole("button", { name })).toBeNull();
    // Neither the folder's path nor a branch: a scratchpad is not a checkout.
    expect(screen.queryByText(/scratch\/c1/)).toBeNull();
    expect(screen.queryByText("main")).toBeNull();
  });

  it("keeps the project chrome for a coding chat", () => {
    render(<ChatHeader {...props(task("t1", "Fix the build"))} />);
    for (const name of ["Browser", "Changes", "Terminal", "Games"]) expect(screen.getByRole("button", { name })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reveal scratchpad" })).toBeNull();
  });
});
