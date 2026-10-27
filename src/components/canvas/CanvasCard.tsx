import { createContext, useContext } from "react";
import { motion, useReducedMotion } from "motion/react";
import type { CanvasBoard, CanvasTouch } from "../../canvas-state";
import type { CanvasActivity } from "./CanvasPanel";
import { Icon } from "../Icons";

/** The chat's living documents and App-owned navigation to the Canvas tab. */
export interface CanvasScope {
  boards: ReadonlyMap<string, CanvasBoard>;
  activity?: ReadonlyMap<string, CanvasActivity>;
  /** The canvas the side panel is showing, if any. */
  shownId?: string;
  /** Show the canvas in the side panel and focus the surviving artboards this turn touched. */
  onOpen?: (canvasId: string, ids: string[]) => void;
}

export const CanvasScopeContext = createContext<CanvasScope>({ boards: new Map() });

export function canvasTouchSummary(touch: CanvasTouch): string {
  return [
    touch.added.length ? `Added ${touch.added.length}` : "",
    touch.updated.length ? `Revised ${touch.updated.length}` : "",
    touch.removed.length ? `Removed ${touch.removed.length}` : "",
  ].filter(Boolean).join(" · ") || "Renamed";
}

/**
 * One row per canvas a turn touched, under that turn's work: the board itself lives in the side
 * panel's Canvas tab, and this opens it on what the turn changed.
 */
export function CanvasCard({ touch, index = 0 }: { touch: CanvasTouch; index?: number }) {
  const { boards, activity, shownId, onOpen } = useContext(CanvasScopeContext);
  const reduce = useReducedMotion() ?? false;
  const board = boards.get(touch.canvas);
  const touched = new Set([...touch.added, ...touch.updated]);
  const focus = board?.artboards.filter((item) => touched.has(item.id)).map((item) => item.id) ?? [];
  const live = activity?.get(touch.canvas);
  const title = board?.title ?? touch.title;
  return (
    <motion.div className="canvas-card-wrap" initial={reduce ? false : { opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 420, damping: 34, delay: reduce ? 0 : index * 0.06 }}>
      <button type="button" className={`canvas-card${shownId === touch.canvas ? " shown" : ""}${live ? " live" : ""}`}
        aria-label={board ? `Canvas: ${title}. ${canvasTouchSummary(touch)}. Open in the side panel` : `Canvas: ${title}. This canvas is gone`}
        aria-current={shownId === touch.canvas ? "true" : undefined}
        disabled={!onOpen || !board} onClick={() => onOpen?.(touch.canvas, focus)}>
        <span className="canvas-card-mark" aria-hidden="true"><Icon name="canvas" /></span>
        <span className="canvas-card-title">{title}</span>
        <span className="canvas-card-summary">{board ? live ? live === "looking" ? "Checking…" : "Drawing…" : canvasTouchSummary(touch) : "Gone"}</span>
        {board && <Icon name="chevron" className="canvas-card-chevron" />}
      </button>
    </motion.div>
  );
}
