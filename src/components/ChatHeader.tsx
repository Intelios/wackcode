import { useId, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import type { GitBranches, GitCheckoutKind, ProjectRecord, RunInfo, TaskRecord } from "../types";
import { RunButton } from "./RunButton";
import { BranchPicker } from "./BranchPicker";
import { Icon, type IconName } from "./Icons";
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
  gamesOpen: boolean;
  onToggleGames: () => void;
  onRename: (name: string) => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  /** Bumps when the title model names the chat: the title swipes to the new name and glints. */
  titlePulse?: number;
  onListBranches: () => Promise<GitBranches>;
  onCheckoutBranch: (name: string, kind: GitCheckoutKind) => Promise<void>;
}

export function ChatHeader({ task, project, run, onSaveRunCommand, onRun, onStopRun, onShowRunOutput, git, onListBranches, onCheckoutBranch, changesCount, changesOpen, browserOpen, onToggleChanges, onToggleBrowser, terminalOpen, onToggleTerminal, gamesOpen, onToggleGames, onRename, onTaskAction, titlePulse = 0 }: ChatHeaderProps) {
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
  // A Chat mode chat has a scratchpad and a browser, and none of the project chrome.
  const chat = task.kind === "chat";
  const reduce = useReducedMotion() ?? false;
  const pillId = useId();
  const panels: { label: string; icon: IconName; keys?: string; count?: number; open: boolean; onToggle: () => void }[] = [
    { label: "Browser", icon: "browser", open: browserOpen, onToggle: onToggleBrowser },
    ...(chat ? [] : [
      { label: "Changes", icon: "panel" as const, keys: "⌘⇧C", count: changesCount, open: changesOpen, onToggle: onToggleChanges },
      { label: "Terminal", icon: "terminal" as const, keys: "⌘⇧T", open: terminalOpen, onToggle: onToggleTerminal },
      { label: "Games", icon: "gamepad" as const, open: gamesOpen, onToggle: onToggleGames }
    ])
  ];

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
          {chat ? (
            <button type="button" title="Open this chat's scratchpad folder in Finder" aria-label="Reveal scratchpad" onClick={() => onTaskAction(task, "reveal")}>
              <Icon name="folder" /> Scratchpad
            </button>
          ) : (
            <button type="button" title={task.workspacePath} onClick={() => onTaskAction(task, "reveal")}>
              <Icon name="folder" /> {task.projectId === null ? "No project" : task.workspacePath.split("/").filter(Boolean).slice(-2).join("/") || task.workspacePath}
            </button>
          )}
          {git && !chat && <BranchPicker key={task.id} branch={git.branch} variant="meta" onLoad={onListBranches} onCheckout={onCheckoutBranch} />}
        </div>
      </div>
      <div className="header-actions">
        {project && !task.archived && <RunButton key={task.id} project={project} workspacePath={task.workspacePath} run={run}
          onSave={onSaveRunCommand} onRun={onRun} onStop={onStopRun} onShowOutput={onShowRunOutput} />}
        {/* The side panel shows one view at a time, so its triggers are one control: a track whose
            fill glides to whichever view is open, and rests nowhere while the panel is closed. */}
        <div className="panel-switch" role="group" aria-label="Side panel">
          {panels.map((panel) => (
            <Tooltip key={panel.label} label={panel.keys ? <>{panel.label} <kbd>{panel.keys}</kbd></> : panel.label} side="bottom">
              <button type="button" className={`panel-button panel-switch-tab ${panel.open ? "active" : ""}`} onClick={panel.onToggle}
                aria-label={panel.count ? `${panel.label} (${panel.count})` : panel.label} aria-pressed={panel.open}>
                {panel.open && (
                  <motion.span className="panel-switch-pill" aria-hidden="true" layoutId={reduce ? undefined : pillId}
                    style={{ borderRadius: 999 }} transition={{ type: "spring", stiffness: 480, damping: 36 }} />
                )}
                <Icon name={panel.icon} />{panel.count ? <em className="changes-badge" aria-hidden="true">{panel.count}</em> : null}
              </button>
            </Tooltip>
          ))}
        </div>
        <MenuButton label="Chat menu" items={menu} />
      </div>
    </header>
  );
}
