import { useState } from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord, TaskRecord } from "../types";
import { NO_PROJECT_KEY, Sidebar, type TaskAction } from "./Sidebar";

afterEach(() => { cleanup(); vi.useRealTimers(); });

const projects: ProjectRecord[] = [
  { id: "p1", name: "TokenTrail", path: "/code/tokentrail", gitRoot: "/code/tokentrail", gitHasHead: true, runCommand: null, branch: "master", createdAt: "now" }
];

function task(id: string, projectId: string | null, name: string): TaskRecord {
  return {
    id, projectId, name, autoTitleEligible: false, autoTitleAttemptId: null, workspacePath: "/tmp", worktreePath: null, branch: null, usesWorktree: false,
    providerId: "prov", modelId: "m", thinkingLevel: "off", sessionFile: null, status: "idle",
    mode: "build", archived: false, archivedAt: null, lastError: null, createdAt: "now", updatedAt: "now"
  };
}

function Harness({ tasks, pendingDialogTaskIds = new Set<string>(), archivedOpen = false, onSelectTask = () => undefined, titlePulses = {} }: { tasks: TaskRecord[]; pendingDialogTaskIds?: ReadonlySet<string>; archivedOpen?: boolean; onSelectTask?: (id: string) => void; titlePulses?: Record<string, number> }) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  return (
    <Sidebar
      projects={projects}
      tasks={tasks}
      archivedOpen={archivedOpen}
      pendingDialogTaskIds={pendingDialogTaskIds}
      titlePulses={titlePulses}
      collapsedProjectIds={collapsed}
      onSelectTask={onSelectTask}
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
      pinnedProjectIds={new Set<string>()}
      onToggleGit={null}
    />
  );
}

describe("Sidebar order, timestamps and search", () => {
  it("orders chats newest first in both project and projectless groups", () => {
    const dated = (id: string, projectId: string | null, createdAt: string) => ({ ...task(id, projectId, id), createdAt });
    render(<Harness tasks={[
      dated("Project old", "p1", "2020-01-10T12:00:00"),
      dated("Loose old", null, "2020-02-10T12:00:00"),
      dated("Project new", "p1", "2020-06-10T12:00:00"),
      dated("Loose new", null, "2020-07-10T12:00:00")
    ]} />);
    expect(screen.getAllByRole("button", { name: /^(Project|Loose) (old|new)$/ }).map((row) => row.getAttribute("aria-label")))
      .toEqual(["Project new", "Project old", "Loose new", "Loose old"]);
    const row = screen.getByRole("button", { name: "Project new" });
    expect(within(row).getByText("10 Jun 2020")).toHaveAttribute("datetime", "2020-06-10T12:00:00");
    expect(within(row).getByText("10 Jun 2020")).toHaveAttribute("title", `Created ${new Date("2020-06-10T12:00:00").toLocaleString()}`);
  });

  it("refreshes relative ages while idle and clears its timer on unmount", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-30T12:00:00"));
    const { unmount } = render(<Harness tasks={[{ ...task("t1", "p1", "Fresh chat"), createdAt: "2026-09-30T11:59:00" }]} />);
    expect(screen.getByText("1m")).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(60_000); });
    expect(screen.getByText("2m")).toBeInTheDocument();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("reveals matches in collapsed groups and restores collapse when cleared", () => {
    const selected = vi.fn();
    render(<Harness tasks={[task("t1", "p1", "Refactor parser"), task("t2", null, "Loose parser"), task("t3", "p1", "Fix typo")]} onSelectTask={selected} />);
    fireEvent.click(screen.getByRole("button", { name: "Collapse TokenTrail" }));
    fireEvent.click(screen.getByRole("button", { name: "Collapse No project" }));
    const search = screen.getByRole("searchbox", { name: "Search chats" });
    fireEvent.change(search, { target: { value: "  PARSER " } });
    expect(screen.getByRole("button", { name: "Refactor parser" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Loose parser" })).toBeInTheDocument();
    expect(screen.queryByText("Fix typo")).toBeNull();
    expect(screen.getByRole("button", { name: "Collapse TokenTrail" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("button", { name: "Refactor parser" }), { key: "Enter" });
    expect(selected).toHaveBeenCalledWith("t1");
    fireEvent.click(screen.getByRole("button", { name: "Clear chat search" }));
    expect(search).toHaveFocus();
    expect(screen.queryByText("Refactor parser")).toBeNull();
    expect(screen.queryByText("Loose parser")).toBeNull();
    expect(screen.getByRole("button", { name: "Expand TokenTrail" })).toHaveAttribute("aria-expanded", "false");
  });

  it("matches project names, hides unrelated groups, and clears with Escape", () => {
    render(<Harness tasks={[task("t1", "p1", "Refactor parser"), task("t2", null, "Loose chat")]} />);
    const search = screen.getByRole("searchbox", { name: "Search chats" });
    fireEvent.change(search, { target: { value: "tokentrail" } });
    expect(screen.getByText("Refactor parser")).toBeInTheDocument();
    expect(screen.queryByText("No project")).toBeNull();
    fireEvent.change(search, { target: { value: "missing" } });
    expect(screen.getByRole("status")).toHaveTextContent("No chats match your search.");
    expect(screen.queryByText("TokenTrail")).toBeNull();
    fireEvent.keyDown(search, { key: "Escape" });
    expect(search).toHaveValue("");
    expect(screen.getByText("Loose chat")).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("searches archived chats with their archive order and a distinct empty state", () => {
    render(<Harness archivedOpen tasks={[
      { ...task("t1", "p1", "Old parser"), archived: true, archivedAt: "2020-01-10T12:00:00" },
      { ...task("t2", "p1", "New parser"), archived: true, archivedAt: "2020-06-10T12:00:00" },
      task("t3", "p1", "Live parser")
    ]} />);
    const search = screen.getByRole("searchbox", { name: "Search archived chats" });
    fireEvent.change(search, { target: { value: "parser" } });
    expect(screen.getAllByText(/^(Old|New) parser$/).map((title) => title.textContent)).toEqual(["New parser", "Old parser"]);
    expect(screen.queryByText("Live parser")).toBeNull();
    fireEvent.change(search, { target: { value: "missing" } });
    expect(screen.getByRole("status")).toHaveTextContent("No archived chats match your search.");
    expect(screen.getByRole("button", { name: "Delete all archived chats" })).toBeInTheDocument();
  });
});

describe("Sidebar title echo", () => {
  it("crossfades the row's name when its chat's title pulse bumps", async () => {
    const { rerender } = render(<Harness tasks={[task("t1", "p1", "Old name")]} titlePulses={{}} />);
    rerender(<Harness tasks={[task("t1", "p1", "Generated title")]} titlePulses={{ t1: 1 }} />);
    await waitFor(() => expect(screen.queryByText("Old name")).toBeNull());
    expect(screen.getByRole("button", { name: "Generated title" })).toBeInTheDocument();
  });

  it("updates the row's name instantly when no pulse changed", () => {
    const { rerender } = render(<Harness tasks={[task("t1", "p1", "Old name")]} titlePulses={{}} />);
    rerender(<Harness tasks={[task("t1", "p1", "Renamed chat")]} titlePulses={{}} />);
    expect(screen.getByRole("button", { name: "Renamed chat" })).toBeInTheDocument();
    expect(screen.queryByText("Old name")).toBeNull();
  });
});

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
          titlePulses={{}}
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
          pinnedProjectIds={new Set<string>()}
          onToggleGit={null}
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
          titlePulses={{}}
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
          pinnedProjectIds={new Set<string>()}
          onToggleGit={null}
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
        titlePulses={{}}
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
        pinnedProjectIds={new Set<string>()}
        onToggleGit={null}
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
        titlePulses={{}}
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
        pinnedProjectIds={new Set<string>()}
        onToggleGit={null}
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
        titlePulses={{}}
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
        pinnedProjectIds={new Set<string>()}
        onToggleGit={null}
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

describe("Sidebar pinning and Git mode", () => {
  const many: ProjectRecord[] = [
    { id: "p1", name: "Alpha", path: "/code/alpha", gitRoot: "/code/alpha", gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" },
    { id: "p2", name: "Beta", path: "/code/beta", gitRoot: "/code/beta", gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" },
    { id: "p3", name: "Gamma", path: "/code/gamma", gitRoot: "/code/gamma", gitHasHead: true, runCommand: null, branch: "main", createdAt: "now" }
  ];

  function PinHarness({ pinned = [], git, onProjectAction = () => undefined, onToggleGit = () => undefined }: {
    pinned?: string[];
    git?: { top: React.ReactNode; page: React.ReactNode };
    onProjectAction?: (project: ProjectRecord, action: string) => void;
    onToggleGit?: (() => void) | null;
  }) {
    return (
      <Sidebar
        projects={many}
        pinnedProjectIds={new Set(pinned)}
        tasks={[task("t1", "p1", "Alpha chat")]}
        archivedOpen={false}
        pendingDialogTaskIds={new Set()}
        titlePulses={{}}
        collapsedProjectIds={new Set()}
        onSelectTask={() => undefined}
        onNewChat={() => undefined}
        onNewDraft={() => undefined}
        onAddProject={() => undefined}
        onToggleArchived={() => undefined}
        onToggleProjectCollapsed={() => undefined}
        onOpenSettings={() => undefined}
        onTaskAction={() => undefined}
        onProjectAction={onProjectAction}
        onRenameTask={() => undefined}
        onArchiveAll={() => undefined}
        onDeleteAllArchived={() => undefined}
        git={git}
        onToggleGit={onToggleGit}
      />
    );
  }

  const order = () => screen.getAllByRole("button", { name: /^Collapse / }).map((button) => button.getAttribute("aria-label"));

  it("floats pinned projects to the top and pins from the project menu", () => {
    const actions: string[] = [];
    const { rerender } = render(<PinHarness onProjectAction={(project, action) => actions.push(`${project.id}:${action}`)} />);
    expect(order()).toEqual(["Collapse Alpha", "Collapse Beta", "Collapse Gamma"]);
    fireEvent.click(screen.getByRole("button", { name: "Gamma menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Pin project" }));
    expect(actions).toEqual(["p3:pin"]);

    rerender(<PinHarness pinned={["p3"]} onProjectAction={(project, action) => actions.push(`${project.id}:${action}`)} />);
    expect(order()).toEqual(["Collapse Gamma", "Collapse Alpha", "Collapse Beta"]);
    fireEvent.click(screen.getByRole("button", { name: "Gamma menu" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Unpin project" }));
    expect(actions).toEqual(["p3:pin", "p3:unpin"]);
  });

  it("toggles Git mode from its tile and swaps in the Git panel", async () => {
    let toggled = 0;
    const { rerender } = render(<PinHarness onToggleGit={() => { toggled += 1; }} />);
    const tile = screen.getByRole("button", { name: "Git mode" });
    expect(tile).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(tile);
    expect(toggled).toBe(1);

    rerender(<PinHarness git={{ top: <div>repository switcher</div>, page: <div>changed files</div> }} />);
    expect(screen.getByRole("button", { name: "Git mode" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("repository switcher")).toBeInTheDocument();
    expect(await screen.findByText("changed files")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /New chat ⌘N/ })).toBeNull();
    expect(screen.getByRole("navigation", { name: "Git changes" })).toBeInTheDocument();
  });

  it("disables the Git tile when there is no project to open", () => {
    render(<PinHarness onToggleGit={null} />);
    expect(screen.getByRole("button", { name: "Git mode" })).toBeDisabled();
  });
});
