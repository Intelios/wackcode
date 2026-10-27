import { useContext, useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { createCanvasSession, MIN_CANVAS_HEIGHT, useCanvasView } from "../../canvas-session";
import type { CanvasTouch } from "../../canvas-state";
import { CanvasPanel } from "./CanvasPanel";
import { CanvasScopeContext, canvasScopeActivity } from "./CanvasCard";
import { Icon } from "../Icons";

interface Props {
  touch: CanvasTouch;
  ownerKey: string;
  onResizeStart?: () => void;
  onResizeEnd?: () => void;
}

/** A full-width transcript occurrence. Its fixed-height placeholder survives workspace expansion. */
export function CanvasInline({ touch, ownerKey, onResizeStart, onResizeEnd }: Props) {
  const scope = useContext(CanvasScopeContext);
  const [fallbackSession] = useState(createCanvasSession);
  const session = scope.session ?? fallbackSession;
  const state = useCanvasView(session, scope.taskId ?? "", touch.canvas);
  const board = scope.boards.get(touch.canvas);
  const root = useRef<HTMLElement>(null);
  const [maxHeight, setMaxHeight] = useState(720);
  const [seen, setSeen] = useState(false);
  const resizeCleanup = useRef<(() => void) | undefined>(undefined);
  const height = Math.min(maxHeight, state.height);
  const expanded = scope.expandedId === touch.canvas || state.expanded;

  useLayoutEffect(() => {
    const scroller = root.current?.closest<HTMLElement>(".conversation-scroll");
    const measure = () => setMaxHeight(Math.max(MIN_CANVAS_HEIGHT, Math.round((scroller?.clientHeight || window.innerHeight) * 0.85)));
    measure();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    if (scroller) observer?.observe(scroller);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, []);

  useEffect(() => {
    if (seen || !root.current) return;
    if (typeof IntersectionObserver === "undefined") { setSeen(true); return; }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) { setSeen(true); observer.disconnect(); }
    }, { root: root.current.closest(".conversation-scroll"), rootMargin: "600px" });
    observer.observe(root.current);
    return () => observer.disconnect();
  }, [seen]);
  useEffect(() => () => resizeCleanup.current?.(), []);

  const resize = (next: number) => session.update(touch.canvas, { height: Math.round(Math.max(MIN_CANVAS_HEIGHT, Math.min(maxHeight, next))) });
  const startResize = (event: ReactPointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    resizeCleanup.current?.();
    const startY = event.clientY;
    const startHeight = height;
    const wasActive = state.active;
    session.activate(touch.canvas);
    onResizeStart?.();
    const move = (next: PointerEvent) => resize(startHeight + next.clientY - startY);
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      resizeCleanup.current = undefined;
      if (!wasActive) session.update(touch.canvas, { active: false });
      onResizeEnd?.();
    };
    resizeCleanup.current = end;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  };

  if (!board) return null;
  return <figure ref={root} className="canvas-inline" style={{ height }} data-canvas-inline={touch.canvas} data-canvas-owner={ownerKey}
    aria-label={`Canvas workspace: ${board.title}`}>
    {expanded ? <div className="canvas-inline-placeholder">
      <span className="canvas-card-mark" aria-hidden="true"><Icon name="canvas" /></span>
      <strong>{board.title}</strong><span>Expanded above the composer</span>
      <button type="button" className="secondary-button compact" onClick={scope.onRestore}>Restore canvas</button>
    </div> : <>
      <CanvasPanel taskId={scope.taskId ?? ""} board={board} session={session} activity={canvasScopeActivity(scope, touch.canvas)}
        focus={scope.focus?.canvasId === touch.canvas ? scope.focus : undefined} visible={seen}
        onExpand={() => scope.onExpand?.(touch.canvas)} onMention={scope.onMention ?? (() => {})} />
      <div className="canvas-height-resizer" role="separator" aria-label="Canvas height" aria-orientation="horizontal"
        aria-valuemin={MIN_CANVAS_HEIGHT} aria-valuemax={maxHeight} aria-valuenow={height} tabIndex={0}
        onPointerDown={startResize} onKeyDown={(event) => {
          let next = height;
          if (event.key === "ArrowUp") next -= event.shiftKey ? 60 : 24;
          else if (event.key === "ArrowDown") next += event.shiftKey ? 60 : 24;
          else if (event.key === "Home") next = MIN_CANVAS_HEIGHT;
          else if (event.key === "End") next = maxHeight;
          else return;
          event.preventDefault();
          onResizeStart?.();
          resize(next);
          onResizeEnd?.();
        }}><span aria-hidden="true" /></div>
    </>}
  </figure>;
}
