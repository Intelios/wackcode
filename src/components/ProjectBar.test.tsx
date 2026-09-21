import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ProjectRecord } from "../types";
import { ProjectBar } from "./ProjectBar";

afterEach(cleanup);

const projects: ProjectRecord[] = [
  { id: "p1", name: "TokenTrail", path: "/code/tokentrail", gitRoot: "/code/tokentrail", gitHasHead: true, branch: "master", createdAt: "now" },
  { id: "p2", name: "scratch-nogit", path: "/tmp/scratch-nogit", gitRoot: null, gitHasHead: false, branch: null, createdAt: "now" }
];

function Harness({ initialProjectId = "p1", worktreeable = true }: { initialProjectId?: string | null; worktreeable?: boolean }) {
  const [projectId, setProjectId] = useState<string | null>(initialProjectId);
  const [useWorktree, setUseWorktree] = useState(false);
  const list = worktreeable ? projects : projects.map((project) => ({ ...project, gitHasHead: false }));
  return (
    <ProjectBar
      projects={list}
      projectId={projectId}
      useWorktree={useWorktree}
      onSelectProject={setProjectId}
      onToggleWorktree={setUseWorktree}
      onAddProject={() => undefined}
    />
  );
}

describe("ProjectBar", () => {
  it("shows the selected project name and branch", () => {
    render(<Harness />);
    expect(screen.getByRole("button", { name: /TokenTrail/ })).toBeInTheDocument();
    expect(screen.getByText("master")).toBeInTheDocument();
  });

  it("lists projects, No project, and Add folder in the picker", () => {
    const selected: (string | null)[] = [];
    render(
      <ProjectBar
        projects={projects}
        projectId="p1"
        useWorktree={false}
        onSelectProject={(id) => selected.push(id)}
        onToggleWorktree={() => undefined}
        onAddProject={() => undefined}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /TokenTrail/ }));
    fireEvent.click(screen.getByText("No project"));
    expect(selected).toEqual([null]);
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
});
