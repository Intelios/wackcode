import { useLayoutEffect, useRef } from "react";
import { useReducedMotion } from "motion/react";
import { scrambleText, ULTRA_DECODE_MS } from "../ultra-fx";

interface UltraLabelProps {
  text: string;
  /** Decode once as it mounts, which is the moment something switches into Ultra. */
  decodeOnMount?: boolean;
  className?: string;
}

/**
 * An Ultra label ("Ultra", "Ultra Plan") in the prism, decoding from scrambled glyphs as it
 * arrives and again when its button is hovered or keyboard-focused; at rest it stays still,
 * since it sits beside where you type. The real text never leaves the DOM: while decoding it
 * only turns transparent, so it keeps its width (ModeToggle measures the button) and its
 * accessible name, and an aria-hidden overlay shows the noise. The overlay's text is written
 * straight from the frame loop, never through state. A disabled button and reduced motion
 * never decode.
 */
export function UltraLabel({ text, decodeOnMount = true, className = "" }: UltraLabelProps) {
  const rootRef = useRef<HTMLSpanElement>(null);
  const noiseRef = useRef<HTMLSpanElement>(null);
  const reduced = useReducedMotion() ?? false;

  // Layout effect: the first decode frame replaces the text before it paints, so the label
  // never flashes resolved first.
  useLayoutEffect(() => {
    const root = rootRef.current;
    const noise = noiseRef.current;
    if (!root || !noise || reduced) return;
    const host = root.closest("button");
    let raf = 0;
    let startedAt = 0;
    const finish = () => {
      raf = 0;
      delete root.dataset.decoding;
      noise.textContent = "";
    };
    const frame = (now: number) => {
      const progress = (now - startedAt) / ULTRA_DECODE_MS;
      if (progress >= 1) return finish();
      noise.textContent = scrambleText(text, progress);
      raf = requestAnimationFrame(frame);
    };
    const decode = () => {
      if (raf || host?.disabled) return;
      startedAt = performance.now();
      root.dataset.decoding = "";
      noise.textContent = scrambleText(text, 0);
      raf = requestAnimationFrame(frame);
    };
    const onFocus = () => {
      // A click focuses the button too; only keyboard focus replays it.
      try {
        if (host?.matches(":focus-visible")) decode();
      } catch {
        // An engine without :focus-visible just skips the focus replay.
      }
    };
    if (decodeOnMount) decode();
    host?.addEventListener("pointerenter", decode);
    host?.addEventListener("focusin", onFocus);
    return () => {
      cancelAnimationFrame(raf);
      finish();
      host?.removeEventListener("pointerenter", decode);
      host?.removeEventListener("focusin", onFocus);
    };
  }, [text, decodeOnMount, reduced]);

  return (
    <span ref={rootRef} className={`ultra-label ${className}`}>
      <span className="ultra-label-text">{text}</span>
      <span ref={noiseRef} className="ultra-label-noise" aria-hidden="true" />
    </span>
  );
}
