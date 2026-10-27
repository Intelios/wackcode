import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { ZOOM_STEP, intersects, panBy, zoomAt, type Camera, type Size } from "../../canvas-camera";
import { GROUP_BAND, LABEL_BAND, type CanvasLayout } from "../../canvas-layout";
import { resolveArtboardHtml, type CanvasBoard as Board } from "../../canvas-state";
import type { CanvasTool } from "../../canvas-session";
import { CanvasArtboard, type ArtboardMotion } from "./CanvasArtboard";

/** A press that moves less than this is a click, not a pan. */
const CLICK_SLOP = 4;
/** Off-screen artboards within this many screen pixels mount ahead of a pan. */
const MOUNT_MARGIN = 600;
/** Labels keep their screen size as the board zooms out, down to about 1/12 of life size. */
const MAX_LABEL_SCALE = 12;
/** Below this zoom only the artboards' names stay: sizes, notes and row labels would crowd. */
const FAR_ZOOM = 0.25;

interface CanvasBoardProps {
  board: Board;
  visible: boolean;
  tool: CanvasTool;
  onTool: (tool: CanvasTool) => void;
  layout: CanvasLayout;
  camera: Camera;
  /** `user` for the user's own pans and zooms; the panel stops auto-following after one. */
  onCamera: (camera: Camera, source: "user") => void;
  onViewport: (size: Size) => void;
  selected?: string;
  entered?: string;
  motions: Readonly<Record<string, ArtboardMotion>>;
  onSelect: (id: string | undefined) => void;
  onEnter: (id: string | undefined) => void;
  onZoomTo: (id: string) => void;
  onMeasure: (id: string, height: number) => void;
  onMention: (id: string) => void;
  onExport: (id: string, format: "png" | "html") => void;
  /** Keyboard zoom and fit, so the panel's controls and the keys do the same thing. */
  onFit: () => void;
  onZoomStep: (factor: number) => void;
  onZoomReset: () => void;
}

/** Next artboard for an arrow key: along the row for ←/→, the nearest one in the next row for ↑/↓. */
function neighbour(layout: CanvasLayout, from: string | undefined, key: string): string | undefined {
  const items = layout.artboards;
  if (!items.length) return undefined;
  const current = items.find((item) => item.id === from);
  if (!current) return items[0].id;
  const rows = [...new Set(items.map((item) => item.y))].sort((a, b) => a - b);
  const row = rows.indexOf(current.y);
  const inRow = (y: number) => items.filter((item) => item.y === y).sort((a, b) => a.x - b.x);
  if (key === "ArrowLeft" || key === "ArrowRight") {
    const members = inRow(current.y);
    const index = members.indexOf(current) + (key === "ArrowRight" ? 1 : -1);
    return members[Math.max(0, Math.min(members.length - 1, index))].id;
  }
  const target = rows[row + (key === "ArrowDown" ? 1 : -1)];
  if (target === undefined) return current.id;
  const centre = current.x + current.width / 2;
  return inRow(target).reduce((best, item) =>
    Math.abs(item.x + item.width / 2 - centre) < Math.abs(best.x + best.width / 2 - centre) ? item : best).id;
}

/**
 * The board itself: a world layer under one transform, over the plain well. It fills its panel
 * page and always owns the wheel there: a two-finger scroll pans, a pinch or ⌘-scroll zooms
 * about the pointer.
 *
 * Dragging anywhere pans (artboard frames ignore the pointer until played). With Select, a click
 * picks an artboard and a double-click plays it, so its own hover states and scrolling work;
 * Esc steps back out, then clears the selection. Hand (H, or Space held) only pans. Arrow keys
 * move the selection; Enter plays it; + and − zoom; ⇧1 fits; ⇧2 zooms to the selection; ⌘0 is
 * 100 %; V and H pick the tool.
 */
export function CanvasBoard({
  board, visible, tool, onTool, layout, camera, onCamera, onViewport, selected, entered, motions,
  onSelect, onEnter, onZoomTo, onMeasure, onMention, onExport, onFit, onZoomStep, onZoomReset,
}: CanvasBoardProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  const [panning, setPanning] = useState(false);
  const [hand, setHand] = useState(false);
  const drag = useRef<{ x: number; y: number; camera: Camera; moved: boolean; pointer: number } | undefined>(undefined);
  const live = useRef({ camera, onCamera });
  live.current = { camera, onCamera };
  const seen = useRef(new Set<string>());

  useEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      const next = { width: element.clientWidth, height: element.clientHeight };
      setSize(next);
      onViewport(next);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [onViewport]);

  // React's wheel listener is passive, and the board must stop the page from scrolling.
  useEffect(() => {
    const element = viewportRef.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const { camera: current, onCamera: update } = live.current;
      const rect = element.getBoundingClientRect();
      const point = { x: event.clientX - rect.left, y: event.clientY - rect.top };
      if (event.ctrlKey || event.metaKey) {
        // A trackpad pinch arrives as ctrl+wheel with small deltas; a mouse wheel's are larger.
        update(zoomAt(current, Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.0025)), point), "user");
      } else {
        update(panBy(current, -event.deltaX, -event.deltaY), "user");
      }
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  // Space held anywhere over the board turns every frame back into a pan surface.
  useEffect(() => {
    const up = (event: globalThis.KeyboardEvent) => { if (event.code === "Space") setHand(false); };
    const blur = () => setHand(false);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  const artboardAt = (target: EventTarget | null): string | undefined => {
    const element = target instanceof Element ? target.closest<HTMLElement>("[data-artboard]") : null;
    return element?.dataset.artboard;
  };

  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 && event.button !== 1) return;
    // Buttons in a label act on their own; the board doesn't pan from them.
    if (event.target instanceof Element && event.target.closest("button, a, input")) return;
    drag.current = { x: event.clientX, y: event.clientY, camera, moved: false, pointer: event.pointerId };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    viewportRef.current?.focus({ preventScroll: true });
  };

  const pointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (!start || start.pointer !== event.pointerId) return;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (!start.moved && Math.hypot(dx, dy) < CLICK_SLOP) return;
    if (!start.moved) { start.moved = true; setPanning(true); }
    onCamera(panBy(start.camera, dx, dy), "user");
  };

  const pointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (!start || start.pointer !== event.pointerId) return;
    drag.current = undefined;
    setPanning(false);
    if (start.moved || tool === "hand") return;
    const id = artboardAt(event.target);
    if (entered && id !== entered) onEnter(undefined);
    onSelect(id);
  };

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const key = event.key;
    if (event.code === "Space") { event.preventDefault(); setHand(true); return; }
    if (key.startsWith("Arrow")) {
      event.preventDefault();
      const next = neighbour(layout, selected, key);
      if (next) { onSelect(next); onZoomTo(next); }
    } else if (key === "Enter" && selected) {
      event.preventDefault();
      onEnter(selected);
    } else if (key === "Escape" && (entered || selected)) {
      if (entered) onEnter(undefined);
      else onSelect(undefined);
      event.preventDefault();
      event.stopPropagation();
    } else if ((key === "v" || key === "h") && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      onTool(key === "h" ? "hand" : "select");
    } else if (key === "!" || (event.shiftKey && event.code === "Digit1")) {
      event.preventDefault();
      onFit();
    } else if ((key === "@" || (event.shiftKey && event.code === "Digit2")) && selected) {
      event.preventDefault();
      onZoomTo(selected);
    } else if (key === "+" || key === "=") {
      event.preventDefault();
      onZoomStep(ZOOM_STEP);
    } else if (key === "-" || key === "_") {
      event.preventDefault();
      onZoomStep(1 / ZOOM_STEP);
    } else if (key === "0" && event.metaKey) {
      event.preventDefault();
      onZoomReset();
    }
  };

  const byId = useMemo(() => new Map(board.artboards.map((artboard) => [artboard.id, artboard])), [board]);
  const select = useCallback((id: string) => onSelect(id), [onSelect]);
  const stepOut = useCallback(() => {
    onEnter(undefined);
    viewportRef.current?.focus({ preventScroll: true });
  }, [onEnter]);

  // Frames mount once they come near the screen, then stay mounted so panning back never reloads.
  for (const placed of layout.artboards) {
    if (seen.current.has(placed.id) || !size.width || !visible) continue;
    const near = { x: placed.x - MOUNT_MARGIN / camera.zoom, y: placed.y - MOUNT_MARGIN / camera.zoom, width: placed.width + (MOUNT_MARGIN * 2) / camera.zoom, height: placed.height + (MOUNT_MARGIN * 2) / camera.zoom };
    if (intersects(camera, near, size)) seen.current.add(placed.id);
  }

  return (
    <div
      ref={viewportRef}
      className={`canvas-viewport${panning ? " panning" : ""}${hand || tool === "hand" ? " hand" : ""}${entered ? " has-entered" : ""}${camera.zoom < FAR_ZOOM ? " far" : ""}`}
      role="region"
      aria-label={`Canvas: ${board.title}`}
      aria-roledescription="canvas"
      aria-description="Scroll to pan, pinch to zoom. Arrow keys pick an artboard and Enter plays it."
      tabIndex={0}
      onPointerDown={pointerDown}
      onPointerMove={pointerMove}
      onPointerUp={pointerUp}
      onPointerCancel={() => { drag.current = undefined; setPanning(false); }}
      onDoubleClick={(event) => {
        const id = tool === "select" ? artboardAt(event.target) : undefined;
        if (id) onEnter(id);
      }}
      onKeyDown={keyDown}
    >
      <div
        className="canvas-world"
        style={{
          transform: `translate3d(${camera.x}px, ${camera.y}px, 0) scale(${camera.zoom})`,
          ["--canvas-label-scale" as string]: String(Math.min(1 / camera.zoom, MAX_LABEL_SCALE)),
        }}
      >
        {layout.groups.map((group) => group.label ? (
          // Anchored at the row's frame tops, then lifted in screen pixels above the names there.
          <div key={`group:${group.label}`} className="canvas-group-label" style={{ left: group.x, top: group.y + GROUP_BAND + LABEL_BAND }}>
            <span>{group.label}</span>
          </div>
        ) : null)}
        <ul className="canvas-artboards" aria-label="Artboards">
          {layout.artboards.map((placed) => {
            const artboard = byId.get(placed.id);
            if (!artboard) return null;
            return (
              <li key={placed.id}>
                <CanvasArtboard
                  placed={placed}
                  name={artboard.name}
                  notes={artboard.notes}
                  html={resolveArtboardHtml(board, artboard)}
                  fixedHeight={artboard.height !== undefined}
                  mounted={visible && seen.current.has(placed.id)}
                  selected={selected === placed.id}
                  entered={entered === placed.id}
                  motion={motions[placed.id]}
                  onMeasure={onMeasure}
                  onSelect={select}
                  onZoomTo={onZoomTo}
                  onMention={onMention}
                  onExport={onExport}
                  onPlay={onEnter}
                  onEscape={stepOut}
                />
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
