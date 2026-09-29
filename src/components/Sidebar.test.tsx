import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ProjectRecord, TaskRecord } from "../types";
import { NO_PROJECT_KEY, Sidebar, type TaskAction } from "./Sidebar";

afterEach(cleanup);

const projects: ProjectRecord[] = [
  { id: "p1", name: "TokenTrail", path: "/code/tokentrail", gitRoot: "/code/tokentrail", gitHasHead: true, branch: "master", createdAt: "now" }
];

function task(id: string, projectId: string | null, name: string): TaskRecord {
  return {
    id, projectId, name, autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/tmp", worktreePath: null, branch: null, usesWorktree: false,
    providerId: "prov", modelId: "m", thinkingLevel: "off", sessionFile: null, status: "idle",
    mode: "build", archived: false, archivedAt: null, lastError: null, createdAt: "now", updatedAt: "now"
  };
}

function Harness({ tasks, pendingDialogTaskIds = new Set<string>() }: { tasks: TaskRecord[]; pendingDialogTaskIds?: ReadonlySet<string> }) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  return (
    <Sidebar
      projects={projects}
      tasks={tasks}
      archivedOpen={false}
      pendingDialogTaskIds={pendingDialogTaskIds}
      collapsedProjectIds={collapsed}
      onSelectTask={() => undefined}
      onNewChat={() => undefined}
      onNewDraft={() => undefined}
      onAddProject={() => undefined}
      onToggleArchived={() => undefined}
      onToggleProjectCollapsed={(key) => setCollapsed((current) => {
        const next = new Set(current);
        if (next.has(key)) next.delete(key); else next.add(key);
        return next;
      })}
      onOpenSettings={() => undefined}
      onTaskAction={() => undefined}
      onProjectAction={() => undefined}
      onRenameTask={() => undefined}
      onArchiveAll={() => undefined}
      onDeleteAllArchived={() => undefined}
    />
  );
}

describe("Sidebar collapsible projects", () => {
  it("hides a project's chats when its heading is clicked and shows a count", () => {
    render(<Harness tasks={[task("t1", "p1", "Refactor parser"), task("t2", null, "Loose chat")]} />);
    fireEvent.click(screen.getByText("TokenTrail"));
    expect(screen.queryByText("Refactor parser")).toBeNull();
    expect(screen.getByText("1")).toBeInTheDocument();
    fireEvent.click(screen.getByText("TokenTrail"));
    expect(screen.getByText("Refactor parser")).toBeInTheDocument();
    expect(screen.queryByText("1")).toBeNull();
  });

  it("reflects the collapsed state on the chevron button", () => {
    render(<Harness tasks={[task("t1", "p1", "Refactor parser")]} />);
    const chevron = screen.getByRole("button", { name: "Collapse TokenTrail" });
    expect(chevron).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(chevron);
    expect(screen.getByRole("button", { name: "Expand TokenTrail" })).toHaveAttribute("aria-expanded", "false");
  });

  it("does not toggle collapse when a heading action button is clicked", () => {
    const newChat: (string | null)[] = [];
    function ActionHarness() {
      const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set(["p1"]));
      return (
        <Sidebar
          projects={projects}
          tasks={[task("t1", "p1", "Refactor parser")]}
          archivedOpen={false}
          pendingDialogTaskIds={new Set<string>()}
          collapsedProjectIds={collapsed}
          onSelectTask={() => undefined}
          onNewChat={(project) => newChat.push(project?.id ?? null)}
          onNewDraft={() => undefined}
          onAddProject={() => undefined}
          onToggleArchived={() => undefined}
          onToggleProjectCollapsed={(key) => setCollapsed((current) => {
            const next = new Set(current);
            if (next.has(key)) next.delete(key); else next.add(key);
            return next;
          })}
          onOpenSettings={() => undefined}
          onTaskAction={() => undefined}
          onProjectAction={() => undefined}
          onRenameTask={() => undefined}
          onArchiveAll={() => undefined}
          onDeleteAllArchived={() => undefined}
        />
      );
    }
    render(<ActionHarness />);
    fireEvent.click(screen.getByRole("button", { name: "New chat in TokenTrail" }));
    expect(newChat).toEqual(["p1"]);
    expect(screen.queryByText("Refactor parser")).toBeNull();
    expect(screen.getByText("1")).toBeInTheDocument();
  });

  it("flags a collapsed group hiding a chat that awaits an answer", () => {
    render(<Harness tasks={[task("t1", "p1", "Refactor parser")]} pendingDialogTaskIds={new Set(["t1"])} />);
    fireEvent.click(screen.getByText("TokenTrail"));
    expect(screen.getByTitle("A chat in this group is waiting for your answer")).toBeInTheDocument();
  });

  it("labels Plan and Ultra Plan chats with their own chips", () => {
    const planned = { ...task("t1", "p1", "Refactor parser"), mode: "plan" as const };
    const grilled = { ...task("t2", "p1", "Rework auth"), mode: "ultraplan" as const };
    render(<Harness tasks={[planned, grilled, task("t3", "p1", "Fix typo")]} />);
    expect(screen.getByText("Plan")).toHaveClass("task-mode-chip");
    expect(screen.getByText("Ultra Plan")).toHaveClass("task-mode-chip", "ultra");
    expect(screen.getAllByText(/^(Ultra )?Plan$/)).toHaveLength(2);
  });

  it("collapses the No project group under its own key", () => {
    const toggled: string[] = [];
    function LooseHarness() {
      return (
        <Sidebar
          projects={projects}
          tasks={[task("t2", null, "Loose chat")]}
          archivedOpen={false}
          pendingDialogTaskIds={new Set<string>()}
          collapsedProjectIds={new Set([NO_PROJECT_KEY])}
          onSelectTask={() => undefined}
          onNewChat={() => undefined}
          onNewDraft={() => undefined}
          onAddProject={() => undefined}
          onToggleArchived={() => undefined}
          onToggleProjectCollapsed={(key) => toggled.push(key)}
          onOpenSettings={() => undefined}
          onTaskAction={() => undefined}
          onProjectAction={() => undefined}
          onRenameTask={() => undefined}
          onArchiveAll={() => undefined}
          onDeleteAllArchived={() => undefined}
        />
      );
    }
    render(<LooseHarness />);
    expect(screen.queryByText("Loose chat")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Expand No project" }));
    expect(toggled).toEqual([NO_PROJECT_KEY]);
  });
});

describe("Sidebar footer tiles", () => {
  function FooterHarness({ tasks }: { tasks: TaskRecord[] }) {
    const [archivedOpen, setArchivedOpen] = useState(false);
    return (
      <Sidebar
        projects={projects}
        tasks={tasks}
        archivedOpen={archivedOpen}
        pendingDialogTaskIds={new Set<string>()}
        collapsedProjectIds={new Set()}
        onSelectTask={() => undefined}
        onNewChat={() => undefined}
        onNewDraft={() => undefined}
        onAddProject={() => undefined}
        onToggleArchived={() => setArchivedOpen((current) => !current)}
        onToggleProjectCollapsed={() => undefined}
        onOpenSettings={() => undefined}
        onTaskAction={() => undefined}
        onProjectAction={() => undefined}
        onRenameTask={() => undefined}
        onArchiveAll={() => undefined}
        onDeleteAllArchived={() => undefined}
      />
    );
  }

  it("renders Add project and Settings tiles", () => {
    render(<FooterHarness tasks={[task("t1", "p1", "Refactor parser")]} />);
    expect(screen.getByRole("button", { name: "Add project" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("shows the archive tile only when an archived chat exists, and opens the Archived view from it", async () => {
    const archived = { ...task("t2", null, "Old chat"), archived: true, archivedAt: "2026-09-27T10:00:00Z" };
    const { rerender } = render(<FooterHarness tasks={[task("t1", "p1", "Refactor parser")]} />);
    expect(screen.queryByRole("button", { name: "Show archived" })).toBeNull();

    rerender(<FooterHarness tasks={[task("t1", "p1", "Refactor parser"), archived]} />);
    const tile = screen.getByRole("button", { name: "Show archived" });
    expect(tile).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(tile);
    expect(screen.getByRole("button", { name: "Hide archived" })).toHaveAttribute("aria-pressed", "true");
    // The Archived view replaces the chat list, with its own header and close button.
    expect(await screen.findByRole("heading", { name: "Archived" })).toBeInTheDocument();
    expect(await screen.findByText("Old chat")).toBeInTheDocument();
    expect(screen.queryByText("Refactor parser")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close archived chats" }));
    expect(await screen.findByText("Refactor parser")).toBeInTheDocument();
  });
});

describe("Sidebar task actions", () => {
  function ActionHarness({
    tasks,
    onTaskAction = () => undefined
  }: {
    tasks: TaskRecord[];
    onTaskAction?: (task: TaskRecord, action: TaskAction) => void;
  }) {
    return (
      <Sidebar
        projects={projects}
        tasks={tasks}
        archivedOpen={false}
        pendingDialogTaskIds={new Set<string>()}
        collapsedProjectIds={new Set()}
        onSelectTask={() => undefined}
        onNewChat={() => undefined}
        onNewDraft={() => undefined}
        onAddProject={() => undefined}
        onToggleArchived={() => undefined}
        onToggleProjectCollapsed={() => undefined}
        onOpenSettings={() => undefined}
        onTaskAction={onTaskAction}
        onProjectAction={() => undefined}
        onRenameTask={() => undefined}
        onArchiveAll={() => undefined}
        onDeleteAllArchived={() => undefined}
      />
    );
  }

  it("renders Archive and Delete buttons on each chat row", () => {
    render(<ActionHarness tasks={[task("t1", "p1", "Refactor parser")]} />);
    expect(screen.getByRole("button", { name: "Archive Refactor parser" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete Refactor parser" })).toBeInTheDocument();
  });

  it("confirms delete on second click without a modal", () => {
    const actions: [TaskRecord, TaskAction][] = [];
    const t = task("t1", "p1", "Refactor parser");
    render(<ActionHarness tasks={[t]} onTaskAction={(task, action) => actions.push([task, action])} />);

    const deleteBtn = screen.getByRole("button", { name: "Delete Refactor parser" });
    fireEvent.click(deleteBtn);

    expect(actions).toHaveLength(0);
    const confirmBtn = screen.getByRole("button", { name: "Confirm delete Refactor parser" });
    expect(confirmBtn).toHaveTextContent("Delete?");

    fireEvent.click(confirmBtn);
    expect(actions).toEqual([[t, "delete-direct"]]);
  });

  it("confirms archive on second click", () => {
    const actions: [TaskRecord, TaskAction][] = [];
    const t = task("t1", "p1", "Refactor parser");
    render(<ActionHarness tasks={[t]} onTaskAction={(task, action) => actions.push([task, action])} />);

    const archiveBtn = screen.getByRole("button", { name: "Archive Refactor parser" });
    fireEvent.click(archiveBtn);

    expect(actions).toHaveLength(0);
    const confirmBtn = screen.getByRole("button", { name: "Confirm archive Refactor parser" });
    expect(confirmBtn).toHaveTextContent("Archive?");

    fireEvent.click(confirmBtn);
    expect(actions).toEqual([[t, "archive"]]);
  });

  it("keeps archived chats out of the normal chat list", () => {
    const archived = { ...task("t2", "p1", "Old chat"), archived: true };
    render(<ActionHarness tasks={[task("t1", "p1", "Refactor parser"), archived]} />);
    expect(screen.getByText("Refactor parser")).toBeInTheDocument();
    expect(screen.queryByText("Old chat")).toBeNull();
  });

  it("cancels confirmation when clicking outside", () => {
    const t = task("t1", "p1", "Refactor parser");
    render(<ActionHarness tasks={[t]} />);

    fireEvent.click(screen.getByRole("button", { name: "Delete Refactor parser" }));
    expect(screen.getByRole("button", { name: "Confirm delete Refactor parser" })).toHaveTextContent("Delete?");

    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("button", { name: "Confirm delete Refactor parser" })).toBeNull();
    expect(screen.getByRole("button", { name: "Delete Refactor parser" })).toBeInTheDocument();
  });

  it("cancels confirmation when Escape is pressed", () => {
    const t = task("t1", "p1", "Refactor parser");
    render(<ActionHarness tasks={[t]} />);

    fireEvent.click(screen.getByRole("button", { name: "Archive Refactor parser" }));
    expect(screen.getByRole("button", { name: "Confirm archive Refactor parser" })).toHaveTextContent("Archive?");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Confirm archive Refactor parser" })).toBeNull();
    expect(screen.getByRole("button", { name: "Archive Refactor parser" })).toBeInTheDocument();
  });

  it("switches confirmation between archive and delete on the same task", () => {
    const t = task("t1", "p1", "Refactor parser");
    render(<ActionHarness tasks={[t]} />);

    fireEvent.click(screen.getByRole("button", { name: "Archive Refactor parser" }));
    expect(screen.getByRole("button", { name: "Confirm archive Refactor parser" })).toHaveTextContent("Archive?");

    fireEvent.click(screen.getByRole("button", { name: "Delete Refactor parser" }));
    expect(screen.queryByRole("button", { name: "Confirm archive Refactor parser" })).toBeNull();
    expect(screen.getByRole("button", { name: "Confirm delete Refactor parser" })).toHaveTextContent("Delete?");
  });
});


describe("Sidebar bulk actions", () => {
  function BulkHarness({ tasks, archivedOpen = false, onArchiveAll = () => undefined, onDeleteAllArchived = () => undefined }: {
    tasks: TaskRecord[];
    archivedOpen?: boolean;
    onArchiveAll?: (projectId: string | null) => void;
    onDeleteAllArchived?: () => void;
  }) {
    return (
      <Sidebar
        projects={projects}
        tasks={tasks}
        archivedOpen={archivedOpen}
        pendingDialogTaskIds={new Set<string>()}
        collapsedProjectIds={new Set()}
        onSelectTask={() => undefined}
        onNewChat={() => undefined}
        onNewDraft={() => undefined}
        onAddProject={() => undefined}
        onToggleArchived={() => undefined}
        onToggleProjectCollapsed={() => undefined}
        onOpenSettings={() => undefined}
        onTaskAction={() => undefined}
        onProjectAction={() => undefined}
        onRenameTask={() => undefined}
        onArchiveAll={onArchiveAll}
        onDeleteAllArchived={onDeleteAllArchived}
      />
    );
  }

  it("archives all chats in a project, or with no project, from the group menu", () => {
    const calls: (string | null)[] = [];
    render(<BulkHarness tasks={[task("t1", "p1", "Refactor parser"), task("t2", null, "Loose chat")]} onArchiveAll={(id) => calls.push(id)} />);
    fireEvent.click(screen.getByRole("button", { name: "TokenTrail menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Archive all chats" }));
    fireEvent.click(screen.getByRole("button", { name: "No project menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Archive all chats" }));
    expect(calls).toEqual(["p1", null]);
  });

  it("disables Archive all chats for a project with no open chats", () => {
    render(<BulkHarness tasks={[{ ...task("t1", "p1", "Old chat"), archived: true }]} />);
    fireEvent.click(screen.getByRole("button", { name: "TokenTrail menu" }));
    expect(screen.getByRole("menuitem", { name: "Archive all chats" })).toBeDisabled();
  });

  it("offers Delete all in the Archived view header", async () => {
    let deleted = 0;
    render(<BulkHarness tasks={[{ ...task("t1", "p1", "Old chat"), archived: true }]} archivedOpen onDeleteAllArchived={() => { deleted += 1; }} />);
    fireEvent.click(screen.getByRole("button", { name: "Delete all archived chats" }));
    expect(deleted).toBe(1);
  });
});
