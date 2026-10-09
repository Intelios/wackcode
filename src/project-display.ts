import type { ProjectRecord } from "./types";

/**
 * Shared project ordering and display rules: pins, shortened paths, tile letters and search.
 * Kept out of the component so they're unit-testable.
 */

/** "/Users/jack/Documents/GitHub/wackcode" -> "~/Documents/GitHub/wackcode". A path with more
 *  than four folders keeps its root and the last two: "~/…/GitHub/wackcode". macOS-only by
 *  design (see AGENTS.md), so the home folder is recognised by shape, with no IPC. */
export function shortPath(path: string): string {
  const tilded = path.replace(/^\/Users\/[^/]+(?=\/|$)/, "~");
  const trimmed = tilded.length > 1 ? tilded.replace(/\/+$/, "") : tilded;
  const parts = trimmed.split("/");
  const segments = parts.filter(Boolean);
  if (segments.length <= 4) return trimmed;
  const root = parts[0] === "~" ? "~/" : "/";
  return `${root}…/${segments.slice(-2).join("/")}`;
}

/** The tile letter: the first letter or digit of the name, uppercased. Null when there is none
 *  (an emoji-only name), and the tile then shows a folder icon instead. */
export function monogram(name: string): string | null {
  const letter = Array.from(name).find((character) => /[\p{L}\p{N}]/u.test(character));
  return letter ? letter.toUpperCase() : null;
}

/** Case-insensitive match on the name or the full path. An empty needle matches everything. */
export function matchesProject(project: ProjectRecord, needle: string): boolean {
  const query = needle.trim().toLowerCase();
  if (!query) return true;
  return project.name.toLowerCase().includes(query) || project.path.toLowerCase().includes(query);
}

/** Project ids pinned to the top of the sidebar and project picker. */
export const PINNED_PROJECTS_KEY = "wackcode:pinnedProjects";

/** Pinned projects first, each group in its existing order. */
export function orderProjects(projects: ProjectRecord[], pinned: ReadonlySet<string>): ProjectRecord[] {
  return [...projects.filter((project) => pinned.has(project.id)), ...projects.filter((project) => !pinned.has(project.id))];
}
