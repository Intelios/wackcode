import { useState } from "react";
import type { GitBranches, GitCheckoutKind, ProjectRecord, TaskRecord, TerminalExit } from "../types";
import { BranchPicker } from "./BranchPicker";
import { Icon } from "./Icons";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";
import { chatMenu } from "./chat-menu";
import { useContextMenu } from "./ui/ContextMenu";
import type { TaskAction } from "./Sidebar";


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
  const contextMenu = useContextMenu();
  const [renaming, setRenaming] = useState(false);
  const [value, setValue] = useState(task.name);

  function commit() {
    const name = value.trim();
    setRenaming(false);
    if (name && name !== task.name) onRename(name);
    else setValue(task.name);
  }

  const menu = chatMenu(task, project, () => { setValue(task.name); setRenaming(true); }, onTaskAction);

  return (
    <header className="workspace-header">
      <div className="task-title" onContextMenu={(event) => contextMenu(event, menu, "Chat menu")}>
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
        <Tooltip label="Browser" side="bottom">
          <button type="button" className={`panel-button ${browserOpen ? "active" : ""}`} onClick={onToggleBrowser} aria-label="Browser" aria-pressed={browserOpen}>
            <Icon name="browser" />
          </button>
        </Tooltip>
        <Tooltip label={<>Changes <kbd>⌘⇧C</kbd></>} side="bottom">
          <button type="button" className={`panel-button ${changesOpen ? "active" : ""}`} onClick={onToggleChanges} aria-label={changesCount ? `Changes (${changesCount})` : "Changes"} aria-pressed={changesOpen}>
            <Icon name="panel" />{changesCount ? <em className="changes-badge" aria-hidden="true">{changesCount}</em> : null}
          </button>
        </Tooltip>
        <Tooltip label={<>Terminal <kbd>⌘⇧T</kbd></>} side="bottom">
          <button
            type="button"
            className={`panel-button ${terminalOpen ? "active" : ""}`}
            onClick={onToggleTerminal}
            aria-label="Terminal"
            aria-pressed={terminalOpen}
          >
            <Icon name="terminal" />
            {/* A blinking caret marks a shell running out of sight; it pulses while it works and
                dims when the shell has exited. */}
            {terminal && !terminalOpen && (
              <em
                className={`terminal-caret ${terminal.exit ? "exited" : terminal.busy ? "busy" : ""}`}
                aria-hidden="true"
              />
            )}
          </button>
        </Tooltip>
        <MenuButton label="Chat menu" items={menu} />
      </div>
    </header>
  );
}
