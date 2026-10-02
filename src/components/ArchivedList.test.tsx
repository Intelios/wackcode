import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord, TaskRecord } from "../types";
import { formatRelativeTime } from "../chat-utils";
import { ArchivedList, type ArchivedTaskAction } from "./ArchivedList";

afterEach(cleanup);

const projects: ProjectRecord[] = [
  { id: "p1", name: "TokenTrail", path: "/code/tokentrail", gitRoot: "/code/tokentrail", gitHasHead: true, runCommand: null, branch: "master", createdAt: "now" }
];

function task(id: string, extra: Partial<TaskRecord> = {}): TaskRecord {
  return {
    id, projectId: "p1", name: `Chat ${id}`, autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/tmp", worktreePath: null, branch: null, usesWorktree: false,
    providerId: "prov", modelId: "m", thinkingLevel: "off", sessionFile: null, status: "idle",
    mode: "build", archived: true, archivedAt: "2020-06-15T12:00:00", lastError: null, createdAt: "now", updatedAt: "now",
    ...extra
  };
}

function renderList(tasks: TaskRecord[], handlers: { onSelectTask?: (id: string) => void; onTaskAction?: (task: TaskRecord, action: ArchivedTaskAction) => void } = {}) {
  return render(
    <ArchivedList
      tasks={tasks}
      projects={projects}
      onSelectTask={handlers.onSelectTask ?? (() => undefined)}
      onTaskAction={handlers.onTaskAction ?? (() => undefined)}
    />
  );
}

describe("ArchivedList", () => {
  it("lists archived chats newest-archived first with title, project, and archive age", () => {
    // Timezone-less stamps keep the day stable on any machine; long-past dates keep the
    // relative buckets ("15 Jun 2020") stable on any run date.
    renderList([
      task("old", { name: "Old chat", archivedAt: "2020-01-10T12:00:00" }),
      task("new", { name: "New chat", archivedAt: "2020-06-15T12:00:00" }),
      task("loose", { name: "Loose chat", projectId: null, archivedAt: null, updatedAt: "2020-03-20T12:00:00" })
    ]);
    const titles = [...document.querySelectorAll(".archived-title")].map((el) => el.textContent);
    expect(titles).toEqual(["New chat", "Loose chat", "Old chat"]);
    expect(screen.getAllByText("TokenTrail")).toHaveLength(2);
    expect(screen.getByText("No project")).toBeInTheDocument();
    expect(screen.getByText("15 Jun 2020")).toBeInTheDocument();
    expect(screen.getByText("20 Mar 2020")).toBeInTheDocument();
    expect(screen.getByText("10 Jan 2020")).toBeInTheDocument();
  });

  it("shows a chat's archive age in the relative shorthand", () => {
    renderList([task("t1", { archivedAt: new Date(Date.now() - 5 * 60_000).toISOString() })]);
    expect(screen.getByText("5m")).toBeInTheDocument();
  });

  it("opens a chat when its row is clicked", () => {
    const onSelectTask = vi.fn();
    renderList([task("t1")], { onSelectTask });
    fireEvent.click(screen.getByText("Chat t1"));
    expect(onSelectTask).toHaveBeenCalledWith("t1");
  });

  it("confirms unarchive on second click", () => {
    const onTaskAction = vi.fn();
    const t = task("t1");
    renderList([t], { onTaskAction });

    fireEvent.click(screen.getByRole("button", { name: "Unarchive Chat t1" }));
    expect(onTaskAction).not.toHaveBeenCalled();
    const confirmBtn = screen.getByRole("button", { name: "Confirm unarchive Chat t1" });
    expect(confirmBtn).toHaveTextContent("Unarchive?");

    fireEvent.click(confirmBtn);
    expect(onTaskAction).toHaveBeenCalledWith(t, "unarchive");
  });

  it("confirms delete on second click without a modal", () => {
    const onTaskAction = vi.fn();
    const t = task("t1");
    renderList([t], { onTaskAction });

    fireEvent.click(screen.getByRole("button", { name: "Delete Chat t1" }));
    expect(onTaskAction).not.toHaveBeenCalled();
    const confirmBtn = screen.getByRole("button", { name: "Confirm delete Chat t1" });
    expect(confirmBtn).toHaveTextContent("Delete?");

    fireEvent.click(confirmBtn);
    expect(onTaskAction).toHaveBeenCalledWith(t, "delete-direct");
  });

  it("cancels confirmation when Escape is pressed", () => {
    renderList([task("t1")]);
    fireEvent.click(screen.getByRole("button", { name: "Delete Chat t1" }));
    expect(screen.getByRole("button", { name: "Confirm delete Chat t1" })).toHaveTextContent("Delete?");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Confirm delete Chat t1" })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete Chat t1" })).toBeInTheDocument();
  });

  it("shows an empty state once nothing is archived", () => {
    renderList([]);
    expect(screen.getByText("No archived chats.")).toBeInTheDocument();
  });
});
