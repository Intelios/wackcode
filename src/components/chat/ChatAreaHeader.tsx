/**
 * The Chat area's header: the duck (which mirrors what it's doing), the chat's title with the
 * auto-title glint, the browser toggle and the chat menu. No project, branch, run or Git
 * chrome; the scratchpad lives in the menu and in the replies that write to it.
 */
import { useState } from "react";
import type { TaskRecord } from "../../types";
import { Icon } from "../Icons";
import { MenuButton } from "../ui/MenuButton";
import { Tooltip } from "../ui/Tooltip";
import { TextSwap } from "../TextSwap";
import { chatMenu } from "../chat-menu";
import { useContextMenu } from "../ui/ContextMenu";
import type { TaskAction } from "../Sidebar";
import { ChatAvatar } from "./ChatBubbles";

interface ChatAreaHeaderProps {
  task: TaskRecord;
  /** The agent is replying; `thinking` while it reasons. */
  live: boolean;
  thinking: boolean;
  browserOpen: boolean;
  onToggleBrowser: () => void;
  onRename: (name: string) => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  titlePulse?: number;
}

export function ChatAreaHeader({ task, live, thinking, browserOpen, onToggleBrowser, onRename, onTaskAction, titlePulse = 0 }: ChatAreaHeaderProps) {
  const contextMenu = useContextMenu();
  const [renaming, setRenaming] = useState(false);
  const [value, setValue] = useState(task.name);
  const menu = chatMenu(task, undefined, () => { setValue(task.name); setRenaming(true); }, onTaskAction);

  function commit() {
    const name = value.trim();
    setRenaming(false);
    if (name && name !== task.name) onRename(name);
    else setValue(task.name);
  }

  return (
    <header className="chat-header" data-tauri-drag-region>
      <ChatAvatar live={live} thinking={thinking} className="chat-header-avatar" />
      <div className="chat-header-title" onContextMenu={(event) => contextMenu(event, menu, "Chat menu")}>
        {renaming ? (
          <input className="title-input" autoFocus aria-label="Chat name" value={value} onChange={(event) => setValue(event.target.value)} onBlur={commit}
            onKeyDown={(event) => {
              if (event.key === "Enter") commit();
              if (event.key === "Escape") { setValue(task.name); setRenaming(false); }
            }} />
        ) : (
          <h1 onDoubleClick={() => { setValue(task.name); setRenaming(true); }} title="Double-click to rename">
            <TextSwap key={task.id} text={task.name} swapKey={titlePulse} variant="title" swappedClassName="title-glint" />
          </h1>
        )}
        <span className="chat-header-status" aria-live="polite">{task.archived ? "Archived" : live ? (thinking ? "pondering…" : "typing…") : "here for you"}</span>
      </div>
      <span className="chat-header-spacer" data-tauri-drag-region />
      <Tooltip label="Browser" side="bottom">
        <button type="button" className={`panel-button ${browserOpen ? "active" : ""}`} onClick={onToggleBrowser} aria-label="Browser" aria-pressed={browserOpen}>
          <Icon name="browser" />
        </button>
      </Tooltip>
      <MenuButton label="Chat menu" items={menu} />
    </header>
  );
}
