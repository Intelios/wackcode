import { useId, type KeyboardEvent } from "react";
import { motion, useReducedMotion } from "motion/react";
import { swapKey, type SidePanelView } from "../side-panel";
import { Icon, type IconName } from "./Icons";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";

/** One tab as the strip draws it (App's `panelTabItems` words it). */
export interface PanelTabItem {
  view: SidePanelView;
  label: string;
  icon: IconName;
  /** The agent is at work in this view: its icon sends out a ring, even from the back. */
  live?: boolean;
  count?: number;
}

/** A view the "+" menu can open. */
export interface PanelAddItem {
  view: SidePanelView;
  label: string;
  icon: IconName;
  keys?: string;
}

interface PanelTabStripProps {
  tabs: PanelTabItem[];
  active: SidePanelView;
  addable: PanelAddItem[];
  expanded: boolean;
  onSelect: (view: SidePanelView) => void;
  onClose: (view: SidePanelView) => void;
  onAdd: (view: SidePanelView) => void;
  onToggleExpand: () => void;
  onHide: () => void;
}

/**
 * The side panel's top bar: its tabs as pills on the shell, whose fill glides to the front one
 * like the header's view track, then "+" for the views not open yet, Expand and Hide. It runs in
 * line with the chat header, and whatever it leaves bare drags the window.
 */
export function PanelTabStrip({ tabs, active, addable, expanded, onSelect, onClose, onAdd, onToggleExpand, onHide }: PanelTabStripProps) {
  const reduce = useReducedMotion() ?? false;
  const pillId = useId();
  const activeKey = swapKey(active);

  // Arrow keys walk the tabs, as in any macOS tab bar; Home and End jump to either end.
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const at = tabs.findIndex((tab) => swapKey(tab.view) === activeKey);
    const to = event.key === "ArrowRight" ? at + 1 : event.key === "ArrowLeft" ? at - 1 : event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : -2;
    if (to === -2 || !tabs[to]) return;
    event.preventDefault();
    onSelect(tabs[to].view);
    requestAnimationFrame(() => document.getElementById(`panel-tab-${swapKey(tabs[to].view)}`)?.focus());
  }

  return (
    <div className="panel-tabstrip" data-tauri-drag-region>
      <div className="panel-tabs" role="tablist" aria-label="Side panel tabs" onKeyDown={onKeyDown} data-tauri-drag-region>
        {tabs.map((tab) => {
          const key = swapKey(tab.view);
          const selected = key === activeKey;
          return (
            <div key={key} className={`panel-tab${selected ? " active" : ""}`}>
              {selected && (
                <motion.span className="panel-tab-pill" aria-hidden="true" layoutId={reduce ? undefined : pillId}
                  style={{ borderRadius: 999 }} transition={{ type: "spring", stiffness: 480, damping: 36 }} />
              )}
              <button type="button" role="tab" id={`panel-tab-${key}`} className="panel-tab-select" aria-selected={selected}
                aria-controls="side-panel-page" tabIndex={selected ? 0 : -1} title={tab.label} onClick={() => onSelect(tab.view)}
                aria-label={tab.count ? `${tab.label} (${tab.count} changed)` : tab.label}>
                <span className={`panel-tab-icon${tab.live ? " live" : ""}`} aria-hidden="true"><Icon name={tab.icon} /></span>
                <span className="panel-tab-label">{tab.label}</span>
                {tab.count ? <em className="changes-badge" aria-hidden="true">{tab.count}</em> : null}
              </button>
              <button type="button" className="panel-tab-close" aria-label={`Close ${tab.label} tab`} onClick={() => onClose(tab.view)}>
                <Icon name="close" />
              </button>
            </div>
          );
        })}
      </div>
      {addable.length > 0 && (
        <MenuButton label="Open a tab" icon={<Icon name="plus" />} className="panel-tab-add" align="start"
          items={addable.map((item) => ({ label: item.label, icon: <Icon name={item.icon} />, hint: item.keys, onSelect: () => onAdd(item.view) }))} />
      )}
      <span className="panel-tabstrip-drag" data-tauri-drag-region />
      <div className="panel-tabstrip-actions">
        <Tooltip label={expanded ? "Restore" : "Expand over the chat"} side="bottom">
          <button type="button" className="icon-button" aria-label={expanded ? "Restore panel" : "Expand panel"} onClick={onToggleExpand}>
            <Icon name={expanded ? "collapse" : "expand"} />
          </button>
        </Tooltip>
        <Tooltip label="Hide panel" side="bottom">
          <button type="button" className="icon-button" aria-label="Hide panel" onClick={onHide}><Icon name="panel" /></button>
        </Tooltip>
      </div>
    </div>
  );
}
