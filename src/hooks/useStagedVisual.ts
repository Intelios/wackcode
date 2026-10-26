import { useEffect, useState, type RefObject } from "react";
import { api } from "../api";
import { visualThemeVars, visualVariables } from "../theme";
import type { VisualKind, VisualThemeVars } from "../types";

/** A visual whose persisted document includes local images, absent from transcript snapshots. */
export interface SavedVisualDocument {
  taskId?: string;
  toolCallId?: string;
}

/**
 * Stages model-authored HTML through Rust (`visuals.rs`) for a sandboxed reply or artboard.
 * Saved image documents are fetched on demand from their chat before staging. `retry` loads
 * the document again; disabled, off-screen artboards do not stage anything.
 */
export function useStagedVisual(html: string, kind: VisualKind = "reply", enabled = true, saved?: SavedVisualDocument) {
  const [src, setSrc] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  const { taskId, toolCallId } = saved ?? {};
  const inlined = saved !== undefined;

  useEffect(() => {
    let mounted = true;
    setSrc(undefined);
    setFailed(false);
    if (!enabled) return;
    const document = inlined
      ? taskId && toolCallId ? api.visualDocument(taskId, toolCallId) : Promise.reject(new Error("This visual has no saved document."))
      : Promise.resolve(html);
    document.then((body) => mounted ? api.stageVisual(body, visualThemeVars(), kind) : undefined)
      .then((url) => { if (mounted) setSrc(url); })
      .catch(() => { if (mounted) setFailed(true); });
    return () => { mounted = false; };
  }, [html, kind, enabled, attempt, inlined, taskId, toolCallId]);

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
