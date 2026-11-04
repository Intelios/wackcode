/**
 * Recursive directory watches behind the per-turn caches. Before each run the worker
 * re-read every skill, command and memory note — hundreds of files a turn — to
 * answer "did anything change?". An `fs.watch` on each loaded folder answers the
 * same thing with zero filesystem calls on the prompt path: an event marks the
 * listeners dirty and the next load re-reads once, then goes back to watching.
 *
 * Events are coalesced aggressively: a folder that has seen any change since the
 * last load is treated as dirty wholesale rather than tracked per file. A folder
 * that cannot be watched (missing at watch time, watcher error, non-recursive
 * platform) is marked unwatchable, and any cache touching it reloads every time —
 * the same behavior as before caching existed.
 */
import { watch, type FSWatcher } from "node:fs";

const watchers = new Map<string, FSWatcher | false>();
const listeners = new Map<string, Set<() => void>>();

function ensureWatcher(path: string): boolean {
  const existing = watchers.get(path);
  if (existing !== undefined) return existing !== false;
  try {
    // macOS reports subtree changes through FSEvents; Linux reports top-level
    // files only, where it still catches the flat folders this module watches.
    const watcher = watch(path, { recursive: true }, () => {
      for (const listener of listeners.get(path) ?? []) listener();
    });
    watcher.on("error", () => {
      watcher.close();
      watchers.set(path, false);
    });
    watcher.unref();
    watchers.set(path, watcher);
    return true;
  } catch {
    watchers.set(path, false);
    return false;
  }
}

/**
 * Run `listener` on any change under `path`, starting the folder's shared watcher
 * if needed. Returns whether the folder is watchable: `false` means the listener
 * can never fire, so callers must treat the folder as always dirty.
 */
export function onFsChange(path: string, listener: () => void): boolean {
  let set = listeners.get(path);
  if (!set) {
    set = new Set();
    listeners.set(path, set);
  }
  set.add(listener);
  return ensureWatcher(path);
}
