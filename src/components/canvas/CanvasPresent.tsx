import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useFrameTheme, useStagedVisual } from "../../hooks/useStagedVisual";
import { Icon } from "../Icons";

export interface PresentedArtboard {
  id: string;
  name: string;
  group?: string;
  width: number;
  height: number;
  /** Its height is fixed; otherwise its frame's reported height wins. */
  fixedHeight: boolean;
  html: string;
}

interface Props {
  title: string;
  /** In board order: rows top to bottom, each left to right. */
  artboards: PresentedArtboard[];
  startId?: string;
  onClose: (lastId: string | undefined) => void;
}

/** Room around the artboard: the bar below and a margin on every side. */
const MARGIN = 40;
const BAR = 64;

/**
 * Present: the board's artboards one at a time over the whole window, each live (it takes the
 * pointer and keyboard like a played artboard), scaled to fit and never past life size.
 * ←/→ step through them while focus is outside the frame; Esc anywhere leaves, including from
 * inside a frame (it posts `wackcode-visual:escape`). It portals to the body, because the
 * panel's sliding page is transformed and would trap a fixed overlay.
 */
export function CanvasPresent({ title, artboards, startId, onClose }: Props) {
  const reduce = useReducedMotion() ?? false;
  const [index, setIndex] = useState(() => Math.max(0, artboards.findIndex((item) => item.id === startId)));
  const [direction, setDirection] = useState(1);
  const root = useRef<HTMLDivElement>(null);
  const [room, setRoom] = useState({ width: window.innerWidth, height: window.innerHeight });
  const current = artboards[Math.min(index, artboards.length - 1)];
  const close = useRef(onClose);
  close.current = onClose;
  const currentId = useRef(current?.id);
  currentId.current = current?.id;

  const step = (by: number) => {
    if (artboards.length < 2) return;
    setDirection(by);
    setIndex((value) => (value + by + artboards.length) % artboards.length);
    root.current?.focus({ preventScroll: true });
  };

  useLayoutEffect(() => {
    const measure = () => setRoom({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", measure);
    root.current?.focus({ preventScroll: true });
    return () => window.removeEventListener("resize", measure);
  }, []);

  useEffect(() => {
    if (!current) close.current(undefined);
  }, [current]);

  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      event.stopPropagation();
      close.current(currentId.current);
    };
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, []);

  if (!current) return null;
  const scale = Math.min(1, (room.width - MARGIN * 2) / current.width, (room.height - MARGIN * 2 - BAR) / current.height);
  return createPortal(
    <motion.div
      ref={root}
      className="canvas-present"
      role="dialog"
      aria-modal="true"
      aria-label={`Presenting ${title}`}
      tabIndex={-1}
      initial={reduce ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: reduce ? 0 : 0.2 }}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight" || event.key === "ArrowDown" || event.key === " ") { event.preventDefault(); step(1); }
        else if (event.key === "ArrowLeft" || event.key === "ArrowUp") { event.preventDefault(); step(-1); }
      }}
    >
      <div className="canvas-present-stage">
        <AnimatePresence mode="popLayout" initial={false} custom={direction}>
          <motion.div
            key={current.id}
            className="canvas-present-slide"
            custom={direction}
            initial={reduce ? false : { opacity: 0, x: 40 * direction, scale: 0.98 }}
            animate={{ opacity: 1, x: 0, scale: 1 }}
            exit={reduce ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, x: -30 * direction, scale: 0.98, transition: { duration: 0.18 } }}
            transition={{ type: "spring", stiffness: 380, damping: 36 }}
            style={{ width: current.width * scale, height: current.height * scale }}
          >
            <PresentFrame artboard={current} scale={scale} onEscape={() => close.current(currentId.current)} />
          </motion.div>
        </AnimatePresence>
      </div>
      <div className="canvas-present-bar" role="toolbar" aria-label="Presentation">
        <button type="button" className="icon-button" aria-label="Previous artboard" title="Previous (←)" disabled={artboards.length < 2} onClick={() => step(-1)}>
          <Icon name="back" />
        </button>
        <span className="canvas-present-caption" aria-live="polite">
          <strong>{current.name}</strong>
          {current.group && <span>{current.group}</span>}
          <span className="canvas-present-count">{index + 1} / {artboards.length}</span>
        </span>
        <button type="button" className="icon-button" aria-label="Next artboard" title="Next (→)" disabled={artboards.length < 2} onClick={() => step(1)}>
          <Icon name="chevron" />
        </button>
        <span className="canvas-present-divider" aria-hidden="true" />
        <button type="button" className="secondary-button compact" onClick={() => close.current(currentId.current)}>
          Exit <kbd>esc</kbd>
        </button>
      </div>
    </motion.div>,
    document.body
  );
}

/** One live artboard document at its real size, scaled to the slide. */
function PresentFrame({ artboard, scale, onEscape }: { artboard: PresentedArtboard; scale: number; onEscape: () => void }) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const { src, failed, retry } = useStagedVisual(artboard.html, "artboard", true);
  const [ready, setReady] = useState(false);
  useFrameTheme(frameRef, src);
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      // Only this slide's own frame may talk to it.
      if (!frameRef.current || event.source !== frameRef.current.contentWindow) return;
      const data = event.data as { type?: unknown } | null;
      if (data?.type === "wackcode-visual:size") setReady(true);
      else if (data?.type === "wackcode-visual:escape") onEscape();
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [onEscape]);
  return (
    <div className="canvas-present-frame" style={{ width: artboard.width, height: artboard.height, transform: `scale(${scale})` }}>
      {failed ? (
        <div className="canvas-artboard-state">
          <span>This artboard couldn’t be drawn.</span>
          <button type="button" className="secondary-button compact" onClick={retry}>Try again</button>
        </div>
      ) : src ? (
        <iframe ref={frameRef} className={`canvas-artboard-doc${ready ? " ready" : ""}`} src={src} title={artboard.name}
          sandbox="allow-scripts" referrerPolicy="no-referrer" style={{ width: artboard.width, height: artboard.height, pointerEvents: "auto" }} />
      ) : null}
    </div>
  );
}
