import { memo } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { GitBranches, GitCheckoutKind, ProjectRecord, RunInfo } from "../types";
import { BranchPicker } from "./BranchPicker";
import { ProjectPicker } from "./ProjectPicker";
import { RunButton } from "./RunButton";
import { GlassPlatter } from "./ui/GlassPlatter";
import { Tooltip } from "./ui/Tooltip";

export interface ProjectCommandActions {
  run?: RunInfo;
  build?: RunInfo;
  onSaveRunCommand: (command: string) => Promise<void>;
  onRun: () => Promise<void>;
  onStopRun: () => Promise<void>;
  onShowRunOutput: () => void;
  onSaveBuildCommand: (command: string) => Promise<void>;
  onBuild: () => Promise<void>;
  onStopBuild: () => Promise<void>;
  onShowBuildOutput: () => void;
}

interface ProjectBarProps {
  projects: ProjectRecord[];
  projectId: string | null;
  useWorktree: boolean;
  /** The sidebar's own pin set: the picker lists pinned projects first and can pin or unpin. */
  pinned: ReadonlySet<string>;
  glass?: boolean;
  onSelectProject: (projectId: string | null) => void;
  onSetPinned: (projectId: string, pinned: boolean) => void;
  onToggleWorktree: (value: boolean) => void;
  onAddProject: () => void;
  onListBranches: (projectId: string) => Promise<GitBranches>;
  onCheckoutBranch: (projectId: string, name: string, kind: GitCheckoutKind) => Promise<void>;
  /** Project-local Run and Build on the welcome screen; no chat is created. */
  commands?: ProjectCommandActions;
}

export const ProjectBar = memo(function ProjectBar({ projects, projectId, useWorktree, pinned, glass = false, onSelectProject, onSetPinned, onToggleWorktree, onAddProject, onListBranches, onCheckoutBranch, commands }: ProjectBarProps) {
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
        glass={glass}
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
          glass={glass}
          onLoad={() => onListBranches(project.id)}
          onCheckout={(name, kind) => onCheckoutBranch(project.id, name, kind)}
        />
      )}
      <LayoutGroup id="workspace-location">
        <div className="segmented" role="group" aria-label="Workspace location">
          {glass && <GlassPlatter />}
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
      {project && commands && (
        <div className="project-bar-commands" role="group" aria-label="Project commands">
          <RunButton key={`build:${project.id}`} kind="build" project={project} workspacePath={project.path} run={commands.build}
            onSave={commands.onSaveBuildCommand} onRun={commands.onBuild} onStop={commands.onStopBuild} onShowOutput={commands.onShowBuildOutput}
            location="project" popoverSide="top" glass={glass} />
          <RunButton key={`run:${project.id}`} project={project} workspacePath={project.path} run={commands.run}
            onSave={commands.onSaveRunCommand} onRun={commands.onRun} onStop={commands.onStopRun} onShowOutput={commands.onShowRunOutput}
            location="project" popoverSide="top" glass={glass} />
        </div>
      )}
    </div>
  );
});
