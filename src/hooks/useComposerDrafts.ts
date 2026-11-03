import { useState, useSyncExternalStore, type SetStateAction } from "react";
import type { AttachmentState } from "../attachment-utils";
import type { ChatRefRecord } from "../types";

export interface ComposerDraft extends AttachmentState {
  text: string;
  /** The `@` chats this draft references; the id rides the message as a tail section at send. */
  chatRefs?: ChatRefRecord[];
}

export const EMPTY_DRAFT: ComposerDraft = { text: "", images: [], files: [] };

export interface ComposerDraftState {
  key: string;
  value: ComposerDraft;
  update: (next: SetStateAction<ComposerDraft>) => void;
  /** App-owned drafts notify only their mounted composer. Plain controlled drafts may omit it. */
  subscribe?: (listener: () => void) => () => void;
}

const noSubscription = () => () => {};

/** Subscribe at the editor, so typing never updates App or the transcript. */
export function useComposerDraftValue(state: ComposerDraftState | undefined): ComposerDraft | undefined {
  return useSyncExternalStore(state?.subscribe ?? noSubscription, () => state?.value);
}

/** In-memory drafts live above the composer so hiding/remounting it cannot lose them.
 * Each updater captures its chat key: late sends, file reads and queue restores must
 * write to their originating chat even if selection has moved elsewhere. The owner
 * does not subscribe: a keystroke only wakes the composer for that key. */
export function useComposerDrafts() {
  const [store] = useState(() => {
    const drafts = new Map<string, ComposerDraft>();
    const handles = new Map<string, ComposerDraftState>();
    const listeners = new Map<string, Set<() => void>>();
    const removed = new Set<string>();
    const get = (key: string) => drafts.get(key) ?? EMPTY_DRAFT;
    const notify = (key: string) => listeners.get(key)?.forEach((listener) => listener());
    const update = (key: string, next: SetStateAction<ComposerDraft>) => {
      if (removed.has(key)) return;
      const value = typeof next === "function" ? next(get(key)) : next;
      if (value === get(key)) return;
      drafts.set(key, value);
      notify(key);
    };
    const remove = (key: string) => {
      // A pending async operation must not recreate a deleted chat's attachments.
      removed.add(key);
      handles.delete(key);
      if (drafts.delete(key)) notify(key);
    };
    const forChat = (key: string): ComposerDraftState => {
      let handle = handles.get(key);
      if (!handle) {
        handle = {
          key, get value() { return get(key); }, update: (next) => update(key, next),
          subscribe: (listener) => {
            let group = listeners.get(key);
            if (!group) { group = new Set(); listeners.set(key, group); }
            group.add(listener);
            return () => { group.delete(listener); if (!group.size) listeners.delete(key); };
          }
        };
        handles.set(key, handle);
      }
      return handle;
    };
    return { get, update, remove, forChat };
  });
  return store;
}
