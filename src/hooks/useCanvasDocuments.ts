import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { dependsOnSavedDocuments, type CanvasBoard } from "../canvas-state";

export interface CanvasDocumentFields {
  id: string;
  html?: string;
  css?: string;
}

interface Fetched {
  canvasId: string;
  fields: ReadonlyMap<string, CanvasDocumentFields>;
}

const EMPTY: ReadonlySet<string> = new Set();

/** Overlay the fetched bytes onto the artboards whose flags say they came from saved documents. */
function overlayDocuments(board: CanvasBoard, fields: ReadonlyMap<string, CanvasDocumentFields>): CanvasBoard {
  let changed = false;
  const artboards = board.artboards.map((artboard) => {
    const saved = fields.get(artboard.id);
    if (!saved) return artboard;
    const html = artboard.savedHtml && typeof saved.html === "string" && saved.html !== artboard.html ? saved.html : undefined;
    const css = artboard.savedCss && typeof saved.css === "string" && saved.css !== artboard.css ? saved.css : undefined;
    if (html === undefined && css === undefined) return artboard;
    changed = true;
    return { ...artboard, ...(html !== undefined ? { html } : {}), ...(css !== undefined ? { css } : {}) };
  });
  return changed ? { ...board, artboards } : board;
}

/**
 * A canvas whose artboards carry saved documents (local images inlined at call time) fetches
 * the current bytes from its chat on demand — snapshots never carry them (`canvas_documents`).
 * The returned board has the fetched markup overlaid; `unloaded` names the artboards still
 * waiting on their first fetch, and `failed` says it didn't come. While a refetch is in flight
 * the last bytes keep serving, so a revision doesn't blank the board.
 */
export function useCanvasDocuments(taskId: string, board: CanvasBoard) {
  const depends = useMemo(
    () => new Set(board.artboards.filter((artboard) => dependsOnSavedDocuments(board, artboard)).map((artboard) => artboard.id)),
    [board],
  );
  const [fetched, setFetched] = useState<Fetched>();
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!depends.size) return;
    let mounted = true;
    setFailed(false);
    api.canvasDocuments(taskId, board.id)
      .then((result) => {
        if (!mounted) return;
        const fields = new Map<string, CanvasDocumentFields>();
        for (const entry of result.artboards) {
          if (!entry || typeof entry !== "object" || typeof entry.id !== "string") continue;
          fields.set(entry.id, entry);
        }
        setFetched({ canvasId: board.id, fields });
      })
      .catch(() => { if (mounted) setFailed(true); });
    return () => { mounted = false; };
  }, [taskId, board, depends.size, attempt]);

  const loaded = fetched && fetched.canvasId === board.id ? fetched : undefined;
  const shown = useMemo(() => (loaded ? overlayDocuments(board, loaded.fields) : board), [board, loaded]);
  return {
    board: shown,
    unloaded: loaded || !depends.size ? EMPTY : depends,
    failed: failed && !loaded,
    retry: () => setAttempt((value) => value + 1),
  };
}
