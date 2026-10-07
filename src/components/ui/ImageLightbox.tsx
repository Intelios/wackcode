import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { animate, motion, useMotionValue, useReducedMotion } from "motion/react";
import { Icon } from "../Icons";

interface Props {
  /** Shown at once (a small preview), and kept if the original can't be loaded. */
  preview: string;
  /** Resolves to the full-size image's URL, or undefined when only the preview exists. */
  load?: () => Promise<string | undefined>;
  alt: string;
  onClose: () => void;
}

/** One click zooms this far in. */
const ZOOM = 2.5;
/** Pointer travel (px) past which a press becomes a pan instead of a click. */
const PAN_SLOP = 3;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/**
 * A screenshot at full size over the app. It grows out of the preview while the original loads,
 * then sharpens into it. Escape or a click outside closes it. Clicking the image zooms ZOOM×
 * around the clicked point (click again, or −, to zoom back out; + zooms to centre); while
 * zoomed it pans by dragging.
 */
export function ImageLightbox({ preview, load, alt, onClose }: Props) {
  const reduce = useReducedMotion();
  const [full, setFull] = useState<string>();
  const [zoomed, setZoomed] = useState(false);
  const [panning, setPanning] = useState(false);
  const scale = useMotionValue(1);
  const x = useMotionValue(0);
  const y = useMotionValue(0);
  const imgRef = useRef<HTMLImageElement>(null);
  // The press in progress: where it started, the offsets it started from, and whether it grew into a pan.
  const pressRef = useRef<{ startX: number; startY: number; fromX: number; fromY: number; panning: boolean } | null>(null);
  // Set when a pan ends, so the click the browser fires right after doesn't also toggle the zoom.
  const pannedRef = useRef(false);

  useEffect(() => {
    let live = true;
    void load?.().then((url) => { if (live && url) setFull(url); }).catch(() => undefined);
    return () => { live = false; };
  }, [load]);

  const glide = useCallback((target: { scale: number; x: number; y: number }) => {
    const transition = reduce ? { duration: 0 } : { type: "spring" as const, stiffness: 260, damping: 30 };
    animate(scale, target.scale, transition);
    animate(x, target.x, transition);
    animate(y, target.y, transition);
  }, [reduce, scale, x, y]);

  /** Zooms so the point at [px, py] (0..1 across the image) lands centred, clamped to the image. */
  const zoomTo = useCallback((px: number, py: number) => {
    const rect = imgRef.current?.getBoundingClientRect();
    const w = rect?.width ?? 0;
    const h = rect?.height ?? 0;
    setZoomed(true);
    glide({
      scale: ZOOM,
      x: clamp(ZOOM * (0.5 - px) * w, -(ZOOM - 1) * w / 2, (ZOOM - 1) * w / 2),
      y: clamp(ZOOM * (0.5 - py) * h, -(ZOOM - 1) * h / 2, (ZOOM - 1) * h / 2),
    });
  }, [glide]);

  const zoomOut = useCallback(() => {
    setZoomed(false);
    glide({ scale: 1, x: 0, y: 0 });
  }, [glide]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      else if (event.key === "+" || event.key === "=") { if (!zoomed) zoomTo(0.5, 0.5); }
      else if ((event.key === "-" || event.key === "_") && zoomed) zoomOut();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose, zoomed, zoomTo, zoomOut]);

  useEffect(() => {
    // How far the offsets may stray from centre at the current scale. Reads only refs and motion
    // values, so the first-render closure stays correct for the listener's whole life.
    const bounds = () => {
      const w = imgRef.current?.offsetWidth ?? 0;
      const h = imgRef.current?.offsetHeight ?? 0;
      const s = scale.get();
      return { maxX: (w * (s - 1)) / 2, maxY: (h * (s - 1)) / 2 };
    };
    const onPointerMove = (event: PointerEvent) => {
      const press = pressRef.current;
      if (!press) return;
      const dx = event.clientX - press.startX;
      const dy = event.clientY - press.startY;
      if (!press.panning) {
        if (dx * dx + dy * dy < PAN_SLOP * PAN_SLOP) return;
        press.panning = true;
        setPanning(true);
      }
      const { maxX, maxY } = bounds();
      x.set(clamp(press.fromX + dx, -maxX, maxX));
      y.set(clamp(press.fromY + dy, -maxY, maxY));
    };
    const onPointerUp = () => {
      if (pressRef.current?.panning) pannedRef.current = true;
      pressRef.current = null;
      setPanning(false);
    };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };
  }, [scale, x, y]);

  const onPointerDown = (event: React.PointerEvent) => {
    if (event.button !== 0) return;
    // A press takes over from any zoom animation still settling.
    x.stop();
    y.stop();
    scale.stop();
    pressRef.current = { startX: event.clientX, startY: event.clientY, fromX: x.get(), fromY: y.get(), panning: false };
  };

  const onImageClick = (event: React.MouseEvent) => {
    if (pannedRef.current) { pannedRef.current = false; return; }
    if (zoomed) { zoomOut(); return; }
    const rect = imgRef.current?.getBoundingClientRect();
    zoomTo(
      rect && rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0.5,
      rect && rect.height > 0 ? (event.clientY - rect.top) / rect.height : 0.5,
    );
  };

  return createPortal(
    <motion.div
      className="modal-backdrop image-lightbox"
      role="presentation"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: reduce ? 0 : 0.18 }}
      onMouseDown={(event) => event.button === 0 && event.target === event.currentTarget && onClose()}
    >
      <motion.figure
        className={`image-lightbox-figure${zoomed ? " zoomed" : ""}${panning ? " panning" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={alt}
        initial={reduce ? false : { scale: 0.92, opacity: 0, y: 8 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        transition={{ type: "spring", stiffness: 420, damping: 32 }}
      >
        <motion.img
          ref={imgRef}
          className={full ? "loaded" : "preview"}
          src={full ?? preview}
          alt={alt}
          draggable={false}
          style={{ x, y, scale }}
          onPointerDown={onPointerDown}
          onClick={onImageClick}
        />
        <button type="button" className="image-lightbox-close" aria-label="Close" onClick={onClose} autoFocus>
          <Icon name="close" />
        </button>
      </motion.figure>
    </motion.div>,
    document.body
  );
}
