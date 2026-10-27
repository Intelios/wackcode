import { useEffect, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { swapDirection, swapKey, type SidePanelView } from "../side-panel";
import { PanelTabStrip, type PanelAddItem, type PanelTabItem } from "./PanelTabStrip";
import { clampPanelDrag } from "../shell-layout";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

/** A new view arrives from one side as the old one clears out toward the other (`swapDirection`). */
function pageVariants(reduce: boolean) {
  return {
    enter: (direction: number) => ({ opacity: 0, x: 18 * direction }),
    center: { opacity: 1, x: 0 },
    exit: (direction: number) => ({ opacity: 0, x: -12 * direction, transition: { duration: reduce ? 0 : 0.16, ease: EASE } })
  };
}

interface SidePanelProps {
  /** What the panel shows; null closes it. */
  view: SidePanelView | null;
  /** The width on screen, already fitted to the window (`solveShell`). */
  width: number;
  /** A drag's new width: the user's preference, which may be wider than the room left with the sidebar. */
  onWidthChange: (width: number) => void;
  /** Double-clicking the resizer: back to a balanced split. */
  onResetWidth: () => void;
  /** The window is being resized: follow it without the drawer's ease. */
  immediate?: boolean;
  /** The panel's accessible name for the view it shows. */
  label: string;
  /** The tabs this chat can show, in order; `view` is the front one. */
  tabs: PanelTabItem[];
  addable: PanelAddItem[];
  expanded: boolean;
  onSelectTab: (view: SidePanelView) => void;
  onCloseTab: (view: SidePanelView) => void;
  onAddTab: (view: SidePanelView) => void;
  onToggleExpand: () => void;
  onHide: () => void;
  children: (view: SidePanelView) => ReactNode;
}

/**
 * The right-hand panel beside the chat. It holds tabs (`side-panel.ts`), one view in front, and
 * owns what they share: the tab strip, the width and its resizer, the drawer motion when it opens
 * and closes, and the page turn when another tab comes forward. Each view brings its own toolbar.
 *
 * Opening animates the panel's width while its content keeps the final width, so the content
 * rides in with the panel's edge like a drawer instead of reflowing on every frame. Dragging the
 * resizer changes the width without a transition.
 */
export function SidePanel({ view, width, onWidthChange, onResetWidth, immediate = false, label, tabs, addable, expanded, onSelectTab, onCloseTab, onAddTab, onToggleExpand, onHide, children }: SidePanelProps) {
  const reduce = useReducedMotion();
  const [resizing, setResizing] = useState(false);
  const resizeCleanup = useRef<(() => void) | undefined>(undefined);
  useEffect(() => () => resizeCleanup.current?.(), []);
  // Which way the next page turn slides, derived when the view changes (React's pattern for
  // state that follows a prop).
  const [shown, setShown] = useState(view);
  const [direction, setDirection] = useState<1 | -1>(1);
  if (view !== shown) {
    setShown(view);
    // A page slides in from the side its tab sits on; views without both tabs fall back to kind order.
    const from = shown ? tabs.findIndex((tab) => swapKey(tab.view) === swapKey(shown)) : -1;
    const to = view ? tabs.findIndex((tab) => swapKey(tab.view) === swapKey(view)) : -1;
    if (view) setDirection(from >= 0 && to >= 0 && from !== to ? (to < from ? -1 : 1) : swapDirection(shown, view));
  }

  function startResize(event: React.PointerEvent) {
    event.preventDefault();
    resizeCleanup.current?.();
    const startX = event.clientX;
    const startWidth = width;
    setResizing(true);
    // Past the room the sidebar leaves, the sidebar steps aside (`solveShell`); the chat column
    // keeps its minimum either way.
    const move = (moveEvent: PointerEvent) =>
      onWidthChange(clampPanelDrag(startWidth + startX - moveEvent.clientX, window.innerWidth));
    const cleanup = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
    };
    const end = () => {
      setResizing(false);
      cleanup();
      resizeCleanup.current = undefined;
    };
    resizeCleanup.current = cleanup;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
  }

  return (
    <AnimatePresence initial={false}>
      {view && (
        <motion.aside
          key="side-panel"
          id="side-panel"
          className="side-panel"
          aria-label={label}
          initial={reduce ? false : { width: 0 }}
          animate={{ width }}
          exit={{ width: 0, transition: { duration: reduce ? 0 : 0.26, ease: EASE } }}
          transition={resizing || immediate || reduce ? { duration: 0 } : { duration: 0.38, ease: EASE }}
        >
          <div className="panel-resizer" onPointerDown={startResize} onDoubleClick={onResetWidth} />
          <div className="side-panel-inner" style={{ width }}>
            <PanelTabStrip tabs={tabs} active={view} addable={addable} expanded={expanded} onSelect={onSelectTab}
              onClose={onCloseTab} onAdd={onAddTab} onToggleExpand={onToggleExpand} onHide={onHide} />
            <div className="side-panel-pages" id="side-panel-page" role="tabpanel" aria-labelledby={`panel-tab-${swapKey(view)}`}>
            <AnimatePresence mode="popLayout" initial={false} custom={direction} propagate>
              <motion.div
                key={swapKey(view)}
                className="side-panel-page"
                custom={direction}
                variants={pageVariants(reduce === true)}
                initial={reduce ? false : "enter"}
                animate="center"
                exit="exit"
                transition={{ duration: reduce ? 0 : 0.26, ease: EASE }}
              >
                {children(view)}
              </motion.div>
            </AnimatePresence>
            </div>
          </div>
        </motion.aside>
      )}
    </AnimatePresence>
  );
}
