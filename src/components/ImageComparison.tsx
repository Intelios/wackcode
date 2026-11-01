/**
 * App-owned pixels and controls, shared by Code and Chat. Clip in viewport coordinates and
 * transform both images together, so zoom/pan never move the divider or misalign the pair.
 * State belongs to this accepted call and survives registry/snapshot updates.
 */
import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { animate, motion, useMotionValue, useReducedMotion } from "motion/react";
import type { ImageComparisonDetails } from "../image-comparisons";
import type { ReplyImageSource } from "../reply-images";
import { Icon } from "./Icons";
import { ReplyImageScope, useReplyImageLoader } from "./ReplyImage";
import { useNavigationDismiss } from "./ui/NavigationScope";

const ZOOM = 2.5;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

interface Pixels { before: string; after: string }
interface PairProps { comparison: ImageComparisonDetails; before: ReplyImageSource; after: ReplyImageSource }

function decodedImage(url: string): Promise<{ url: string; ratio: number }> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => image.naturalWidth && image.naturalHeight
      ? resolve({ url, ratio: image.naturalWidth / image.naturalHeight })
      : reject(new Error("Image is empty."));
    image.onerror = () => reject(new Error("Image could not be loaded."));
    image.src = url;
  });
}

export function ImageComparison({ comparison }: { comparison: ImageComparisonDetails }) {
  const scope = useContext(ReplyImageScope);
  const before = scope?.images.get(comparison.before);
  const after = scope?.images.get(comparison.after);
  if (!before || !after || before.failed || after.failed) return <ComparisonUnavailable title={comparison.title} />;
  // A context edit can replace pixels at the same entry/index. Retire the old originals and
  // any open viewer then; rebuilding an unchanged registry must preserve both.
  const key = JSON.stringify([comparison.before, before.imageId ?? before.thumbnail, comparison.after, after.imageId ?? after.thumbnail]);
  return <ComparisonPair key={key} comparison={comparison} before={before} after={after} />;
}

function ComparisonUnavailable({ title }: { title: string }) {
  return <div className="image-comparison-unavailable" role="img" aria-label={`Image comparison unavailable: ${title}`}><Icon name="image" /><span>Image comparison unavailable</span></div>;
}

function ComparisonPair({ comparison, before, after }: PairProps) {
  const root = useRef<HTMLElement>(null);
  const loadBefore = useReplyImageLoader(before);
  const loadAfter = useReplyImageLoader(after);
  const [visible, setVisible] = useState(false);
  const [originals, setOriginals] = useState<Pixels>();
  const [degraded, setDegraded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ratio, setRatio] = useState(16 / 9);
  const [position, setPosition] = useState(50);
  const [expanded, setExpanded] = useState(false);
  const close = useCallback(() => setExpanded(false), []);

  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") { setVisible(true); return; }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) { setVisible(true); observer.disconnect(); }
    });
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!visible) return;
    let live = true;
    void Promise.all([loadBefore(), loadAfter()]).then(async ([left, right]) => {
      if (!left || !right) throw new Error("Saved originals are unavailable.");
      const [beforeImage, afterImage] = await Promise.all([decodedImage(left), decodedImage(right)]);
      if (live) {
        setOriginals({ before: beforeImage.url, after: afterImage.url });
        setRatio(afterImage.ratio);
      }
    }).catch(() => { if (live) setDegraded(true); });
    return () => { live = false; };
  }, [visible, loadBefore, loadAfter]);

  if (failed) return <ComparisonUnavailable title={comparison.title} />;
  const pixels = originals ?? (before.thumbnail && after.thumbnail ? { before: before.thumbnail, after: after.thumbnail } : undefined);
  const controls = { comparison, position, onPosition: setPosition, onRatio: setRatio, onError: () => setFailed(true) };
  const quality = degraded && pixels && <div className="image-comparison-quality" role="status" title="The full-size images could not be loaded. Showing saved previews.">Preview quality</div>;
  return (
    <figure ref={root} className="image-comparison" aria-label={`Image comparison: ${comparison.title}`} style={{ "--comparison-ratio": ratio } as CSSProperties}>
      <div inert={expanded}>
        {pixels ? <ComparisonStage {...controls} pixels={pixels} onExpand={() => { setVisible(true); setExpanded(true); }} />
          : <div className="image-comparison-pending" role="status"><Icon name="image" />Preparing comparison…</div>}
        {quality}
      </div>
      {expanded && pixels && <ComparisonOverlay title={comparison.title} ratio={ratio} onClose={close}><ComparisonStage {...controls} pixels={pixels} expanded onClose={close} />{quality}</ComparisonOverlay>}
    </figure>
  );
}

interface StageProps {
  comparison: ImageComparisonDetails;
  pixels: Pixels;
  position: number;
  onPosition: (position: number) => void;
  onRatio: (ratio: number) => void;
  onError: () => void;
  onExpand?: () => void;
  expanded?: boolean;
  onClose?: () => void;
}

function ComparisonStage({ comparison, pixels, position, onPosition, onRatio, onError, onExpand, expanded = false, onClose }: StageProps) {
  const reduce = useReducedMotion();
  const stage = useRef<HTMLDivElement>(null);
  const dividing = useRef<number | null>(null);
  const press = useRef<{ id: number; x: number; y: number; fromX: number; fromY: number; moved: boolean } | null>(null);
  const panned = useRef(false);
  const [zoomed, setZoomed] = useState(false);
  const [panning, setPanning] = useState(false);
  const scale = useMotionValue(1);
  const x = useMotionValue(0);
  const y = useMotionValue(0);

  const glide = useCallback((target: { scale: number; x: number; y: number }) => {
    const transition = reduce ? { duration: 0 } : { type: "spring" as const, stiffness: 260, damping: 30 };
    animate(scale, target.scale, transition);
    animate(x, target.x, transition);
    animate(y, target.y, transition);
  }, [reduce, scale, x, y]);

  const zoomOut = useCallback(() => {
    setZoomed(false);
    glide({ scale: 1, x: 0, y: 0 });
  }, [glide]);

  const zoomTo = useCallback((px = 0.5, py = 0.5) => {
    const rect = stage.current?.getBoundingClientRect();
    const w = rect?.width ?? 0;
    const h = rect?.height ?? 0;
    setZoomed(true);
    glide({ scale: ZOOM, x: clamp(ZOOM * (0.5 - px) * w, -(ZOOM - 1) * w / 2, (ZOOM - 1) * w / 2), y: clamp(ZOOM * (0.5 - py) * h, -(ZOOM - 1) * h / 2, (ZOOM - 1) * h / 2) });
  }, [glide]);

  useEffect(() => {
    if (!expanded) return;
    const onKey = (event: KeyboardEvent) => {
      if (["+", "="].includes(event.key)) { event.preventDefault(); zoomTo(); }
      else if (["-", "_", "0"].includes(event.key)) { event.preventDefault(); zoomOut(); }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [expanded, zoomTo, zoomOut]);

  const divideAt = (clientX: number) => {
    const rect = stage.current?.getBoundingClientRect();
    if (rect && rect.width > 0) onPosition(clamp(Math.round((clientX - rect.left) / rect.width * 100), 0, 100));
  };
  const endPan = (event: PointerEvent<HTMLDivElement>) => {
    if (press.current?.id !== event.pointerId) return;
    panned.current = press.current.moved;
    press.current = null;
    setPanning(false);
  };

  return (
    <div className={`image-comparison-stage${expanded ? " expanded" : ""}${zoomed ? " zoomed" : ""}${panning ? " panning" : ""}`} ref={stage}
      onPointerDown={(event) => {
        if (!expanded || !zoomed || event.button !== 0 || (event.target as Element).closest("button, [role='slider']")) return;
        event.preventDefault();
        x.stop(); y.stop();
        press.current = { id: event.pointerId, x: event.clientX, y: event.clientY, fromX: x.get(), fromY: y.get(), moved: false };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={(event) => {
        const start = press.current;
        if (!start || start.id !== event.pointerId) return;
        const dx = event.clientX - start.x;
        const dy = event.clientY - start.y;
        if (!start.moved && dx * dx + dy * dy < 9) return;
        start.moved = true;
        setPanning(true);
        const maxX = (event.currentTarget.clientWidth * (scale.get() - 1)) / 2;
        const maxY = (event.currentTarget.clientHeight * (scale.get() - 1)) / 2;
        x.set(clamp(start.fromX + dx, -maxX, maxX));
        y.set(clamp(start.fromY + dy, -maxY, maxY));
      }}
      onPointerUp={endPan} onPointerCancel={endPan} onLostPointerCapture={endPan}
      onClick={(event) => {
        if (!expanded || (event.target as Element).closest("button, [role='slider']")) return;
        if (panned.current) { panned.current = false; return; }
        if (zoomed) zoomOut();
        else {
          const rect = event.currentTarget.getBoundingClientRect();
          zoomTo(rect.width ? (event.clientX - rect.left) / rect.width : 0.5, rect.height ? (event.clientY - rect.top) / rect.height : 0.5);
        }
      }}>
      <div className="image-comparison-layer" style={{ clipPath: `inset(0 0 0 ${position}%)` }}>
        <motion.img src={pixels.after} alt={comparison.afterLabel} draggable={false} style={{ x, y, scale }} onError={onError}
          onLoad={(event) => { const image = event.currentTarget; if (image.naturalWidth && image.naturalHeight) onRatio(image.naturalWidth / image.naturalHeight); }} />
        <span className="image-comparison-label after">{comparison.afterLabel}</span>
      </div>
      <div className="image-comparison-layer" style={{ clipPath: `inset(0 ${100 - position}% 0 0)` }}>
        <motion.img src={pixels.before} alt={comparison.beforeLabel} draggable={false} style={{ x, y, scale }} onError={onError} />
        <span className="image-comparison-label before">{comparison.beforeLabel}</span>
      </div>
      <div className="image-comparison-divider" role="slider" tabIndex={0} aria-label={`${comparison.title}: comparison slider`}
        aria-valuemin={0} aria-valuemax={100} aria-valuenow={position} aria-orientation="horizontal"
        aria-valuetext={`${comparison.beforeLabel} ${position}%, ${comparison.afterLabel} ${100 - position}%`} style={{ left: `${position}%` }}
        onKeyDown={(event) => {
          const next = event.key === "Home" ? 0 : event.key === "End" ? 100 : ["ArrowLeft", "ArrowDown"].includes(event.key) ? position - 1 : ["ArrowRight", "ArrowUp"].includes(event.key) ? position + 1 : undefined;
          if (next !== undefined) { event.preventDefault(); event.stopPropagation(); onPosition(clamp(next, 0, 100)); }
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault(); event.stopPropagation();
          event.currentTarget.focus();
          dividing.current = event.pointerId;
          event.currentTarget.setPointerCapture?.(event.pointerId);
          divideAt(event.clientX);
        }}
        onPointerMove={(event) => { if (dividing.current === event.pointerId) { event.stopPropagation(); divideAt(event.clientX); } }}
        onPointerUp={() => { dividing.current = null; }} onPointerCancel={() => { dividing.current = null; }} onLostPointerCapture={() => { dividing.current = null; }}>
        <span className="image-comparison-grip" aria-hidden="true"><Icon name="back" /><Icon name="chevron" /></span>
      </div>
      <div className="image-comparison-actions">
        {onExpand && <button type="button" className="icon-button" aria-label="Expand image comparison" onClick={onExpand}><Icon name="expand" /></button>}
        {onClose && <button type="button" className="icon-button" aria-label="Close image comparison" onClick={onClose}><Icon name="close" /></button>}
      </div>
      {expanded && <div className="image-comparison-zoom">
        <button type="button" className="icon-button" aria-label="Zoom out" disabled={!zoomed} onClick={zoomOut}><Icon name="minus" /></button>
        <button type="button" className="secondary-button" onClick={zoomOut}>Fit</button>
        <button type="button" className="icon-button" aria-label="Zoom in" disabled={zoomed} onClick={() => zoomTo()}><Icon name="plus" /></button>
      </div>}
    </div>
  );
}

function ComparisonOverlay({ title, ratio, onClose, children }: { title: string; ratio: number; onClose: () => void; children: ReactNode }) {
  const reduce = useReducedMotion();
  const dialog = useRef<HTMLDivElement>(null);
  useNavigationDismiss(onClose);
  useLayoutEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.querySelector<HTMLElement>("[aria-label='Close image comparison']")?.focus();
    return () => { queueMicrotask(() => { if (previous?.isConnected && !previous.closest("[inert]")) previous.focus({ preventScroll: true }); }); };
  }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); }
      if (event.key !== "Tab") return;
      const controls = Array.from(dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled), [role='slider']") ?? []);
      const first = controls[0];
      const last = controls.at(-1);
      if (!dialog.current?.contains(document.activeElement) || (event.shiftKey && document.activeElement === first) || (!event.shiftKey && document.activeElement === last)) {
        event.preventDefault(); (event.shiftKey ? last : first)?.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  return createPortal(
    <motion.div className="modal-backdrop image-comparison-backdrop" role="presentation" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: reduce ? 0 : 0.18 }}
      onMouseDown={(event) => { if (event.button === 0 && event.target === event.currentTarget) onClose(); }}>
      <motion.div ref={dialog} className="image-comparison-dialog" role="dialog" aria-modal="true" aria-label={`Image comparison: ${title}`} style={{ "--comparison-ratio": ratio } as CSSProperties}
        initial={reduce ? false : { opacity: 0, scale: 0.94, y: 8 }} animate={{ opacity: 1, scale: 1, y: 0 }} transition={{ type: "spring", stiffness: 420, damping: 32 }}>
        {children}
      </motion.div>
    </motion.div>, document.body
  );
}
