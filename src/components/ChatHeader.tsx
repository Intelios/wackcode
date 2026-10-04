import { useState } from "react";
import type { GitBranches, GitCheckoutKind, ProjectRecord, RunInfo, TaskRecord } from "../types";
import { RunButton } from "./RunButton";
import { BranchPicker } from "./BranchPicker";
import { Icon } from "./Icons";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";
import { TextSwap } from "./TextSwap";
import { chatMenu } from "./chat-menu";
import { useContextMenu } from "./ui/ContextMenu";
import type { TaskAction } from "./Sidebar";


interface ChatHeaderProps {
  task: TaskRecord;
  run?: RunInfo;
  onSaveRunCommand: (command: string) => Promise<void>;
  onRun: () => Promise<void>;
  onStopRun: () => Promise<void>;
  onShowRunOutput: () => void;
  project?: ProjectRecord;
  /** The chat's checkout when it is a Git repository; `branch` is null on a detached HEAD. */
  git?: { branch: string | null };
  changesCount?: number;
  changesOpen: boolean;
  browserOpen: boolean;
  onToggleChanges: () => void;
  onToggleBrowser: () => void;
  terminalOpen: boolean;
  onToggleTerminal: () => void;
  onRename: (name: string) => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  /** Bumps when the title model names the chat: the title swipes to the new name and glints. */
  titlePulse?: number;
  onListBranches: () => Promise<GitBranches>;
  onCheckoutBranch: (name: string, kind: GitCheckoutKind) => Promise<void>;
}

export function ChatHeader({ task, project, run, onSaveRunCommand, onRun, onStopRun, onShowRunOutput, git, onListBranches, onCheckoutBranch, changesCount, changesOpen, browserOpen, onToggleChanges, onToggleBrowser, terminalOpen, onToggleTerminal, onRename, onTaskAction, titlePulse = 0 }: ChatHeaderProps) {
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
          <h1 onDoubleClick={() => { setValue(task.name); setRenaming(true); }} title="Double-click to rename">
            {/* Keyed by task.id so switching chats remounts it (initial={false} → no swap);
                only the model's auto-title bumps the pulse and gets the swipe + glint. */}
            <TextSwap key={task.id} text={task.name} swapKey={titlePulse} variant="title" swappedClassName="title-glint" />
          </h1>
        )}
        <div className="workspace-meta">
          <button type="button" title={task.workspacePath} onClick={() => onTaskAction(task, "reveal")}>
            <Icon name="folder" /> {task.projectId === null ? "No project" : task.workspacePath.split("/").filter(Boolean).slice(-2).join("/") || task.workspacePath}
          </button>
          {git && <BranchPicker key={task.id} branch={git.branch} variant="meta" onLoad={onListBranches} onCheckout={onCheckoutBranch} />}
        </div>
      </div>
      <div className="header-actions">
        {project && !task.archived && <RunButton key={task.id} project={project} workspacePath={task.workspacePath} run={run}
          onSave={onSaveRunCommand} onRun={onRun} onStop={onStopRun} onShowOutput={onShowRunOutput} />}
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
          </button>
        </Tooltip>
        <MenuButton label="Chat menu" items={menu} />
      </div>
    </header>
  );
}
