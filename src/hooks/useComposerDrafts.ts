import { useCallback, useRef, useState, type SetStateAction } from "react";
import type { AttachmentState } from "../attachment-utils";

export interface ComposerDraft extends AttachmentState {
  text: string;
}

export const EMPTY_DRAFT: ComposerDraft = { text: "", images: [], files: [] };

export interface ComposerDraftState {
  key: string;
  value: ComposerDraft;
  update: (next: SetStateAction<ComposerDraft>) => void;
}

/** In-memory drafts live above the composer so hiding/remounting it cannot lose them.
 * Each updater captures its chat key: late sends, file reads and queue restores must
 * write to their originating chat even if selection has moved elsewhere. */
export function useComposerDrafts() {
  const [drafts, setDrafts] = useState(() => new Map<string, ComposerDraft>());
  const latest = useRef(drafts);
  latest.current = drafts;
  const removed = useRef(new Set<string>());
  const get = useCallback((key: string) => latest.current.get(key) ?? EMPTY_DRAFT, []);
  const update = useCallback((key: string, next: SetStateAction<ComposerDraft>) => {
    setDrafts((current) => {
      if (removed.current.has(key)) return current;
      const value = typeof next === "function" ? next(current.get(key) ?? EMPTY_DRAFT) : next;
      if (value === current.get(key)) return current;
      return new Map(current).set(key, value);
    });
  }, []);
  const remove = useCallback((key: string) => {
    // A pending async operation must not recreate a deleted chat's attachments.
    removed.current.add(key);
    setDrafts((current) => {
      if (!current.has(key)) return current;
      const next = new Map(current);
      next.delete(key);
      return next;
    });
  }, []);
  return {
    get, update, remove,
    forChat: (key: string): ComposerDraftState => ({ key, value: get(key), update: (next) => update(key, next) })
  };
}
