import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ZOOM_STEP, cameraStorageKey, fitBounds, parseCamera, zoomTo, type Camera, type Size } from "../../canvas-camera";
import { boundsOf, layoutCanvas } from "../../canvas-layout";
import { artboardMention, resolveArtboardHtml, type CanvasBoard as Board } from "../../canvas-state";
import { Icon } from "../Icons";
import { MenuButton } from "../ui/MenuButton";
import { CanvasBoard } from "./CanvasBoard";
import type { ArtboardMotion } from "./CanvasArtboard";

const GLIDE_MS = 420;
/** How long a new artboard's pop or a revised one's glint plays. */
const MOTION_MS = 1_400;
/** The fit for one artboard never magnifies past life size. */
const FOCUS_MAX_ZOOM = 1;

export type CanvasActivity = "drawing" | "looking" | undefined;

interface CanvasPanelProps {
  taskId: string;
  board: Board;
  /** Every canvas in this chat, for the switcher. */
  boards: Board[];
  /** The agent is working on this canvas right now. */
  activity: CanvasActivity;
  expanded: boolean;
  /** A transcript card asked to show these artboards; a new nonce is a new request. */
  focus?: { ids: string[]; nonce: number };
  onSelectCanvas: (canvasId: string) => void;
  onExpand: () => void;
  onClose: () => void;
  /** Puts an artboard's plain-text mention in the draft. */
  onMention: (text: string) => void;
}

function signature(board: Board, id: string): string {
  const artboard = board.artboards.find((item) => item.id === id);
  return artboard ? JSON.stringify([artboard.name, artboard.width, artboard.height, artboard.notes, resolveArtboardHtml(board, artboard)]) : "";
}

function savedCamera(taskId: string, canvasId: string): Camera | undefined {
  try {
    const raw = localStorage.getItem(cameraStorageKey(taskId, canvasId));
    return raw ? parseCamera(JSON.parse(raw)) : undefined;
  } catch {
    return undefined;
  }
}

const ease = (t: number) => 1 - (1 - t) ** 3;

/**
 * One canvas in the side panel: the shared panel head (switcher, zoom, Expand, close) over the
 * board. It owns the camera — restored per canvas from `localStorage`, otherwise fitted — and the
 * small choreography around the agent's changes: new artboards pop in, revised ones glint, and
 * the camera glides to them unless the user has panned away since, in which case a pill offers
 * to. Reduced motion drops the glides and the effects; everything still lands.
 */
export function CanvasPanel({ taskId, board, boards, activity, expanded, focus, onSelectCanvas, onExpand, onClose, onMention }: CanvasPanelProps) {
  const reduce = useReducedMotion() ?? false;
  const [camera, setCamera] = useState<Camera>(() => savedCamera(taskId, board.id) ?? { x: 0, y: 0, zoom: 1 });
  const [viewport, setViewport] = useState<Size>({ width: 0, height: 0 });
  const [measured, setMeasured] = useState<Record<string, number>>({});
  const [selected, setSelected] = useState<string>();
  const [entered, setEntered] = useState<string>();
  const [motions, setMotions] = useState<Record<string, ArtboardMotion>>({});
  const [arrivals, setArrivals] = useState<string[]>([]);
  const cameraRef = useRef(camera);
  cameraRef.current = camera;
  const glideFrame = useRef(0);
  /** Follow the agent's changes until the user moves the camera themself. */
  const following = useRef(true);
  const fitted = useRef(savedCamera(taskId, board.id) !== undefined);
  const signatures = useRef<Map<string, string> | undefined>(undefined);

  const layout = useMemo(() => layoutCanvas(board, measured), [board, measured]);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;

  const stopGlide = () => {
    if (glideFrame.current) cancelAnimationFrame(glideFrame.current);
    glideFrame.current = 0;
  };
  useEffect(() => stopGlide, []);

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
  }, [reduce]);

  const showIds = useCallback((ids: readonly string[], maxZoom = FOCUS_MAX_ZOOM) => {
    const bounds = boundsOf(layoutRef.current.artboards.filter((item) => ids.includes(item.id)));
    if (bounds && viewport.width) glide(fitBounds(bounds, viewport, maxZoom));
  }, [glide, viewport]);

  const fit = useCallback(() => {
    following.current = true;
    setArrivals([]);
    if (viewport.width && layoutRef.current.artboards.length) glide(fitBounds(layoutRef.current.bounds, viewport, FOCUS_MAX_ZOOM));
  }, [glide, viewport]);

  // The first sight of a canvas with no saved camera fits it whole.
  useEffect(() => {
    if (fitted.current || !viewport.width || !layout.artboards.length) return;
    fitted.current = true;
    setCamera(fitBounds(layout.bounds, viewport, FOCUS_MAX_ZOOM));
  }, [viewport, layout]);

  // Expanding, restoring or resizing the panel refits the board, until the user takes the camera.
  const fittedViewport = useRef<Size>({ width: 0, height: 0 });
  useEffect(() => {
    const before = fittedViewport.current;
    fittedViewport.current = viewport;
    if (!before.width || !viewport.width || !layoutRef.current.artboards.length || !following.current) return;
    if (Math.abs(before.width - viewport.width) < 40 && Math.abs(before.height - viewport.height) < 40) return;
    glide(fitBounds(layoutRef.current.bounds, viewport, FOCUS_MAX_ZOOM));
  }, [viewport, glide]);

  // Remembered per canvas, written once the camera settles.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      try { localStorage.setItem(cameraStorageKey(taskId, board.id), JSON.stringify(camera)); } catch { /* a convenience only */ }
    }, 300);
    return () => window.clearTimeout(timer);
  }, [camera, taskId, board.id]);

  // What changed since the last board: new artboards pop, revised ones glint.
  useEffect(() => {
    const next = new Map(board.artboards.map((artboard) => [artboard.id, signature(board, artboard.id)]));
    const previous = signatures.current;
    signatures.current = next;
    if (!previous) return;
    const changes: Record<string, ArtboardMotion> = {};
    for (const [id, value] of next) {
      if (!previous.has(id)) changes[id] = "pop";
      else if (previous.get(id) !== value) changes[id] = "glint";
    }
    const ids = Object.keys(changes);
    if (!ids.length) return;
    if (!reduce) setMotions((current) => ({ ...current, ...changes }));
    if (following.current) showIds(ids);
    else setArrivals((current) => [...new Set([...current, ...ids])]);
    if (reduce) return;
    const timer = window.setTimeout(() => setMotions((current) => {
      const rest = { ...current };
      for (const id of ids) delete rest[id];
      return rest;
    }), MOTION_MS);
    return () => window.clearTimeout(timer);
  }, [board]);

  // A card in the transcript asked for these artboards.
  useEffect(() => {
    if (!focus || !viewport.width) return;
    const ids = focus.ids.filter((id) => board.artboards.some((artboard) => artboard.id === id));
    following.current = true;
    setArrivals([]);
    if (ids.length) {
      setSelected(ids[0]);
      showIds(ids);
    } else {
      fit();
    }
  }, [focus?.nonce, viewport.width > 0]);

  // An artboard that left the board can't stay selected or stepped into.
  useEffect(() => {
    if (selected && !board.artboards.some((artboard) => artboard.id === selected)) setSelected(undefined);
    if (entered && !board.artboards.some((artboard) => artboard.id === entered)) setEntered(undefined);
  }, [board, selected, entered]);

  const userCamera = useCallback((next: Camera) => {
    stopGlide();
    following.current = false;
    setCamera(next);
  }, []);
  const onMeasure = useCallback((id: string, height: number) => {
    setMeasured((current) => current[id] === height ? current : { ...current, [id]: height });
  }, []);
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
    following.current = false;
    glide(zoomTo(cameraRef.current, cameraRef.current.zoom * factor, viewport));
  }, [glide, viewport]);
  const zoomExact = useCallback((zoom: number) => {
    following.current = false;
    glide(zoomTo(cameraRef.current, zoom, viewport));
  }, [glide, viewport]);

  const selectedArtboard = selected ? board.artboards.find((item) => item.id === selected) : undefined;
  const percent = Math.round(camera.zoom * 100);
  const count = board.artboards.length;
  return (
    <section className="canvas-panel">
      <header className="panel-header" data-tauri-drag-region>
        <span className={`panel-mark ${activity ? "live" : ""}`} aria-hidden="true"><Icon name="canvas" /></span>
        <div className="panel-title" data-tauri-drag-region>
          <span className="panel-kicker" data-tauri-drag-region>Canvas · {count} artboard{count === 1 ? "" : "s"}</span>
          {boards.length > 1 ? (
            <MenuButton
              className="canvas-switcher"
              label={`Canvas: ${board.title}. Switch canvas`}
              align="start"
              icon={<><h3>{board.title}</h3><Icon name="chevron" /></>}
              items={boards.map((item) => ({ label: item.title, selected: item.id === board.id, onSelect: () => onSelectCanvas(item.id) }))}
            />
          ) : (
            <h3 data-tauri-drag-region>{board.title}</h3>
          )}
        </div>
        {activity && (
          <span className="terminal-pill running" role="status">
            <span className="terminal-pill-dot" aria-hidden="true" />{activity === "looking" ? "Checking artboards" : "Drawing"}
          </span>
        )}
        <div className="panel-header-actions">
          <div className="canvas-zoom" role="group" aria-label="Zoom">
            <button type="button" className="icon-button" aria-label="Zoom out" title="Zoom out (−)" onClick={() => zoomStep(1 / ZOOM_STEP)}><Icon name="minus" /></button>
            <MenuButton
              className="canvas-zoom-level"
              label={`Zoom ${percent}%. Zoom options`}
              icon={<span>{percent}%</span>}
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
          <button type="button" className="icon-button" title={expanded ? "Restore canvas" : "Expand canvas"} aria-label={expanded ? "Restore canvas" : "Expand canvas"} onClick={onExpand}>
            <Icon name={expanded ? "collapse" : "expand"} />
          </button>
          <button type="button" className="icon-button" title="Close canvas panel" aria-label="Close canvas panel" onClick={onClose}><Icon name="close" /></button>
        </div>
      </header>
      <div className="canvas-stage">
        <CanvasBoard
          board={board}
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
                following.current = true;
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
