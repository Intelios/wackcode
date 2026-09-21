import { useEffect, useRef, useState } from "react";

function prefersReducedMotion(): boolean {
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Animates `target` toward full length a few characters per frame so streamed
 * text flows in instead of popping. When `active` is false (or the user prefers
 * reduced motion) the full text is returned immediately.
 */
export function useSmoothText(target: string | undefined, active: boolean): string {
  const text = target ?? "";
  const [shown, setShown] = useState(text);
  const shownLength = useRef(text.length);

  useEffect(() => {
    shownLength.current = Math.min(shownLength.current, text.length);
    if (!active || prefersReducedMotion() || shownLength.current >= text.length) {
      shownLength.current = text.length;
      setShown(text);
      return;
    }
    let frame = 0;
    const step = () => {
      const backlog = text.length - shownLength.current;
      shownLength.current += Math.min(Math.max(2, Math.ceil(backlog / 6)), 64);
      if (shownLength.current >= text.length) {
        shownLength.current = text.length;
        setShown(text);
        return;
      }
      setShown(text.slice(0, shownLength.current));
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [text, active]);

  return shown;
}
