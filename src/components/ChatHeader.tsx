import { useState } from "react";
import type { GitBranches, GitCheckoutKind, ProjectRecord, TaskRecord, TerminalExit } from "../types";
import { BranchPicker } from "./BranchPicker";
import { Icon } from "./Icons";
import { MenuButton } from "./ui/MenuButton";
import type { MenuEntry } from "./ui/Menu";
import type { TaskAction } from "./Sidebar";
import { canFork } from "../tree-utils";

interface ChatHeaderProps {
  task: TaskRecord;
  project?: ProjectRecord;
  /** The chat's checkout when it is a Git repository; `branch` is null on a detached HEAD. */
  git?: { branch: string | null };
  changesCount?: number;
  changesOpen: boolean;
  browserOpen: boolean;
  onToggleChanges: () => void;
  onToggleBrowser: () => void;
  /** The chat's terminal session, if one is running — drives the button's caret hint. */
  terminal?: { busy: boolean; exit: TerminalExit | null };
  terminalOpen: boolean;
  onToggleTerminal: () => void;
  onRename: (name: string) => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  onListBranches: () => Promise<GitBranches>;
  onCheckoutBranch: (name: string, kind: GitCheckoutKind) => Promise<void>;
}

export function ChatHeader({ task, project, git, onListBranches, onCheckoutBranch, changesCount, changesOpen, browserOpen, onToggleChanges, onToggleBrowser, terminal, terminalOpen, onToggleTerminal, onRename, onTaskAction }: ChatHeaderProps) {
  const [renaming, setRenaming] = useState(false);
  const [value, setValue] = useState(task.name);

  function commit() {
    const name = value.trim();
    setRenaming(false);
    if (name && name !== task.name) onRename(name);
    else setValue(task.name);
  }

  const menu: MenuEntry[] = [
    { label: "Rename", icon: <Icon name="pencil" />, onSelect: () => { setValue(task.name); setRenaming(true); } },
    {
      label: "Move to worktree",
      icon: <Icon name="branch" />,
      disabled: Boolean(task.usesWorktree || task.sessionFile || !project?.gitHasHead),
      onSelect: () => onTaskAction(task, "worktree")
    },
    { label: "Fork chat", icon: <Icon name="branch" />, disabled: !canFork(task), onSelect: () => onTaskAction(task, "fork") },
    "separator",
    { label: "Reveal in Finder", icon: <Icon name="folder" />, onSelect: () => onTaskAction(task, "reveal") },
    { label: "Copy path", icon: <Icon name="copy" />, onSelect: () => onTaskAction(task, "copy") },
    "separator",
    { label: task.archived ? "Unarchive" : "Archive", icon: <Icon name={task.archived ? "unarchive" : "archive"} />, onSelect: () => onTaskAction(task, task.archived ? "unarchive" : "archive") },
    { label: "Delete", icon: <Icon name="trash" />, danger: true, onSelect: () => onTaskAction(task, "delete") }
  ];

  return (
    <header className="workspace-header">
      <div className="task-title">
        {renaming ? (
          <input
            className="title-input"
            autoFocus
            value={value}
            onChange={(event) => setValue(event.target.value)}
            onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === "Enter") commit();
              if (event.key === "Escape") { setValue(task.name); setRenaming(false); }
            }}
          />
        ) : (
          <h1 onDoubleClick={() => { setValue(task.name); setRenaming(true); }} title="Double-click to rename">{task.name}</h1>
        )}
        <div className="workspace-meta">
          <button type="button" title={task.workspacePath} onClick={() => onTaskAction(task, "reveal")}>
            <Icon name="folder" /> {task.projectId === null ? "No project" : task.workspacePath.split("/").filter(Boolean).slice(-2).join("/") || task.workspacePath}
          </button>
          {git && <BranchPicker key={task.id} branch={git.branch} variant="meta" onLoad={onListBranches} onCheckout={onCheckoutBranch} />}
        </div>
      </div>
      <div className="header-actions">
        <button type="button" className={`panel-button ${browserOpen ? "active" : ""}`} onClick={onToggleBrowser}>
          <Icon name="browser" /> Browser
        </button>
        <button type="button" className={`panel-button ${changesOpen ? "active" : ""}`} onClick={onToggleChanges}>
          <Icon name="panel" /> Changes{changesCount ? <em className="changes-badge">{changesCount}</em> : null}
        </button>
        <button
          type="button"
          className={`panel-button ${terminalOpen ? "active" : ""}`}
          onClick={onToggleTerminal}
          title="Terminal — ⌘⇧T"
        >
          <Icon name="terminal" /> Terminal
          {/* A blinking caret marks a shell running out of sight; it pulses while it works and
              dims when the shell has exited. */}
          {terminal && !terminalOpen && (
            <em
              className={`terminal-caret ${terminal.exit ? "exited" : terminal.busy ? "busy" : ""}`}
              aria-hidden="true"
            />
          )}
        </button>
        <MenuButton label="Chat menu" items={menu} />
      </div>
    </header>
  );
}
