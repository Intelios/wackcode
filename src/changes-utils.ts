import type { DiffComment, GitChangeFile, GitDiffHunk, GitDiffLine, GitDiffSection } from "./types";

/** One selectable unit in the file list: a file as seen in one layer (staged or working). */
export interface ChangeEntry {
  file: GitChangeFile;
  section: GitDiffSection;
}

export function changeEntries(files: GitChangeFile[]): ChangeEntry[] {
  return files.flatMap((file) => file.sections.map((section) => ({ file, section })));
}

/** `dir/` dimmed + `name` bright, so rows keep the filename readable at any width. */
export function splitPath(path: string): { dir: string; base: string } {
  const slash = path.lastIndexOf("/");
  return slash === -1 ? { dir: "", base: path } : { dir: path.slice(0, slash + 1), base: path.slice(slash + 1) };
}

/** Human label for a hunk separator: the new-side line range, or the removed range when nothing is added. */
export function hunkLabel(hunk: GitDiffHunk): string {
  let added = 0;
  let removed = 0;
  let context = 0;
  for (const line of hunk.lines) {
    if (line.kind === "addition") added += 1;
    else if (line.kind === "deletion") removed += 1;
    else if (line.kind === "context") context += 1;
  }
  const newCount = context + added;
  if (newCount === 0) {
    const end = hunk.oldStart + removed - 1;
    return removed === 1 ? `Removed line ${hunk.oldStart}` : `Removed lines ${hunk.oldStart}–${end}`;
  }
  const end = hunk.newStart + newCount - 1;
  return newCount === 1 ? `Line ${hunk.newStart}` : `Lines ${hunk.newStart}–${end}`;
}

/** Where a diff line anchors a comment: deletions on the old side, everything else on the new. */
export function lineAnchor(line: GitDiffLine): { side: DiffComment["side"]; line: number } | null {
  const side = line.kind === "deletion" ? "old" : "new";
  const number = side === "old" ? line.oldLine : line.newLine;
  return number === null ? null : { side, line: number };
}

/** A comment is fresh only while its section's revision still matches — the diff moved since. */
export function isFresh(comment: DiffComment, file: GitChangeFile | undefined): boolean {
  return Boolean(file?.sections.some((section) => section.layer === comment.layer && section.revision === comment.revision));
}

function lineKey(side: DiffComment["side"], line: number): string {
  return `${side}:${line}`;
}

/**
 * Comments for one section, partitioned: `inline` keys anchor lines still visible
 * (`"new:42"`, `"old:17"`), `orphans` are fresh comments whose line left the diff.
 */
export function sectionComments(path: string, section: GitDiffSection, comments: DiffComment[]): { inline: Map<string, DiffComment[]>; orphans: DiffComment[] } {
  const visible = new Set<string>();
  for (const hunk of section.hunks) {
    for (const line of hunk.lines) {
      if (line.oldLine !== null) visible.add(lineKey("old", line.oldLine));
      if (line.newLine !== null) visible.add(lineKey("new", line.newLine));
    }
  }
  const inline = new Map<string, DiffComment[]>();
  const orphans: DiffComment[] = [];
  for (const comment of comments) {
    if (comment.path !== path || comment.layer !== section.layer || comment.revision !== section.revision) continue;
    const key = lineKey(comment.side, comment.line);
    if (visible.has(key)) {
      const group = inline.get(key);
      if (group) group.push(comment);
      else inline.set(key, [comment]);
    } else {
      orphans.push(comment);
    }
  }
  return { inline, orphans };
}
