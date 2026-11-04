/**
 * Cheap (mtime, size) fingerprints behind the per-turn caches. Before each run the
 * worker re-read every skill, command and memory note — hundreds of files a turn —
 * to answer "did anything change?". A stat-only fingerprint answers the same thing:
 * an edit, add, remove or rename under a watched folder moves it, so unchanged
 * content is never re-read.
 *
 * A stamp covers the whole watched set, including entries the reader ignores, so it
 * is conservative by construction: anything that could change a load moves it. The
 * reverse — same mtime, same size, different bytes — stays invisible by design;
 * that is the documented trade-off of (mtime, size) caching.
 */
import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";

/** One file's stamp, or the file's absence: missing and unreadable both stamp "!". */
export function fileStamp(path: string): string {
  try {
    const stats = statSync(path);
    return `${stats.isDirectory() ? "d" : "f"}:${stats.mtimeMs}:${stats.size}`;
  } catch {
    return "!";
  }
}

/**
 * A folder's top level — every entry stamped shallowly — plus any `extraFiles`
 * deeper in the tree that the last load touched. For flat folders (commands,
 * memory notes) this is a complete fingerprint: membership comes from the listing,
 * edits from each entry's mtime. For recursive readers (skills) it covers every
 * file the last load discovered plus any add/remove at the top level; deeper
 * additions are the folder watcher's job (`fs-watch.ts`).
 */
/** The dot-files every loader still reads: ignore rules decide what Pi skips. */
const IGNORE_FILES = new Set([".gitignore", ".ignore", ".fdignore"]);

export function folderStamp(dir: string, extraFiles: string[] = []): string {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return "!";
  }
  entries.sort((a, b) => a.name.localeCompare(b.name));
  const parts: string[] = [];
  for (const entry of entries) {
    // The readers never touch dot-entries (except ignore files) or node_modules,
    // so their state cannot affect a load and stays out of the stamp.
    if (entry.name.startsWith(".") && !IGNORE_FILES.has(entry.name)) continue;
    if (entry.name === "node_modules") continue;
    parts.push(`${entry.name}=${fileStamp(join(dir, entry.name))}`);
  }
  for (const file of extraFiles) parts.push(`~${file}=${fileStamp(file)}`);
  return parts.join(";");
}
