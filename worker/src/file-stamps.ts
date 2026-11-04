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
 * One level of a directory: every entry stamped shallowly. Used for flat folders
 * (user commands, memory notes): any entry added, removed, renamed or edited moves it.
 */
export function dirStamp(dir: string): string {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return "!";
  }
  entries.sort((a, b) => a.name.localeCompare(b.name));
  const parts: string[] = [];
  for (const entry of entries) parts.push(`${entry.name}=${fileStamp(join(dir, entry.name))}`);
  return parts.join(";");
}

/**
 * A recursive stamp matching the shape of Pi's `loadSkillsFromDir` discovery:
 * `node_modules` and dot-entries never matter to a load, except the ignore files
 * (`.gitignore`, `.ignore`, `.fdignore`) whose text decides what Pi skips — so they
 * are stamped too. Symlinks follow their target like Pi does. Unlike Pi the walk is
 * stat-only: no file contents are read. Entries Pi would ignore are still stamped,
 * which only ever causes an extra reload, never a missed one.
 */
const IGNORE_FILES = new Set([".gitignore", ".ignore", ".fdignore"]);
const MAX_STAMP_DEPTH = 32;

export function skillsTreeStamp(dir: string, depth = 0): string {
  if (depth > MAX_STAMP_DEPTH) return "!deep";
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return "!";
  }
  entries.sort((a, b) => a.name.localeCompare(b.name));
  const parts: string[] = [];
  for (const entry of entries) {
    if (entry.name === "node_modules") continue;
    if (entry.name.startsWith(".") && !IGNORE_FILES.has(entry.name)) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      parts.push(`${entry.name}/=${skillsTreeStamp(path, depth + 1)}`);
      continue;
    }
    if (entry.isSymbolicLink()) {
      let target;
      try {
        target = statSync(path);
      } catch {
        parts.push(`${entry.name}=!`);
        continue;
      }
      if (target.isDirectory()) {
        parts.push(`${entry.name}/=${skillsTreeStamp(path, depth + 1)}`);
        continue;
      }
    }
    parts.push(`${entry.name}=${fileStamp(path)}`);
  }
  return parts.join(";");
}
