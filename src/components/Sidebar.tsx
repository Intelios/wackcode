import { useState } from "react";
import type { ReactNode } from "react";
import type { ProjectRecord, TaskRecord } from "../types";
import { canFork } from "../tree-utils";
import { Icon } from "./Icons";
import { MenuButton } from "./ui/MenuButton";
import type { MenuEntry } from "./ui/Menu";
import { Tooltip } from "./ui/Tooltip";

export type TaskAction = "rename" | "worktree" | "fork" | "reveal" | "copy" | "archive" | "unarchive" | "delete";
export type ProjectAction = "reveal" | "remove";

/** Collapse-key for the "No project" group, which has no ProjectRecord id. */
export const NO_PROJECT_KEY = "__no_project__";

interface SidebarProps {
  projects: ProjectRecord[];
  tasks: TaskRecord[];
  selectedTaskId?: string;
  showArchived: boolean;
  /** Task IDs that have a pending extension dialog waiting for a response. */
  pendingDialogTaskIds: ReadonlySet<string>;
  /** Group keys (project ids or NO_PROJECT_KEY) whose chats are hidden. */
  collapsedProjectIds: ReadonlySet<string>;
  onSelectTask: (id: string) => void;
  onNewChat: (project: ProjectRecord | null) => void;
  onNewDraft: () => void;
  onAddProject: () => void;
  onToggleArchived: () => void;
  onToggleProjectCollapsed: (key: string) => void;
  onOpenSettings: () => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  onProjectAction: (project: ProjectRecord, action: ProjectAction) => void;
  onRenameTask: (taskId: string, name: string) => void;
}

export function Sidebar({ projects, tasks, selectedTaskId, showArchived, pendingDialogTaskIds, collapsedProjectIds, onSelectTask, onNewChat, onNewDraft, onAddProject, onToggleArchived, onToggleProjectCollapsed, onOpenSettings, onTaskAction, onProjectAction, onRenameTask }: SidebarProps) {
  const [renamingId, setRenamingId] = useState<string>();
  const [renameValue, setRenameValue] = useState("");
  const hasArchived = tasks.some((task) => task.archived);
  const looseTasks = tasks.filter((task) => task.projectId === null && (!task.archived || showArchived));

  function startRename(task: TaskRecord) {
    setRenamingId(task.id);
    setRenameValue(task.name);
  }

  function commitRename(taskId: string) {
    const name = renameValue.trim();
    setRenamingId(undefined);
    if (name) onRenameTask(taskId, name);
  }

  function taskMenu(task: TaskRecord, project?: ProjectRecord): MenuEntry[] {
    const canWorktree = Boolean(!task.usesWorktree && !task.sessionFile && project?.gitHasHead);
    return [
      { label: "Rename", icon: <Icon name="pencil" />, onSelect: () => startRename(task) },
      { label: "Move to worktree", icon: <Icon name="branch" />, disabled: !canWorktree, onSelect: () => onTaskAction(task, "worktree") },
      { label: "Fork chat", icon: <Icon name="branch" />, disabled: !canFork(task), onSelect: () => onTaskAction(task, "fork") },
      "separator",
      { label: "Reveal in Finder", icon: <Icon name="folder" />, onSelect: () => onTaskAction(task, "reveal") },
      { label: "Copy path", icon: <Icon name="copy" />, onSelect: () => onTaskAction(task, "copy") },
      "separator",
      task.archived
        ? { label: "Unarchive", icon: <Icon name="archive" />, onSelect: () => onTaskAction(task, "unarchive") }
        : { label: "Archive", icon: <Icon name="archive" />, onSelect: () => onTaskAction(task, "archive") },
      { label: "Delete", icon: <Icon name="trash" />, danger: true, onSelect: () => onTaskAction(task, "delete") }
    ];
  }

  function renderTask(task: TaskRecord, project?: ProjectRecord) {
    return renamingId === task.id ? (
      <div className="task-item renaming" key={task.id}>
        <input
          autoFocus
          value={renameValue}
          onChange={(event) => setRenameValue(event.target.value)}
          onBlur={() => commitRename(task.id)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commitRename(task.id);
            if (event.key === "Escape") setRenamingId(undefined);
          }}
        />
      </div>
    ) : (
      <div
        key={task.id}
        role="button"
        tabIndex={0}
        className={`task-item ${selectedTaskId === task.id ? "active" : ""} ${task.archived ? "archived" : ""}`}
        onClick={() => onSelectTask(task.id)}
        onKeyDown={(event) => { if (event.key === "Enter") onSelectTask(task.id); }}
        onDoubleClick={() => startRename(task)}
      >
        <span className={`task-status ${task.lastError ? "error" : task.status}`} />
        <span className="task-name">{task.name}</span>
        {pendingDialogTaskIds.has(task.id) && <span className="sidebar-question-dot" title="Waiting for your answer" />}
        {task.mode === "plan" && <span className="task-mode-chip">Plan</span>}
        {task.usesWorktree && <Icon name="branch" className="task-branch-icon" />}
        <span className="task-actions" onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
          <MenuButton className="row-menu" label={`${task.name} menu`} items={() => taskMenu(task, project)} />
          <Tooltip label="Delete chat">
            <button type="button" className="ghost-button row-menu danger" onClick={() => onTaskAction(task, "delete")} aria-label={`Delete ${task.name}`}>
              <Icon name="trash" />
            </button>
          </Tooltip>
        </span>
      </div>
    );
  }

  function renderHeading({ name, groupKey, plusLabel, title, groupTasks, onPlus, menu }: {
    name: string;
    groupKey: string;
    plusLabel: string;
    title?: string;
    groupTasks: TaskRecord[];
    onPlus: () => void;
    menu?: ReactNode;
  }) {
    const collapsed = collapsedProjectIds.has(groupKey);
    return (
      <div className="project-heading" title={title} onClick={() => onToggleProjectCollapsed(groupKey)}>
        <button
          type="button"
          className="ghost-button project-chevron"
          aria-expanded={!collapsed}
          aria-label={collapsed ? `Expand ${name}` : `Collapse ${name}`}
          onClick={(event) => { event.stopPropagation(); onToggleProjectCollapsed(groupKey); }}
        >
          <Icon name="chevron" />
        </button>
        <span className="project-name">{name}</span>
        {collapsed && groupTasks.length > 0 && <span className="project-count">{groupTasks.length}</span>}
        {collapsed && groupTasks.some((task) => pendingDialogTaskIds.has(task.id)) && (
          <span className="sidebar-question-dot" title="A chat in this group is waiting for your answer" />
        )}
        <span className="project-actions" onClick={(event) => event.stopPropagation()}>
          {menu}
          <Tooltip label={plusLabel}>
            <button type="button" className="ghost-button" onClick={onPlus} aria-label={plusLabel}>
              <Icon name="plus" />
            </button>
          </Tooltip>
        </span>
      </div>
    );
  }

  return (
    <aside className="sidebar">
      <div className="titlebar-drag" data-tauri-drag-region />
      <div className="sidebar-top">
        <button type="button" className="sidebar-action" onClick={onNewDraft}>
          <Icon name="plus" /> New chat <kbd>⌘N</kbd>
        </button>
      </div>
      <nav className="project-list" aria-label="Projects and chats">
        {projects.length === 0 && looseTasks.length === 0 && (
          <div className="sidebar-empty">Start a new chat — with a project folder or without one.</div>
        )}
        {projects.map((project) => {
          const projectTasks = tasks.filter((task) => task.projectId === project.id && (!task.archived || showArchived));
          return (
            <section className={`project-group ${collapsedProjectIds.has(project.id) ? "" : "open"}`} key={project.id}>
              {renderHeading({
                name: project.name,
                groupKey: project.id,
                plusLabel: `New chat in ${project.name}`,
                title: project.path,
                groupTasks: projectTasks,
                onPlus: () => onNewChat(project),
                menu: (
                  <MenuButton
                    className="ghost-button"
                    label={`${project.name} menu`}
                    items={() => [
                      { label: "New chat", icon: <Icon name="plus" />, onSelect: () => onNewChat(project) },
                      "separator",
                      { label: "Reveal in Finder", icon: <Icon name="folder" />, onSelect: () => onProjectAction(project, "reveal") },
                      { label: "Remove project", icon: <Icon name="trash" />, danger: true, onSelect: () => onProjectAction(project, "remove") }
                    ]}
                  />
                )
              })}
              {!collapsedProjectIds.has(project.id) && projectTasks.map((task) => renderTask(task, project))}
            </section>
          );
        })}
        {looseTasks.length > 0 && (
          <section className={`project-group ${collapsedProjectIds.has(NO_PROJECT_KEY) ? "" : "open"}`}>
            {renderHeading({
              name: "No project",
              groupKey: NO_PROJECT_KEY,
              plusLabel: "New chat with no project",
              groupTasks: looseTasks,
              onPlus: () => onNewChat(null)
            })}
            {!collapsedProjectIds.has(NO_PROJECT_KEY) && looseTasks.map((task) => renderTask(task))}
          </section>
        )}
      </nav>
      <div className="sidebar-footer">
        <button type="button" className="sidebar-action" onClick={onAddProject}>
          <Icon name="folder" /> Add project <kbd>⌘O</kbd>
        </button>
        {hasArchived && (
          <button type="button" className="sidebar-action" onClick={onToggleArchived}>
            <Icon name="archive" /> {showArchived ? "Hide archived" : "Show archived"}
          </button>
        )}
        <button type="button" className="sidebar-action" onClick={onOpenSettings}>
          <Icon name="settings" /> Settings <kbd>⌘,</kbd>
        </button>
      </div>
    </aside>
  );
}
