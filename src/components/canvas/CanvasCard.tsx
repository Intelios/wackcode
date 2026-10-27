import { createContext, useContext } from "react";
import { motion, useReducedMotion } from "motion/react";
import type { CanvasBoard, CanvasTouch } from "../../canvas-state";
import type { CanvasSession } from "../../canvas-session";
import type { CanvasActivity } from "./CanvasPanel";
import { Icon } from "../Icons";

/** The chat's living documents and App-owned navigation; viewers share one session per chat. */
export interface CanvasScope {
  boards: ReadonlyMap<string, CanvasBoard>;
  taskId?: string;
  session?: CanvasSession;
  activity?: CanvasActivity | ReadonlyMap<string, CanvasActivity>;
  expandedId?: string;
  focus?: { canvasId: string; ids: string[]; nonce: number };
  /** Navigate to the live occurrence and focus surviving artboards from this turn. */
  onOpen?: (canvasId: string, ids: string[]) => void;
  onExpand?: (canvasId: string) => void;
  onRestore?: () => void;
  onMention?: (text: string) => void;
}

export const CanvasScopeContext = createContext<CanvasScope>({ boards: new Map() });

export function canvasScopeActivity(scope: CanvasScope, canvasId: string): CanvasActivity {
  return typeof scope.activity === "object" ? scope.activity.get(canvasId) : scope.activity;
}

export function canvasTouchSummary(touch: CanvasTouch): string {
  return [
    touch.added.length ? `Added ${touch.added.length}` : "",
    touch.updated.length ? `Revised ${touch.updated.length}` : "",
    touch.removed.length ? `Removed ${touch.removed.length}` : "",
  ].filter(Boolean).join(" · ") || "Renamed";
}

/** Earlier turns retain a compact reference. Only their latest owner hosts the live board. */
export function CanvasCard({ touch, index = 0 }: { touch: CanvasTouch; index?: number }) {
  const { boards, onOpen } = useContext(CanvasScopeContext);
  const reduce = useReducedMotion() ?? false;
  const board = boards.get(touch.canvas);
  const touched = new Set([...touch.added, ...touch.updated]);
  const focus = board?.artboards.filter((item) => touched.has(item.id)).map((item) => item.id) ?? [];
  return (
    <motion.figure className="canvas-card canvas-reference" aria-label={`Canvas: ${touch.title}`}
      initial={reduce ? false : { opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 420, damping: 34, delay: reduce ? 0 : index * 0.06 }}>
      <figcaption className="canvas-card-head">
        <span className="canvas-card-mark" aria-hidden="true"><Icon name="canvas" /></span>
        <span className="canvas-card-title"><span className="canvas-card-kicker">Canvas</span><strong>{board?.title ?? touch.title}</strong></span>
        <span className="canvas-reference-summary">{canvasTouchSummary(touch)}</span>
        <button type="button" className="secondary-button compact canvas-card-open" disabled={!onOpen || !board} onClick={() => onOpen?.(touch.canvas, focus)}>
          <Icon name="chevron" />View latest
        </button>
      </figcaption>
      {!board && <p className="canvas-card-foot">This canvas is gone</p>}
    </motion.figure>
  );
}
