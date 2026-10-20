import { memo, useEffect, useRef, useState } from "react";
import { useFrameTheme, useStagedVisual } from "../../hooks/useStagedVisual";
import { Icon } from "../Icons";
import { CopyButton } from "../ui/CopyButton";
import type { PlacedArtboard } from "../../canvas-layout";

/** A frame reports its size as soon as it lays out; past this an artboard is not coming. */
const LOAD_TIMEOUT_MS = 6_000;
const MAX_ERRORS = 5;

export type ArtboardMotion = "pop" | "glint" | undefined;

interface CanvasArtboardProps {
  placed: PlacedArtboard;
  name: string;
  notes?: string;
  /** The resolved document (`resolveArtboardHtml`). */
  html: string;
  /** Its height is fixed; otherwise its frame's reported height is the artboard's. */
  fixedHeight: boolean;
  /** Mount the frame. Off-screen artboards wait until they are first seen. */
  mounted: boolean;
  selected: boolean;
  entered: boolean;
  motion: ArtboardMotion;
  onMeasure: (id: string, height: number) => void;
  onSelect: (id: string) => void;
  onZoomTo: (id: string) => void;
  onMention: (id: string) => void;
  /** Esc pressed inside the stepped-into frame. */
  onEscape: () => void;
}

/**
 * One artboard on the board: its label (name, size, actions) above a sandboxed frame staged as
 * an `artboard` document (`visuals.rs`). The frame ignores the pointer until the user steps into
 * the artboard, because a frame swallows the wheel and drag events the board pans with. Its
 * document can't send or open anything; it only reports its size and its script errors.
 */
export const CanvasArtboard = memo(function CanvasArtboard({
  placed, name, notes, html, fixedHeight, mounted, selected, entered, motion, onMeasure, onSelect, onZoomTo, onMention, onEscape,
}: CanvasArtboardProps) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const { src, failed, fail, retry } = useStagedVisual(html, "artboard", mounted);
  const [ready, setReady] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  useFrameTheme(frameRef, src);

  useEffect(() => { setReady(false); setErrors([]); }, [src]);

  useEffect(() => {
    if (!src || ready) return;
    const timer = window.setTimeout(fail, LOAD_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [src, ready]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const frame = frameRef.current;
      // Only this artboard's own frame may talk to it.
      if (!frame || event.source !== frame.contentWindow) return;
      const data = event.data as { type?: unknown; height?: unknown; message?: unknown } | null;
      if (!data || typeof data !== "object") return;
      if (data.type === "wackcode-visual:size" && typeof data.height === "number" && Number.isFinite(data.height)) {
        setReady(true);
        if (!fixedHeight) onMeasure(placed.id, Math.round(data.height));
      } else if (data.type === "wackcode-visual:escape") {
        onEscape();
      } else if (data.type === "wackcode-visual:error" && typeof data.message === "string") {
        const message = data.message.slice(0, 300);
        setErrors((current) => current.length >= MAX_ERRORS ? current : [...current, message]);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [fixedHeight, onMeasure, onEscape, placed.id]);

  const size = `${placed.width}×${fixedHeight ? placed.height : "auto"}`;
  return (
    <div
      className={`canvas-artboard${selected ? " selected" : ""}${entered ? " entered" : ""}${motion ? ` ${motion}` : ""}`}
      style={{ left: placed.x, top: placed.y, width: placed.width, height: placed.height }}
      data-artboard={placed.id}
    >
      <div className="canvas-artboard-label">
        <button
          type="button"
          className="canvas-artboard-name"
          aria-label={`${name}, ${placed.width} by ${fixedHeight ? placed.height : "auto"}${notes ? `. ${notes}` : ""}`}
          aria-pressed={selected}
          title={notes ? `${name} — ${notes}` : name}
          onClick={() => onSelect(placed.id)}
        >
          <span className="canvas-artboard-title">{name}</span>
          <span className="canvas-artboard-size">{size}</span>
          {errors.length > 0 && (
            <span className="canvas-artboard-errors" title={errors.join("\n")}>
              <Icon name="alert" aria-hidden="true" />{errors.length}
            </span>
          )}
        </button>
        <span className="canvas-artboard-actions">
          <button type="button" className="icon-button" aria-label={`Mention “${name}” in chat`} title="Mention in chat" onClick={() => onMention(placed.id)}>
            <Icon name="comment" />
          </button>
          <button type="button" className="icon-button" aria-label={`Zoom to “${name}”`} title="Zoom to" onClick={() => onZoomTo(placed.id)}>
            <Icon name="viewfinder" />
          </button>
          <CopyButton text={html} label={`Copy “${name}” HTML`} />
        </span>
      </div>
      <div className="canvas-artboard-frame">
        {failed ? (
          <div className="canvas-artboard-state">
            <span>This artboard couldn’t be drawn.</span>
            <button type="button" className="secondary-button compact" onClick={retry}>Try again</button>
          </div>
        ) : src ? (
          <iframe
            ref={frameRef}
            className={`canvas-artboard-doc${ready ? " ready" : ""}`}
            src={src}
            title={name}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            tabIndex={entered ? 0 : -1}
            style={{ width: placed.width, height: placed.height }}
          />
        ) : null}
        {!ready && !failed && <span className="canvas-artboard-loading" aria-hidden="true" />}
      </div>
    </div>
  );
});
