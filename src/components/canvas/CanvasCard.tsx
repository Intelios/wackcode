import { createContext, useContext, useMemo } from "react";
import { motion, useReducedMotion } from "motion/react";
import { layoutCanvas } from "../../canvas-layout";
import type { CanvasBoard, CanvasTouch } from "../../canvas-state";
import { Icon } from "../Icons";

/** What a canvas card needs from the chat: the canvases as they are now, and how to open one. */
export interface CanvasScope {
  boards: ReadonlyMap<string, CanvasBoard>;
  /** Opens the canvas in the side panel, showing these artboards. */
  onOpen?: (canvasId: string, ids: string[]) => void;
}

export const CanvasScopeContext = createContext<CanvasScope>({ boards: new Map() });

function summary(touch: CanvasTouch): string {
  const parts = [
    touch.added.length ? `Added ${touch.added.length}` : "",
    touch.updated.length ? `Revised ${touch.updated.length}` : "",
    touch.removed.length ? `Removed ${touch.removed.length}` : "",
  ].filter(Boolean);
  return parts.join(" · ") || "Renamed";
}

/**
 * A turn's mark on a canvas, under its answer: the canvas's title, an outline map of the board
 * as it is now with this turn's artboards lit, and a way in. Older turns keep their cards: the
 * board is one living document, and a card is a door into the part that turn drew.
 */
export function CanvasCard({ touch, index = 0 }: { touch: CanvasTouch; index?: number }) {
  const { boards, onOpen } = useContext(CanvasScopeContext);
  const reduce = useReducedMotion() ?? false;
  const board = boards.get(touch.canvas);
  const layout = useMemo(() => board ? layoutCanvas(board) : undefined, [board]);
  const touched = new Set([...touch.added, ...touch.updated]);
  const lit = layout?.artboards.filter((item) => touched.has(item.id)) ?? [];
  const focus = lit.map((item) => item.id);
  const open = () => onOpen?.(touch.canvas, focus);
  const names = board ? lit.map((item) => board.artboards.find((artboard) => artboard.id === item.id)?.name).filter(Boolean) : [];
  const gone = board === undefined || (touched.size > 0 && lit.length === 0);
  const bounds = layout?.bounds;
  const pad = bounds ? Math.max(bounds.width, bounds.height) * 0.02 : 0;

  return (
    <motion.figure
      className="canvas-card"
      aria-label={`Canvas: ${touch.title}`}
      initial={reduce ? false : { opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 420, damping: 34, delay: reduce ? 0 : index * 0.06 }}
    >
      <figcaption className="canvas-card-head">
        <span className="canvas-card-mark" aria-hidden="true"><Icon name="canvas" /></span>
        <span className="canvas-card-title">
          <span className="canvas-card-kicker">Canvas</span>
          <strong>{board?.title ?? touch.title}</strong>
        </span>
        <button type="button" className="secondary-button compact canvas-card-open" disabled={!onOpen || !board} onClick={open}>
          <Icon name="expand" />Open canvas
        </button>
      </figcaption>
      {layout && bounds && bounds.width > 0 && (
        <button type="button" className="canvas-card-map" aria-label={`Show ${names.length ? names.join(", ") : "the canvas"} on the canvas`} disabled={!onOpen} onClick={open}>
          <svg viewBox={`${-pad} ${-pad} ${bounds.width + pad * 2} ${bounds.height + pad * 2}`} preserveAspectRatio="xMidYMid meet" aria-hidden="true">
            {layout.artboards.map((item) => (
              <rect key={item.id} className={touched.has(item.id) ? "lit" : undefined} x={item.x} y={item.y} width={item.width} height={item.height}
                rx={Math.min(item.width, item.height) * 0.03} />
            ))}
          </svg>
        </button>
      )}
      <p className="canvas-card-foot">
        <span>{summary(touch)}</span>
        {names.length > 0 && <span className="canvas-card-names">{names.join(", ")}</span>}
        {gone && <span className="canvas-card-names">{board ? "Since removed" : "This canvas is gone"}</span>}
      </p>
    </motion.figure>
  );
}
