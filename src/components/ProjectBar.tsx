import { useRef, useState } from "react";
import type { ProjectRecord } from "../types";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

interface ProjectBarProps {
  projects: ProjectRecord[];
  projectId: string | null;
  useWorktree: boolean;
  onSelectProject: (projectId: string | null) => void;
  onToggleWorktree: (value: boolean) => void;
  onAddProject: () => void;
}

export function ProjectBar({ projects, projectId, useWorktree, onSelectProject, onToggleWorktree, onAddProject }: ProjectBarProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const project = projects.find((item) => item.id === projectId);
  const canWorktree = Boolean(project?.gitHasHead);

  return (
    <div className="project-bar">
      <button
        ref={triggerRef}
        type="button"
        className="model-pill"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="folder" />
        <span className="model-pill-name">{project?.name ?? "No project"}</span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      {project?.branch && (
        <span className="project-bar-branch" title={`${project.branch} in ${project.path}`}>
          <Icon name="branch" /> {project.branch}
        </span>
      )}
      <div className="segmented" role="group" aria-label="Workspace location">
        <button
          type="button"
          className={`segmented-option ${!useWorktree ? "selected" : ""}`}
          onClick={() => onToggleWorktree(false)}
        >Local</button>
        <Tooltip label={canWorktree ? "Run in an isolated git worktree" : "Worktrees need a Git project with at least one commit"}>
          <button
            type="button"
            className={`segmented-option ${useWorktree ? "selected" : ""}`}
            disabled={!canWorktree}
            onClick={() => onToggleWorktree(true)}
          >Worktree</button>
        </Tooltip>
      </div>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="top" align="start" className="model-picker-pop">
        <div className="picker">
          <div className="picker-group">
            <div className="picker-heading">Project</div>
            {projects.map((item) => (
              <button
                type="button"
                key={item.id}
                className={`picker-item ${item.id === projectId ? "selected" : ""}`}
                title={item.path}
                onClick={() => { onSelectProject(item.id); setOpen(false); }}
              >
                <span>{item.name}</span>
                {item.id === projectId && <Icon name="check" />}
              </button>
            ))}
            <button
              type="button"
              className={`picker-item ${projectId === null ? "selected" : ""}`}
              onClick={() => { onSelectProject(null); setOpen(false); }}
            >
              <span>No project</span>
              {projectId === null && <Icon name="check" />}
            </button>
            <button
              type="button"
              className="picker-item"
              onClick={() => { onAddProject(); setOpen(false); }}
            >
              <span>Add folder…</span>
            </button>
          </div>
        </div>
      </Popover>
    </div>
  );
}
