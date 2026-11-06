import { useState } from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectRecord } from "../types";
import { ProjectBar, type ProjectCommandActions } from "./ProjectBar";

afterEach(cleanup);

const projects: ProjectRecord[] = [
  { id: "p1", name: "TokenTrail", path: "/code/tokentrail", gitRoot: "/code/tokentrail", gitHasHead: true, runCommand: null, buildCommand: null, branch: "master", createdAt: "now" },
  { id: "p2", name: "scratch-nogit", path: "/tmp/scratch-nogit", gitRoot: null, gitHasHead: false, runCommand: null, buildCommand: null, branch: null, createdAt: "now" }
];

const noPins = new Set<string>();
const commandActions: ProjectCommandActions = {
  onSaveRunCommand: async () => {}, onRun: async () => {}, onStopRun: async () => {}, onShowRunOutput: () => {},
  onSaveBuildCommand: async () => {}, onBuild: async () => {}, onStopBuild: async () => {}, onShowBuildOutput: () => {}
};

function Harness({ initialProjectId = "p1", worktreeable = true }: { initialProjectId?: string | null; worktreeable?: boolean }) {
  const [projectId, setProjectId] = useState<string | null>(initialProjectId);
  const [useWorktree, setUseWorktree] = useState(false);
  const list = worktreeable ? projects : projects.map((project) => ({ ...project, gitHasHead: false }));
  return (
    <ProjectBar
      projects={list}
      projectId={projectId}
      useWorktree={useWorktree}
      pinned={noPins}
      onSelectProject={setProjectId}
      onSetPinned={() => undefined}
      onToggleWorktree={setUseWorktree}
      onAddProject={() => undefined}
      onListBranches={async () => ({ current: "master", branches: [] })}
      onCheckoutBranch={async () => undefined}
    />
  );
}

describe("ProjectBar", () => {
  it("shows the selected project name and branch", () => {
    render(<Harness />);
    expect(screen.getByRole("button", { name: /TokenTrail/ })).toBeInTheDocument();
    expect(screen.getByText("master")).toBeInTheDocument();
  });

  it("lists projects, No project, and Add project in the picker", () => {
    const selected: (string | null)[] = [];
    render(
      <ProjectBar
        projects={projects}
        projectId="p1"
        useWorktree={false}
        pinned={noPins}
        onSelectProject={(id) => selected.push(id)}
        onSetPinned={() => undefined}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
        onListBranches={async () => ({ current: "master", branches: [] })}
        onCheckoutBranch={async () => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /TokenTrail/ }));
    const dialog = screen.getByRole("dialog", { name: "Choose project" });
    expect(within(dialog).getByText("scratch-nogit")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: /Add project/ })).toBeInTheDocument();
    fireEvent.click(within(dialog).getByText("No project"));
    expect(selected).toEqual([null]);
  });

  it("switches the trigger to the chosen project", () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole("button", { name: /Switch project/ }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "Choose project" })).getByText("scratch-nogit"));
    expect(screen.getByRole("button", { name: /Switch project/ })).toHaveAccessibleName("Project: scratch-nogit. Switch project");
  });

  it("shows No project and hides the branch when no project is selected", () => {
    render(<Harness initialProjectId={null} />);
    expect(screen.getByRole("button", { name: /No project/ })).toBeInTheDocument();
    expect(screen.queryByText("master")).toBeNull();
    expect(screen.getByRole("button", { name: "Worktree" })).toBeDisabled();
  });

  it("toggles between Local and Worktree", () => {
    render(<Harness />);
    const worktree = screen.getByRole("button", { name: "Worktree" });
    fireEvent.click(worktree);
    expect(worktree).toHaveClass("selected");
    fireEvent.click(screen.getByRole("button", { name: "Local" }));
    expect(worktree).not.toHaveClass("selected");
  });

  it("disables Worktree for projects without commits", () => {
    render(<Harness worktreeable={false} />);
    expect(screen.getByRole("button", { name: "Worktree" })).toBeDisabled();
  });

  it("offers the selected project's Run and Build commands without a chat", async () => {
    const onBuild = vi.fn(async () => {});
    render(
      <ProjectBar
        projects={projects.map((project) => ({ ...project, runCommand: "pnpm dev", buildCommand: "pnpm build" }))}
        projectId="p1"
        useWorktree={false}
        pinned={noPins}
        commands={{ ...commandActions, onBuild }}
        onSelectProject={() => undefined}
        onSetPinned={() => undefined}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
        onListBranches={async () => ({ current: "master", branches: [] })}
        onCheckoutBranch={async () => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Build" }));
    await vi.waitFor(() => expect(onBuild).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    expect(screen.getByText("Saved for this project. Runs in its local folder.")).toBeInTheDocument();
    expect(screen.getByText("/code/tokentrail")).toBeInTheDocument();
  });

  it("does not offer project commands with no project", () => {
    render(
      <ProjectBar
        projects={projects}
        projectId={null}
        useWorktree={false}
        pinned={noPins}
        commands={commandActions}
        onSelectProject={() => undefined}
        onSetPinned={() => undefined}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
        onListBranches={async () => ({ current: "master", branches: [] })}
        onCheckoutBranch={async () => undefined}
      />
    );
    expect(screen.queryByRole("group", { name: "Project commands" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Run" })).not.toBeInTheDocument();
  });

  it("mounts GlassPlatter on pills and project/branch popovers only when glass is enabled", async () => {
    const { container, rerender } = render(
      <ProjectBar
        projects={projects}
        projectId="p1"
        useWorktree={false}
        pinned={noPins}
        glass
        commands={commandActions}
        onSelectProject={() => undefined}
        onSetPinned={() => undefined}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
        onListBranches={async () => ({ current: "master", branches: [{ name: "master", remote: false, worktree: null }] })}
        onCheckoutBranch={async () => undefined}
      />
    );
    expect(container.querySelectorAll(".glass-platter")).toHaveLength(5);
    fireEvent.click(screen.getByRole("button", { name: /Switch project/ }));
    expect(document.body.querySelector(".project-pop")).toHaveClass("glass");
    fireEvent.click(screen.getByRole("button", { name: /Switch branch/ }));
    expect(await screen.findByRole("dialog", { name: "Switch branch" })).toBeInTheDocument();
    expect(document.body.querySelector(".branch-pop")).toHaveClass("glass");
    fireEvent.click(screen.getByRole("button", { name: "Configure run command" }));
    expect(screen.getByRole("dialog", { name: "Project run command" })).toBeInTheDocument();
    expect(document.body.querySelector(".popover.glass")).toBeInTheDocument();

    rerender(
      <ProjectBar
        projects={projects}
        projectId="p1"
        useWorktree={false}
        pinned={noPins}
        commands={commandActions}
        onSelectProject={() => undefined}
        onSetPinned={() => undefined}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
        onListBranches={async () => ({ current: "master", branches: [] })}
        onCheckoutBranch={async () => undefined}
      />
    );
    expect(container.querySelectorAll(".glass-platter")).toHaveLength(0);
    expect(document.body.querySelector(".branch-pop")).not.toHaveClass("glass");
  });
});
