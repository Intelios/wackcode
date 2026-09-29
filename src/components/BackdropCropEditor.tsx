import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { CENTRED, MAX_ZOOM, MIN_ZOOM, cropVariables, isCentred, loadAspect, pan, sanitizeCrop, zoomTo, type Frame, type ImageCrop } from "../backdrop-crop";
import { Icon } from "./Icons";

interface BackdropCropEditorProps {
  imageUrl: string;
  crop: ImageCrop;
  disabled: boolean;
  /** Show a crop live without saving it: mid-drag, mid-slide. */
  onPreview: (crop: ImageCrop) => void;
  /** Save a crop: a drag or slide let go, a key pressed, Reset. */
  onCommit: (crop: ImageCrop) => void;
}

/** The sidebar's width in `styles.css`; the guides draw it to scale. */
const SIDEBAR_WIDTH = 240;
/** The composer's widest size and its gap to the window's bottom edge. */
const COMPOSER = { width: 760, height: 92, bottom: 28, gutter: 24 };
const KEY_STEP = 16;

function windowSize(): Frame {
  return { width: Math.max(window.innerWidth, 1), height: Math.max(window.innerHeight, 1) };
}

/**
 * Settings › Appearance › Backdrop › Position. A miniature of the window with the picture in it:
 * drag to move the picture, slide to zoom. It draws through the same CSS as `Backdrop`, in the
 * window's own shape, so what lines up here lines up in the app. Optional guides outline the
 * sidebar and composer to line things up against.
 */
export function BackdropCropEditor({ imageUrl, crop, disabled, onPreview, onCommit }: BackdropCropEditorProps) {
  const frameRef = useRef<HTMLDivElement>(null);
  const [aspect, setAspect] = useState(1.6);
  const [win, setWin] = useState(windowSize);
  const [guides, setGuides] = useState(true);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ x: number; y: number; from: ImageCrop; latest: ImageCrop } | undefined>(undefined);
  /** A crop shown by the keyboard, saved when the key comes up. */
  const keyed = useRef<ImageCrop | undefined>(undefined);
  const shown = sanitizeCrop(crop);

  useEffect(() => loadAspect(imageUrl, setAspect), [imageUrl]);
  useEffect(() => {
    const onResize = () => setWin(windowSize());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  function frameSize(): Frame {
    const box = frameRef.current?.getBoundingClientRect();
    return { width: box?.width || 1, height: box?.height || 1 };
  }

  function startDrag(event: PointerEvent<HTMLDivElement>): void {
    if (disabled || event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { x: event.clientX, y: event.clientY, from: shown, latest: shown };
    setDragging(true);
  }

  function moveDrag(event: PointerEvent<HTMLDivElement>): void {
    const state = drag.current;
    if (!state) return;
    // From where the drag began, so rounding never accumulates.
    state.latest = pan(state.from, event.clientX - state.x, event.clientY - state.y, frameSize(), aspect);
    onPreview(state.latest);
  }

  function endDrag(): void {
    const state = drag.current;
    drag.current = undefined;
    setDragging(false);
    if (state && state.latest !== state.from) onCommit(state.latest);
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    if (disabled) return;
    const step = event.shiftKey ? KEY_STEP * 4 : KEY_STEP;
    const current = keyed.current ?? shown;
    let next: ImageCrop | undefined;
    // An arrow moves the picture the way the pointer would move it: towards the arrow.
    if (event.key === "ArrowLeft") next = pan(current, -step, 0, frameSize(), aspect);
    else if (event.key === "ArrowRight") next = pan(current, step, 0, frameSize(), aspect);
    else if (event.key === "ArrowUp") next = pan(current, 0, -step, frameSize(), aspect);
    else if (event.key === "ArrowDown") next = pan(current, 0, step, frameSize(), aspect);
    else if (event.key === "+" || event.key === "=") next = zoomTo(current, current.zoom + 10, frameSize(), aspect);
    else if (event.key === "-") next = zoomTo(current, current.zoom - 10, frameSize(), aspect);
    if (!next) return;
    event.preventDefault();
    keyed.current = next;
    onPreview(next);
  }

  function onKeyUp(): void {
    const next = keyed.current;
    keyed.current = undefined;
    if (next) onCommit(next);
  }

  // The zoom slider previews on every step and saves on release, like the other sliders here:
  // React's onChange fires per step, the native change event once on release.
  const sliderRef = useRef<HTMLInputElement>(null);
  const commitZoom = useRef<((zoom: number) => void) | undefined>(undefined);
  commitZoom.current = (zoom) => onCommit(zoomTo(shown, zoom, frameSize(), aspect));
  useEffect(() => {
    const input = sliderRef.current;
    if (!input) return;
    const listener = () => commitZoom.current?.(Number(input.value));
    input.addEventListener("change", listener);
    return () => input.removeEventListener("change", listener);
  }, []);

  const composerWidth = Math.min(COMPOSER.width, win.width - SIDEBAR_WIDTH - COMPOSER.gutter * 2);
  const composerLeft = SIDEBAR_WIDTH + (win.width - SIDEBAR_WIDTH - composerWidth) / 2;
  const percent = (value: number, of: number) => `${(value / of) * 100}%`;
  const frameStyle = {
    ...cropVariables(shown, aspect),
    "--backdrop-image": `url("${imageUrl}")`,
    aspectRatio: `${win.width} / ${win.height}`
  } as CSSProperties;

  return (
    <div className="tool-setting appearance-setting crop-setting">
      <div
        ref={frameRef}
        className={`crop-frame ${dragging ? "dragging" : ""} ${disabled ? "disabled" : ""}`}
        style={frameStyle}
        role="group"
        aria-label="Image position"
        aria-describedby="crop-hint"
        tabIndex={disabled ? -1 : 0}
        onPointerDown={startDrag}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
      >
        <div className="crop-image" />
        {guides && (
          <div className="crop-guides" aria-hidden="true">
            <span className="crop-guide crop-guide-sidebar" style={{ width: percent(SIDEBAR_WIDTH, win.width) }}>Sidebar</span>
            <span
              className="crop-guide crop-guide-composer"
              style={{ left: percent(composerLeft, win.width), width: percent(composerWidth, win.width), bottom: percent(COMPOSER.bottom, win.height), height: percent(COMPOSER.height, win.height) }}
            >
              Composer
            </span>
          </div>
        )}
      </div>
      <div className="crop-controls">
        <div className="tool-setting-text">
          <span className="tool-setting-name">Position</span>
          <span id="crop-hint" className="tool-setting-description">
            Drag the picture to move it, or focus it and use <kbd>←</kbd> <kbd>↑</kbd> <kbd>→</kbd> <kbd>↓</kbd> and <kbd>+</kbd> <kbd>-</kbd>. The frame is your window's shape, so what lines up here lines up in the app.
          </span>
        </div>
        <div className="appearance-slider crop-zoom">
          <Icon name="image" className="crop-zoom-icon" />
          <input
            ref={sliderRef}
            type="range"
            min={MIN_ZOOM}
            max={MAX_ZOOM}
            step={1}
            value={shown.zoom}
            aria-label="Zoom"
            disabled={disabled}
            style={{ "--fill": `${((shown.zoom - MIN_ZOOM) / (MAX_ZOOM - MIN_ZOOM)) * 100}%` } as CSSProperties}
            onChange={(event) => onPreview(zoomTo(shown, Number(event.target.value), frameSize(), aspect))}
          />
          <output>{shown.zoom}%</output>
        </div>
        <div className="row-actions">
          <button type="button" className="secondary-button compact" aria-pressed={guides} onClick={() => setGuides(!guides)}>
            {guides ? "Hide guides" : "Show guides"}
          </button>
          <button type="button" className="secondary-button compact" disabled={disabled || isCentred(shown)} onClick={() => onCommit(CENTRED)}>
            Reset position
          </button>
        </div>
      </div>
    </div>
  );
}
