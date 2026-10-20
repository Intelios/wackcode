import { useEffect, useState, type RefObject } from "react";
import { api } from "../api";
import { visualThemeVars, visualVariables } from "../theme";
import type { VisualKind, VisualThemeVars } from "../types";

/**
 * Stages model-authored HTML through Rust (`visuals.rs`) and returns the `wcvisual:` URL its
 * sandboxed frame loads: a visual reply's (`VisualCard`) or a canvas artboard's
 * (`CanvasArtboard`). The one `api` call these frames make, because the staged document is the
 * whole surface they wrap. `retry` stages the same document again; nothing is staged while
 * `enabled` is false (an off-screen artboard).
 */
export function useStagedVisual(html: string, kind: VisualKind = "reply", enabled = true) {
  const [src, setSrc] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let mounted = true;
    setSrc(undefined);
    setFailed(false);
    if (!enabled) return;
    api.stageVisual(html, visualThemeVars(), kind)
      .then((url) => { if (mounted) setSrc(url); })
      .catch(() => { if (mounted) setFailed(true); });
    return () => { mounted = false; };
  }, [html, kind, enabled, attempt]);

  return {
    src,
    failed,
    fail: () => setFailed(true),
    retry: () => setAttempt((current) => current + 1),
  };
}

/** A theme change repaints a staged frame in place, keeping whatever its document was doing. */
export function useFrameTheme(frameRef: RefObject<HTMLIFrameElement | null>, src: string | undefined) {
  useEffect(() => {
    if (!src) return;
    const push = () => {
      const theme: VisualThemeVars = visualThemeVars();
      frameRef.current?.contentWindow?.postMessage({ type: "wackcode-visual:theme", vars: visualVariables(theme), scheme: theme.scheme }, "*");
    };
    const observer = new MutationObserver(push);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["style", "data-backdrop"] });
    return () => observer.disconnect();
  }, [frameRef, src]);
}
