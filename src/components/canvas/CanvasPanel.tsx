import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { fitBounds, zoomTo, type Camera, type Size } from "../../canvas-camera";
import { boundsOf, layoutCanvas } from "../../canvas-layout";
import { artboardMention, resolveArtboardHtml, type CanvasBoard as Board } from "../../canvas-state";
import { Icon } from "../Icons";
import { CanvasBoard } from "./CanvasBoard";
import type { ArtboardMotion } from "./CanvasArtboard";
import { createCanvasSession, useCanvasView, type CanvasSession } from "../../canvas-session";
import { canvasScopeActivity, type CanvasScope } from "./CanvasCard";

const GLIDE_MS = 420;
/** How long a new artboard's pop or a revised one's glint plays. */
const MOTION_MS = 1_400;
/** The fit for one artboard never magnifies past life size. */
const FOCUS_MAX_ZOOM = 1;

export type CanvasActivity = "drawing" | "looking" | undefined;

interface CanvasPanelProps {
  taskId: string;
  board: Board;
  session?: CanvasSession;
  activity?: CanvasActivity;
  expanded?: boolean;
  /** A reference asked to select these artboards; a new nonce is a new request. */
  focus?: { ids: string[]; nonce: number };
  onExpand: () => void;
  onMention: (text: string) => void;
  /** Offscreen transcript panes keep their layout without staging any artboard documents. */
  visible?: boolean;
}

function signature(board: Board, id: string): string {
  const artboard = board.artboards.find((item) => item.id === id);
  return artboard ? JSON.stringify([artboard.name, artboard.width, artboard.height, artboard.notes, resolveArtboardHtml(board, artboard)]) : "";
}

const ease = (t: number) => 1 - (1 - t) ** 3;

/**
 * One live Canvas surface, shared by its inline and expanded hosts. The session owns durable
 * viewer state; this host only owns its viewport and short-lived animation frames. Frames stay
 * pointer-inert until stepped into, and an idle inline board leaves wheel events to the chat.
 */
export function CanvasPanel({ taskId, board, session: suppliedSession, activity, expanded = false, focus, onExpand, onMention, visible = true }: CanvasPanelProps) {
  const reduce = useReducedMotion() ?? false;
  const [fallbackSession] = useState(createCanvasSession);
  const session = suppliedSession ?? fallbackSession;
  const state = useCanvasView(session, taskId, board.id);
  const { camera, measured, selected, entered } = state;
  const active = expanded || state.active;
  const root = useRef<HTMLElement>(null);
  const [viewport, setViewport] = useState<Size>({ width: 0, height: 0 });
  const [motions, setMotions] = useState<Record<string, ArtboardMotion>>({});
  const arrivals = state.arrivals;
  const cameraRef = useRef(camera);
  cameraRef.current = camera;
  const glideFrame = useRef(0);
  const motionTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const update = useCallback((patch: Parameters<CanvasSession["update"]>[1]) => session.update(board.id, patch), [session, board.id]);
  const setCamera = useCallback((next: Camera) => update({ camera: next }), [update]);
  const setArrivals = useCallback((next: string[] | ((current: string[]) => string[])) => {
    update({ arrivals: typeof next === "function" ? next(session.get(board.id)?.arrivals ?? []) : next });
  }, [update, session, board.id]);
  const setSelected = useCallback((id: string | undefined) => update({ selected: id }), [update]);
  const setEntered = useCallback((id: string | undefined) => update({ entered: id }), [update]);
  const done = useCallback(() => {
    update({ active: false, entered: undefined });
    if (expanded) onExpand();
  }, [update, expanded, onExpand]);
  const activate = useCallback(() => session.activate(board.id), [session, board.id]);

  useEffect(() => {
    if (!active || expanded) return;
    const outside = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Element) || root.current?.contains(target) || root.current?.closest("[data-canvas-inline]")?.contains(target)) return;
      done();
    };
    let blurTimer: ReturnType<typeof setTimeout> | undefined;
    const blur = () => {
      // Moving focus into an artboard blurs the parent window too. Wait for that handoff:
      // hasFocus includes descendant frames, but becomes false when the user leaves the app.
      clearTimeout(blurTimer);
      blurTimer = setTimeout(() => { if (!document.hasFocus()) done(); }, 0);
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", outside);
    window.addEventListener("blur", blur);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
      window.removeEventListener("blur", blur);
      clearTimeout(blurTimer);
    };
  }, [active, expanded, done]);

  const layout = useMemo(() => layoutCanvas(board, measured), [board, measured]);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;

  const stopGlide = () => {
    if (glideFrame.current) cancelAnimationFrame(glideFrame.current);
    glideFrame.current = 0;
  };
  useEffect(() => () => {
    stopGlide();
    for (const timer of motionTimers.current.values()) clearTimeout(timer);
  }, []);

  const glide = useCallback((target: Camera) => {
    stopGlide();
    if (reduce) { setCamera(target); return; }
    const from = cameraRef.current;
    const start = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / GLIDE_MS);
      const k = ease(t);
      setCamera({ x: from.x + (target.x - from.x) * k, y: from.y + (target.y - from.y) * k, zoom: from.zoom + (target.zoom - from.zoom) * k });
      glideFrame.current = t < 1 ? requestAnimationFrame(step) : 0;
    };
    glideFrame.current = requestAnimationFrame(step);
  }, [reduce, setCamera]);

  const showIds = useCallback((ids: readonly string[], maxZoom = FOCUS_MAX_ZOOM) => {
    const bounds = boundsOf(layoutRef.current.artboards.filter((item) => ids.includes(item.id)));
    if (bounds && viewport.width) glide(fitBounds(bounds, viewport, maxZoom));
  }, [glide, viewport]);

  const fit = useCallback(() => {
    update({ following: true });
    setArrivals([]);
    if (viewport.width && layoutRef.current.artboards.length) glide(fitBounds(layoutRef.current.bounds, viewport, FOCUS_MAX_ZOOM));
  }, [glide, viewport]);

  // The first sight of a canvas with no saved camera fits it whole.
  useEffect(() => {
    if (session.get(board.id)?.fitted || !viewport.width || !layout.artboards.length) return;
    update({ fitted: true });
    setCamera(fitBounds(layout.bounds, viewport, FOCUS_MAX_ZOOM));
  }, [viewport, layout]);

  // Expanding, restoring or resizing the panel refits the board, until the user takes the camera.
  useEffect(() => {
    if (!viewport.width || !viewport.height) return;
    const before = session.get(board.id)?.viewport;
    update({ viewport });
    if (!before?.width || !layoutRef.current.artboards.length) return;
    if (before.width === viewport.width && before.height === viewport.height) return;
    if (session.get(board.id)?.following) glide(fitBounds(layoutRef.current.bounds, viewport, FOCUS_MAX_ZOOM));
    else {
      const current = cameraRef.current;
      setCamera({ ...current, x: current.x + (viewport.width - before.width) / 2, y: current.y + (viewport.height - before.height) / 2 });
    }
  }, [viewport, glide, session, board.id, update, setCamera]);

  // What changed since the last board: new artboards pop, revised ones glint.
  useEffect(() => {
    const next = new Map(board.artboards.map((artboard) => [artboard.id, signature(board, artboard.id)]));
    const previous = session.get(board.id)?.signatures;
    if (previous && Object.keys(previous).length === next.size && [...next].every(([id, value]) => previous[id] === value)) return;
    update({ signatures: Object.fromEntries(next) });
    if (!previous) return;
    const changes: Record<string, ArtboardMotion> = {};
    for (const [id, value] of next) {
      if (!(id in previous)) changes[id] = "pop";
      else if (previous[id] !== value) changes[id] = "glint";
    }
    const ids = Object.keys(changes);
    if (!ids.length) return;
    if (!reduce) setMotions((current) => ({ ...current, ...changes }));
    if (session.get(board.id)?.following) showIds(ids);
    else setArrivals((current) => [...new Set([...current, ...ids])]);
    if (reduce) return;
    // Each artboard's timer survives unrelated board snapshots. A new revision restarts only
    // that artboard's effect; cancelling a prior snapshot cannot leave a glint class stuck.
    for (const id of ids) {
      clearTimeout(motionTimers.current.get(id));
      motionTimers.current.set(id, setTimeout(() => {
        setMotions((current) => { const rest = { ...current }; delete rest[id]; return rest; });
        motionTimers.current.delete(id);
      }, MOTION_MS));
    }
  }, [board]);

  // A card in the transcript asked for these artboards.
  useEffect(() => {
    if (!focus || !viewport.width || session.get(board.id)?.focusNonce === focus.nonce) return;
    update({ focusNonce: focus.nonce });
    const ids = focus.ids.filter((id) => board.artboards.some((artboard) => artboard.id === id));
    if (!ids.length) return;
    update({ following: true });
    setArrivals([]);
    setSelected(ids[0]);
    showIds(ids);
  }, [focus?.nonce, viewport.width > 0]);

  // An artboard that left the board can't stay selected or stepped into.
  useEffect(() => {
    if (selected && !board.artboards.some((artboard) => artboard.id === selected)) setSelected(undefined);
    if (entered && !board.artboards.some((artboard) => artboard.id === entered)) setEntered(undefined);
    const remaining = arrivals.filter((id) => board.artboards.some((artboard) => artboard.id === id));
    if (remaining.length !== arrivals.length) setArrivals(remaining);
  }, [board, selected, entered, arrivals, setArrivals]);

  const userCamera = useCallback((next: Camera) => {
    stopGlide();
    update({ following: false });
    setCamera(next);
  }, [update, setCamera]);
  const onMeasure = useCallback((id: string, height: number) => {
    const current = session.get(board.id)?.measured ?? {};
    if (current[id] !== height) update({ measured: { ...current, [id]: height } });
  }, [session, board.id, update]);
  const zoomToArtboard = useCallback((id: string) => {
    setSelected(id);
    showIds([id]);
  }, [showIds]);
  const enter = useCallback((id: string | undefined) => {
    setEntered(id);
    if (id) {
      setSelected(id);
      showIds([id]);
    }
  }, [showIds]);
  const mention = useCallback((id: string) => {
    const artboard = board.artboards.find((item) => item.id === id);
    if (artboard) onMention(artboardMention(board, artboard));
  }, [board, onMention]);
  const zoomStep = useCallback((factor: number) => {
    update({ following: false });
    glide(zoomTo(cameraRef.current, cameraRef.current.zoom * factor, viewport));
  }, [glide, viewport]);
  const zoomExact = useCallback((zoom: number) => {
    update({ following: false });
    glide(zoomTo(cameraRef.current, zoom, viewport));
  }, [glide, viewport]);

  const selectedArtboard = selected ? board.artboards.find((item) => item.id === selected) : undefined;
  const count = board.artboards.length;
  return (
    <section ref={root} className={`canvas-panel${active ? " interactive" : ""}`} onKeyDown={(event) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (entered) setEntered(undefined);
      else if (active) done();
      else return;
      event.preventDefault();
      event.stopPropagation();
    }}>
      <div className="canvas-stage">
        <CanvasBoard
          board={board}
          active={active}
          visible={visible}
          onActivate={activate}
          onDeactivate={done}
          layout={layout}
          camera={camera}
          onCamera={userCamera}
          onViewport={setViewport}
          selected={selected}
          entered={entered}
          motions={motions}
          onSelect={setSelected}
          onEnter={enter}
          onZoomTo={zoomToArtboard}
          onMeasure={onMeasure}
          onMention={mention}
          onFit={fit}
          onZoomStep={zoomStep}
          onZoomReset={() => zoomExact(1)}
        />
        {count === 0 && (
          <div className="canvas-empty">
            <span className="panel-empty-mark" aria-hidden="true"><Icon name="canvas" /></span>
            <p>Nothing on this canvas any more.</p>
          </div>
        )}
        <div className="canvas-controls">
          {activity && (
            <span className="terminal-pill running" role="status">
              <span className="terminal-pill-dot" aria-hidden="true" />{activity === "looking" ? "Checking artboards" : "Drawing"}
            </span>
          )}
          <button type="button" className="canvas-expand" title={expanded ? "Restore canvas" : "Expand canvas"} aria-label={expanded ? "Restore canvas" : "Expand canvas"} onClick={onExpand}>
            <Icon name={expanded ? "collapse" : "expand"} />
          </button>
        </div>
        <div className="canvas-arrivals-dock">
        <AnimatePresence>
          {arrivals.length > 0 && (
            <motion.button
              key="arrivals"
              type="button"
              className="canvas-arrivals"
              initial={reduce ? false : { opacity: 0, y: 12, scale: 0.94 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.96 }}
              transition={{ type: "spring", stiffness: 520, damping: 34 }}
              onClick={() => {
                update({ following: true });
                showIds(arrivals);
                setArrivals([]);
              }}
            >
              <Icon name="spark" aria-hidden="true" />
              {arrivals.length} new or revised artboard{arrivals.length === 1 ? "" : "s"}
            </motion.button>
          )}
        </AnimatePresence>
        </div>
        {entered ? (
          <p className="canvas-hint" role="status">Interacting with “{board.artboards.find((item) => item.id === entered)?.name}”. Press <kbd>Esc</kbd> to step out.</p>
        ) : selectedArtboard ? (
          <p className="canvas-hint">
            <strong>{selectedArtboard.name}</strong>
            <span>{selectedArtboard.width}×{selectedArtboard.height ?? "auto"}{selectedArtboard.group ? ` · ${selectedArtboard.group}` : ""}</span>
            {selectedArtboard.notes && <span>{selectedArtboard.notes}</span>}
            <span className="canvas-hint-keys"><kbd>↩</kbd> step in</span>
          </p>
        ) : null}
      </div>
    </section>
  );
}

/** Expanded host shares the same session as its inline placeholder, never a second live board. */
export function CanvasExpanded({ scope, canvasId }: { scope: CanvasScope; canvasId: string }) {
  const board = scope.boards.get(canvasId);
  const [fallbackSession] = useState(createCanvasSession);
  const session = scope.session ?? fallbackSession;
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!board) return;
    session.ensure(scope.taskId ?? "", canvasId);
    session.setExpanded(canvasId);
  }, [session, scope.taskId, canvasId, Boolean(board)]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.cancelBubble) return;
      if (event.target instanceof Element && event.target.closest(".popover, [role=dialog], .inline-dialog-wrap")) return;
      if (session.get(canvasId)?.entered) session.update(canvasId, { entered: undefined });
      else scope.onRestore?.();
      event.preventDefault();
      event.stopPropagation();
    };
    document.addEventListener("keydown", key);
    root.current?.querySelector<HTMLElement>(".canvas-viewport")?.focus({ preventScroll: true });
    return () => document.removeEventListener("keydown", key);
  }, [session, canvasId, scope.onRestore]);
  if (!board) return null;
  const activity = canvasScopeActivity(scope, canvasId);
  return <div ref={root} className="canvas-expanded-content"><CanvasPanel taskId={scope.taskId ?? ""} board={board} session={session} activity={activity}
    expanded focus={scope.focus?.canvasId === canvasId ? scope.focus : undefined}
    onExpand={() => scope.onRestore?.()} onMention={scope.onMention ?? (() => {})} /></div>;
}
