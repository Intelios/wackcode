/** Saved artboard documents stay in the session; only the requested current-branch canvas's leave it. */
import { canvasesFromBranch, type Canvas } from "./builtin/canvas/state.js";

export interface CanvasSavedDocument {
  id: string;
  html?: string;
  css?: string;
}

/** Every artboard on the canvas whose markup or stylesheet currently comes from a saved document
 *  (local images inlined at call time), with those fields. Variants resolve from their base's
 *  saved markup, which the same answer carries. */
export function canvasDocuments(messages: readonly unknown[], canvasId: string): { v: 1; canvas: string; artboards: CanvasSavedDocument[] } {
  const canvas: Canvas | undefined = canvasesFromBranch(messages).get(canvasId);
  if (!canvas) throw new Error("That canvas is no longer in this chat.");
  const artboards = canvas.artboards
    .filter((artboard) => (artboard.savedHtml && artboard.html !== undefined) || (artboard.savedCss && artboard.css !== undefined))
    .map((artboard) => ({
      id: artboard.id,
      ...(artboard.savedHtml && artboard.html !== undefined ? { html: artboard.html } : {}),
      ...(artboard.savedCss && artboard.css !== undefined ? { css: artboard.css } : {}),
    }));
  return { v: 1, canvas: canvas.id, artboards };
}
