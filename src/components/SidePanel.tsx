import { useEffect, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { swapDirection, swapKey, type SidePanelView } from "../side-panel";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

export const MIN_PANEL_WIDTH = 290;
export const MAX_PANEL_WIDTH = 1200;

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
  width: number;
  onWidthChange: (width: number) => void;
  /** The panel's accessible name for the view it shows. */
  label: string;
  children: (view: SidePanelView) => ReactNode;
}

/**
 * The right-hand panel beside the chat. It holds one view at a time (`side-panel.ts`) and owns
 * what they share: the width and its resizer, the drawer motion when it opens and closes, and
 * the page turn when one view replaces another. Each view brings its own header.
 *
 * Opening animates the panel's width while its content keeps the final width, so the content
 * rides in with the panel's edge like a drawer instead of reflowing on every frame. Dragging the
 * resizer changes the width without a transition.
 */
export function SidePanel({ view, width, onWidthChange, label, children }: SidePanelProps) {
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
    if (view) setDirection(swapDirection(shown, view));
  }

  function startResize(event: React.PointerEvent) {
    event.preventDefault();
    resizeCleanup.current?.();
    const startX = event.clientX;
    const startWidth = width;
    setResizing(true);
    const move = (moveEvent: PointerEvent) =>
      onWidthChange(Math.max(MIN_PANEL_WIDTH, Math.min(MAX_PANEL_WIDTH, startWidth + startX - moveEvent.clientX)));
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
          transition={resizing || reduce ? { duration: 0 } : { duration: 0.38, ease: EASE }}
        >
          <div className="panel-resizer" onPointerDown={startResize} />
          <div className="side-panel-inner" style={{ width }}>
            <AnimatePresence mode="popLayout" initial={false} custom={direction}>
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
        </motion.aside>
      )}
    </AnimatePresence>
  );
}
