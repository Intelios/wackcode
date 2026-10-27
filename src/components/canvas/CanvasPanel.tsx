import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ZOOM_STEP, fitBounds, zoomTo, type Camera, type Size } from "../../canvas-camera";
import { boundsOf, layoutCanvas } from "../../canvas-layout";
import { artboardMention, resolveArtboardHtml, type CanvasBoard as Board } from "../../canvas-state";
import { Icon } from "../Icons";
import { MenuButton } from "../ui/MenuButton";
import { CanvasBoard } from "./CanvasBoard";
import type { ArtboardMotion } from "./CanvasArtboard";
import { CanvasPresent } from "./CanvasPresent";
import { CanvasProgress } from "./CanvasProgress";
import type { NormalizedBlock } from "../../types";
import { useCanvasView, type CanvasSession, type CanvasTool } from "../../canvas-session";

const GLIDE_MS = 420;
/** How long a new artboard's pop or a revised one's glint plays. */
const MOTION_MS = 1_400;
/** The fit for one artboard never magnifies past life size. */
const FOCUS_MAX_ZOOM = 1;

export type CanvasActivity = "drawing" | "looking" | undefined;

/** What saving one artboard needs: its resolved document at its real size. */
export interface ArtboardExport {
  name: string;
  html: string;
  width: number;
  height: number;
  format: "png" | "html";
}

interface CanvasPanelProps {
  taskId: string;
  /** Every canvas in this chat, for the switcher. */
  boards: readonly Board[];
  board: Board;
  session: CanvasSession;
  activity?: CanvasActivity;
  /** The in-flight call, including content arriving before execution starts. */
  progress?: NormalizedBlock;
  /** A transcript card asked to show these artboards; a new nonce is a new request. */
  focus?: { ids: string[]; nonce: number };
  onSelectCanvas: (canvasId: string) => void;
  onMention: (text: string) => void;
  onExport: (artboard: ArtboardExport) => void;
}

function signature(board: Board, id: string): string {
  const artboard = board.artboards.find((item) => item.id === id);
  return artboard ? JSON.stringify([artboard.name, artboard.width, artboard.height, artboard.notes, resolveArtboardHtml(board, artboard)]) : "";
}

const ease = (t: number) => 1 - (1 - t) ** 3;

/**
 * The Canvas tab: one of the chat's boards filling the side-panel page edge to edge, with its
 * tools floating over it rather than a header taking rows from it (the reason this view skips
 * `.panel-header`). The session owns durable viewer state; this host only owns its viewport,
 * short-lived animation frames and Present. Frames stay pointer-inert until played.
 */
export function CanvasPanel({ taskId, boards, board, session, activity, progress, focus, onSelectCanvas, onMention, onExport }: CanvasPanelProps) {
  const reduce = useReducedMotion() ?? false;
  const state = useCanvasView(session, taskId, board.id);
  const { camera, measured, selected, entered, tool } = state;
  const root = useRef<HTMLElement>(null);
  const [presenting, setPresenting] = useState(false);
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
    root.current?.querySelector<HTMLElement>(".canvas-viewport")?.focus({ preventScroll: true });
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

  const setTool = useCallback((next: CanvasTool) => update({ tool: next }), [update]);
  const exportArtboard = useCallback((id: string, format: "png" | "html") => {
    const artboard = board.artboards.find((item) => item.id === id);
    const placed = layoutRef.current.artboards.find((item) => item.id === id);
    if (artboard && placed) onExport({ name: artboard.name, html: resolveArtboardHtml(board, artboard), width: placed.width, height: placed.height, format });
  }, [board, onExport]);
  const presented = useMemo(() => layout.artboards.flatMap((placed) => {
    const artboard = board.artboards.find((item) => item.id === placed.id);
    return artboard ? [{ id: artboard.id, name: artboard.name, group: artboard.group, width: placed.width, height: placed.height,
      fixedHeight: artboard.height !== undefined, html: resolveArtboardHtml(board, artboard) }] : [];
  }), [layout, board]);
  const present = () => {
    update({ entered: undefined });
    setPresenting(true);
  };
  const endPresent = useCallback((lastId: string | undefined) => {
    setPresenting(false);
    if (lastId) { setSelected(lastId); showIds([lastId]); }
    requestAnimationFrame(() => root.current?.querySelector<HTMLElement>(".canvas-viewport")?.focus({ preventScroll: true }));
  }, [setSelected, showIds]);

  const selectedArtboard = selected ? board.artboards.find((item) => item.id === selected) : undefined;
  const count = board.artboards.length;
  const percent = Math.round(camera.zoom * 100);
  return (
    <section ref={root} className="canvas-panel" onKeyDown={(event) => {
      if (event.defaultPrevented) return;
      if (event.key === "Escape" && (entered || selected)) {
        if (entered) setEntered(undefined);
        else setSelected(undefined);
        event.preventDefault();
        event.stopPropagation();
      }
    }}>
      <div className="canvas-stage">
        <CanvasBoard
          board={board}
          visible
          tool={tool}
          onTool={setTool}
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
          onExport={exportArtboard}
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
        <div className="canvas-toolbar canvas-toolbar-start" role="toolbar" aria-label="Canvas tools">
          {boards.length > 1 && <>
            <MenuButton
              className="canvas-switcher"
              label={`Canvas: ${board.title}. Switch canvas`}
              align="start"
              icon={<><span>{board.title}</span><Icon name="chevron" /></>}
              items={boards.map((item) => ({ label: item.title, selected: item.id === board.id, onSelect: () => onSelectCanvas(item.id) }))}
            />
            <span className="canvas-toolbar-divider" aria-hidden="true" />
          </>}
          {([["select", "pointer", "Select", "V"], ["hand", "hand", "Hand", "H"]] as const).map(([value, icon, label, key]) => (
            <button key={value} type="button" className={`icon-button canvas-tool${tool === value ? " on" : ""}`} aria-pressed={tool === value}
              aria-label={label} title={`${label} (${key})`} onClick={() => setTool(value)}>
              <Icon name={icon} />
            </button>
          ))}
          {activity && (
            <span className="canvas-activity" role="status">
              <span className="terminal-pill-dot" aria-hidden="true" />{activity === "looking" ? "Checking artboards" : "Drawing"}
            </span>
          )}
        </div>
        <div className="canvas-toolbar canvas-toolbar-end" role="toolbar" aria-label="View">
          <button type="button" className="icon-button" aria-label="Present" title="Present" disabled={count === 0} onClick={present}>
            <Icon name="present" />
          </button>
          <span className="canvas-toolbar-divider" aria-hidden="true" />
          <button type="button" className="icon-button" aria-label="Zoom out" title="Zoom out (−)" onClick={() => zoomStep(1 / ZOOM_STEP)}><Icon name="minus" /></button>
          <MenuButton
            className="canvas-zoom-level"
            label={`Zoom ${percent}%. Zoom options`}
            icon={<><span>{percent}%</span><Icon name="chevron" /></>}
            items={[
              { label: "Zoom to fit", hint: "⇧1", onSelect: fit },
              { label: "Zoom to selection", hint: "⇧2", disabled: !selected, onSelect: () => selected && zoomToArtboard(selected) },
              "separator",
              { label: "50%", onSelect: () => zoomExact(0.5) },
              { label: "100%", hint: "⌘0", onSelect: () => zoomExact(1) },
              { label: "200%", onSelect: () => zoomExact(2) },
            ]}
          />
          <button type="button" className="icon-button" aria-label="Zoom in" title="Zoom in (+)" onClick={() => zoomStep(ZOOM_STEP)}><Icon name="plus" /></button>
        </div>
        {progress && <div className="canvas-progress-dock"><CanvasProgress key={progress.toolCallId} call={progress} board={board} compact /></div>}
        <div className={`canvas-arrivals-dock${progress ? " above-progress" : ""}`}>
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
          <p className="canvas-hint" role="status">Playing “{board.artboards.find((item) => item.id === entered)?.name}”. Press <kbd>Esc</kbd> to stop.</p>
        ) : selectedArtboard ? (
          <p className="canvas-hint">
            <strong>{selectedArtboard.name}</strong>
            <span>{selectedArtboard.width}×{selectedArtboard.height ?? "auto"}{selectedArtboard.group ? ` · ${selectedArtboard.group}` : ""}</span>
            {selectedArtboard.notes && <span>{selectedArtboard.notes}</span>}
            <span className="canvas-hint-keys"><kbd>↩</kbd> play</span>
          </p>
        ) : null}
      </div>
      {presenting && <CanvasPresent title={board.title} artboards={presented} startId={selected} onClose={endPresent} />}
    </section>
  );
}
