import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { GitBranches, GitCheckoutKind, ProjectRecord } from "../types";
import { BranchPicker } from "./BranchPicker";
import { ProjectPicker } from "./ProjectPicker";
import { Tooltip } from "./ui/Tooltip";

interface ProjectBarProps {
  projects: ProjectRecord[];
  projectId: string | null;
  useWorktree: boolean;
  /** The sidebar's own pin set: the picker lists pinned projects first and can pin or unpin. */
  pinned: ReadonlySet<string>;
  onSelectProject: (projectId: string | null) => void;
  onSetPinned: (projectId: string, pinned: boolean) => void;
  onToggleWorktree: (value: boolean) => void;
  onAddProject: () => void;
  onListBranches: (projectId: string) => Promise<GitBranches>;
  onCheckoutBranch: (projectId: string, name: string, kind: GitCheckoutKind) => Promise<void>;
}

export function ProjectBar({ projects, projectId, useWorktree, pinned, onSelectProject, onSetPinned, onToggleWorktree, onAddProject, onListBranches, onCheckoutBranch }: ProjectBarProps) {
  const reduce = useReducedMotion();
  const project = projects.find((item) => item.id === projectId);
  const canWorktree = Boolean(project?.gitHasHead);
  const thumb = (
    <motion.span
      layoutId="workspace-thumb"
      className="segmented-thumb"
      transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 520, damping: 36 }}
    />
  );

  return (
    <div className="project-bar">
      <ProjectPicker
        projects={projects}
        projectId={projectId}
        pinned={pinned}
        onSelect={onSelectProject}
        onSetPinned={onSetPinned}
        onAddProject={onAddProject}
      />
      {project?.gitRoot && (
        // A worktree chat starts from the branch checked out here, so switching applies to both modes.
        <BranchPicker
          key={project.id}
          branch={project.branch}
          variant="pill"
          side="top"
          onLoad={() => onListBranches(project.id)}
          onCheckout={(name, kind) => onCheckoutBranch(project.id, name, kind)}
        />
      )}
      <LayoutGroup id="workspace-location">
        <div className="segmented" role="group" aria-label="Workspace location">
          <button
            type="button"
            className={`segmented-option ${!useWorktree ? "selected" : ""}`}
            onClick={() => onToggleWorktree(false)}
          >
            {!useWorktree && thumb}
            <span className="segmented-label">Local</span>
          </button>
          <Tooltip label={canWorktree ? "Run in an isolated git worktree" : "Worktrees need a Git project with at least one commit"}>
            <button
              type="button"
              className={`segmented-option ${useWorktree ? "selected" : ""}`}
              disabled={!canWorktree}
              onClick={() => onToggleWorktree(true)}
            >
              {useWorktree && thumb}
              <span className="segmented-label">Worktree</span>
            </button>
          </Tooltip>
        </div>
      </LayoutGroup>
    </div>
  );
}
