import { memo, useEffect, useRef, useState, type PointerEvent } from "react";
import { motion, useReducedMotion } from "motion/react";
import { Icon } from "./Icons";
import { OrbitSpinner } from "./OrbitSpinner";
import { TextSwap } from "./TextSwap";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";
import { useContextMenu } from "./ui/ContextMenu";
import type { MenuEntry } from "./ui/Menu";

export interface ChatTabItem {
  id: string;
  title: string;
  project: string;
  model: string;
  status: "idle" | "working" | "waiting" | "error" | "completed";
  titlePulse?: number;
}

interface Props {
  tabs: ChatTabItem[];
  activeId?: string;
  canReopen: boolean;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onReopen: () => void;
  onReorder: (id: string, index: number) => void;
}

const STATUS_LABEL = { idle: "", working: "Working", waiting: "Waiting for input", error: "Error", completed: "Finished while away" };

/** A compact workspace shelf, independent of the sidebar's chat library. */
export const ChatTabBar = memo(function ChatTabBar({ tabs, activeId, canReopen, onSelect, onClose, onNew, onReopen, onReorder }: Props) {
  const reduce = useReducedMotion();
  const list = useRef<HTMLDivElement>(null);
  const dragging = useRef<{ id: string; x: number; moved: boolean } | undefined>(undefined);
  const [draggingId, setDraggingId] = useState<string>();
  const suppressClick = useRef(false);
  const contextMenu = useContextMenu();
  const order = tabs.map((tab) => tab.id).join("\n");
  useEffect(() => {
    if (draggingId) return;
    list.current?.querySelector<HTMLElement>("[aria-selected='true']")?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: reduce ? "instant" : "smooth" });
  }, [activeId, order, reduce, draggingId]);

  function moveTab(event: PointerEvent<HTMLButtonElement>) {
    const drag = dragging.current;
    const strip = list.current;
    if (!drag || !strip || (!drag.moved && Math.abs(event.clientX - drag.x) < 5)) return;
    drag.moved = true;
    suppressClick.current = true;
    setDraggingId(drag.id);
    const rect = strip.getBoundingClientRect();
    if (event.clientX < rect.left + 20) strip.scrollLeft -= 24;
    else if (event.clientX > rect.right - 20) strip.scrollLeft += 24;
    const x = event.clientX - rect.left + strip.scrollLeft;
    const nodes = Array.from(strip.querySelectorAll<HTMLElement>(".chat-tab"));
    // Layout offsets exclude motion's transforms, so neighboring springs cannot
    // flip the destination back and forth while a captured pointer keeps moving.
    let destination = 0;
    let distance = Infinity;
    nodes.forEach((node, index) => {
      const delta = Math.abs(x - node.offsetLeft - node.offsetWidth / 2);
      if (delta < distance) { destination = index; distance = delta; }
    });
    if (tabs[destination]?.id !== drag.id) onReorder(drag.id, destination);
  }

  function endDrag(event: PointerEvent<HTMLButtonElement>) {
    dragging.current = undefined;
    setDraggingId(undefined);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }

  function menu(tab: ChatTabItem, index: number): MenuEntry[] {
    return [
      { label: "Close tab", hint: "⌘W", onSelect: () => onClose(tab.id) },
      { label: "Move tab left", disabled: index === 0, onSelect: () => onReorder(tab.id, index - 1) },
      { label: "Move tab right", disabled: index === tabs.length - 1, onSelect: () => onReorder(tab.id, index + 1) },
      "separator",
      { label: "Reopen closed tab", disabled: !canReopen, onSelect: onReopen }
    ];
  }

  return (
    <div className="chat-tab-strip">
      <div className="chat-tab-drag" data-tauri-drag-region />
      <div ref={list} className="chat-tab-list" role="tablist" aria-label="Open chats" data-tauri-drag-region
        onKeyDown={(event) => {
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          const index = tabs.findIndex((tab) => tab.id === activeId);
          const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1
            : (index + (event.key === "ArrowLeft" ? -1 : 1) + tabs.length) % tabs.length;
          if (tabs[next]) {
            onSelect(tabs[next].id);
            list.current?.querySelectorAll<HTMLButtonElement>("[role='tab']")[next]?.focus();
          }
        }}>
        {tabs.map((tab, index) => (
          <motion.div key={tab.id} layout={reduce ? false : "position"} transition={{ type: "spring", stiffness: 420, damping: 34 }}
            className={`chat-tab ${tab.id === activeId ? "active" : ""}${tab.id === draggingId ? " dragging" : ""}`}
            onContextMenu={(event) => contextMenu(event, menu(tab, index), "Tab menu")}>
            <Tooltip side="bottom" label={<span className="chat-tab-detail"><strong>{tab.title}</strong><span>{tab.project} · {tab.model}</span>{STATUS_LABEL[tab.status] && <span>{STATUS_LABEL[tab.status]}</span>}</span>}>
              <button type="button" role="tab" id={`tab-${tab.id}`} aria-controls="chat-tab-panel"
                aria-label={`${tab.title}${STATUS_LABEL[tab.status] ? ` — ${STATUS_LABEL[tab.status]}` : ""}`}
                aria-selected={tab.id === activeId} tabIndex={tab.id === activeId ? 0 : -1}
                className="chat-tab-select" draggable={false}
                onClick={() => { if (suppressClick.current) { suppressClick.current = false; return; } onSelect(tab.id); }}
                onPointerDown={(event) => {
                  if (event.button !== 0) return;
                  suppressClick.current = false;
                  dragging.current = { id: tab.id, x: event.clientX, moved: false };
                  event.currentTarget.setPointerCapture?.(event.pointerId);
                }}
                onPointerMove={moveTab} onPointerUp={endDrag} onPointerCancel={endDrag} onLostPointerCapture={endDrag}>
                <span className={`chat-tab-status ${tab.status}`} aria-hidden="true">
                  {tab.status === "working" ? <OrbitSpinner active /> : tab.status === "error" ? "!" : tab.status === "waiting" ? "?" : tab.status === "completed" ? <Icon name="check" /> : <Icon name="comment" />}
                </span>
                <TextSwap text={tab.title} swapKey={tab.titlePulse ?? 0} variant="title" />
              </button>
            </Tooltip>
            <button type="button" className="icon-button chat-tab-close" aria-label={`Close tab ${tab.title}`} tabIndex={tab.id === activeId ? 0 : -1} onClick={() => onClose(tab.id)}><Icon name="close" /></button>
          </motion.div>
        ))}
      </div>
      <div className="chat-tab-actions">
        <Tooltip label={<>New chat <kbd>⌘N</kbd></>} side="bottom"><button type="button" className="icon-button" aria-label="New chat tab" onClick={onNew}><Icon name="plus" /></button></Tooltip>
        <MenuButton label="Open tabs" items={[
          ...tabs.map((tab, index) => ({ label: tab.title, selected: tab.id === activeId, hint: index < 8 ? `⌘${index + 1}` : index === tabs.length - 1 ? "⌘9" : undefined, onSelect: () => onSelect(tab.id) })),
          "separator", { label: "Reopen closed tab", disabled: !canReopen, onSelect: onReopen }
        ]} />
      </div>
      <div className="chat-tab-drag" data-tauri-drag-region />
    </div>
  );
}, (previous, next) => previous.activeId === next.activeId && previous.canReopen === next.canReopen
  && previous.onSelect === next.onSelect && previous.onClose === next.onClose && previous.onNew === next.onNew
  && previous.onReopen === next.onReopen && previous.onReorder === next.onReorder
  && previous.tabs.length === next.tabs.length && previous.tabs.every((tab, index) => {
    const other = next.tabs[index];
    return tab.id === other.id && tab.title === other.title && tab.project === other.project
      && tab.model === other.model && tab.status === other.status && tab.titlePulse === other.titlePulse;
  }));
