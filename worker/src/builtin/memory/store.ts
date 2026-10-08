/**
 * The memory file format — the one contract shared with `src-tauri/src/memory.rs`, which reads
 * and writes the same files from Settings. One note per `<type>_<slug>.md` file:
 *
 *     ---
 *     type: feedback
 *     title: Run worker tests after protocol changes
 *     description: Protocol edits need pnpm test:worker
 *     modified: 2026-09-28T10:12:00.000Z
 *     ---
 *     Body markdown…
 *
 * A note's *name* is its filename without `.md` — the stable id `memory_recall` and
 * `memory_forget` take, and the one thing that never changes while the note exists. The index
 * below is always generated from the files (there is no MEMORY.md the model must maintain), so
 * it can never drift from what is on disk.
 */

/** In display order: the index lists them grouped by type. */
export const MEMORY_TYPES = ["user", "feedback", "project", "reference"] as const;
export type MemoryType = (typeof MEMORY_TYPES)[number];

export const MAX_TITLE_CHARS = 200;
export const MAX_DESCRIPTION_CHARS = 300;
export const MAX_BODY_CHARS = 50_000;
/** A note name is a filename: the same rule `memory.rs::validate_name` applies in Settings. */
export const MAX_NAME_CHARS = 80;
/** Keeps a runaway project from quietly growing past what a system prompt should carry. */
const INDEX_MAX_ENTRIES = 200;
const INDEX_MAX_CHARS = 25_000;

export interface MemoryNote {
  /** Filename without `.md`, e.g. `feedback_run-worker-tests`. */
  name: string;
  type: MemoryType;
  title: string;
  description: string;
  /** ISO 8601, written at every save so a stale fact can be told from a current one. */
  modified?: string;
  body: string;
}

export function isMemoryType(value: string): value is MemoryType {
  return (MEMORY_TYPES as readonly string[]).includes(value);
}

/** Lowercase letters, digits and single hyphens, like a command name. */
export function slugify(text: string): string {
  const slug = text
    .normalize("NFKD")
    // Strip combining marks left by the decomposition (Ü → U + diaeresis) before punctuation
    // collapses to hyphens, so "déjà" slugs to "deja" rather than "de-ja".
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/g, "");
  return slug || "note";
}

export function memoryName(type: MemoryType, title: string): string {
  return `${type}_${slugify(title)}`;
}

/**
 * Names a note file may carry. `memory_save` takes `name` verbatim from the model and a value
 * with `/`, `..` or a leading dot would land outside the memory folder (or inside `.trash`),
 * so every tool that resolves a name to a path checks this first — like Rust's validation.
 */
export function isMemoryName(name: string): boolean {
  return /^[a-z0-9][a-z0-9_-]{0,79}$/.test(name);
}

/**
 * Mirrors `skills::yaml_scalar` in Rust, so files the two sides write parse identically:
 * plain when safe, otherwise a JSON string (whose escapes YAML also accepts).
 */
export function yamlScalar(value: string): string {
  const plainSafe =
    /[A-Za-z]/.test(value.charAt(0)) &&
    !value.endsWith(" ") && !value.endsWith(":") &&
    !value.includes(": ") && !value.includes(" #") &&
    !/[\0-\x1f\x7f]/.test(value) &&
    !["true", "false", "yes", "no", "on", "off", "null", "y", "n"].includes(value.toLowerCase());
  return plainSafe ? value : JSON.stringify(value);
}

function parseScalar(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    try {
      const parsed = JSON.parse(trimmed) as unknown;
      return typeof parsed === "string" ? parsed : trimmed.slice(1, -1);
    } catch {
      return trimmed.slice(1, -1);
    }
  }
  return trimmed;
}

/** Pi's own frontmatter split (`skills.rs::split` on the Rust side): `---`, keys, `---`, body. */
export function splitFrontmatter(text: string): { frontmatter: Map<string, string>; body: string } {
  const normalized = (text.startsWith("\u{feff}") ? text.slice(1) : text).replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const map = new Map<string, string>();
  if (!normalized.startsWith("---")) return { frontmatter: map, body: normalized.trim() };
  const end = normalized.indexOf("\n---", 3);
  if (end < 0) return { frontmatter: map, body: normalized.trim() };
  for (const line of normalized.slice(4, end).split("\n")) {
    const key = line.match(/^([A-Za-z][\w-]*):\s?(.*)$/);
    if (key) map.set(key[1], parseScalar(key[2]));
  }
  return { frontmatter: map, body: normalized.slice(end + 4).trim() };
}

export function composeMemory(note: Omit<MemoryNote, "name">): string {
  const out = [
    "---",
    `type: ${note.type}`,
    `title: ${yamlScalar(note.title)}`,
    `description: ${yamlScalar(note.description)}`,
    ...(note.modified ? [`modified: ${note.modified}`] : []),
    "---",
    "",
    note.body.trim(),
    ""
  ];
  return out.join("\n");
}

/**
 * One memory file as the tools see it. A file with no frontmatter, a missing title or an
 * unknown type is not a memory (Settings may still show it as a plain file to clean up).
 */
export function parseMemory(name: string, text: string): MemoryNote | undefined {
  const { frontmatter, body } = splitFrontmatter(text);
  const title = frontmatter.get("title")?.trim();
  if (!title) return undefined;
  const type = frontmatter.get("type")?.trim() ?? "project";
  if (!isMemoryType(type)) return undefined;
  return {
    name,
    type,
    title,
    description: frontmatter.get("description")?.trim() ?? "",
    modified: frontmatter.get("modified")?.trim() || undefined,
    body
  };
}

/**
 * The section appended to the system prompt: a one-line-per-note index the model scans to
 * decide what to recall. Details stay out of it — `memory_recall` reads the body on demand —
 * so even a large project costs the prompt one line per note. Undefined when there is nothing.
 */
export function memoryIndex(notes: readonly MemoryNote[], scope: "project" | "chat" = "project"): string | undefined {
  if (notes.length === 0) return undefined;
  const order = new Map(MEMORY_TYPES.map((type, index) => [type, index]));
  const sorted = [...notes].sort((a, b) =>
    (order.get(a.type) ?? order.size) - (order.get(b.type) ?? order.size) || a.title.localeCompare(b.title));
  const lines = sorted.map((note) => {
    const described = note.description ? ` — ${note.description}` : "";
    return `- [${note.type}] ${note.title}${described} (name: ${note.name})`;
  });
  let truncated = false;
  while (lines.length > INDEX_MAX_ENTRIES || lines.join("\n").length > INDEX_MAX_CHARS) {
    lines.pop();
    truncated = true;
    if (lines.length === 0) break;
  }
  return [
    scope === "chat" ? "## Memory" : "## Project memory",
    "",
    `Notes saved from earlier ${scope === "chat" ? "chats" : "conversations in this project"}, one line each. When one is relevant to the current request, read its full text with memory_recall; otherwise leave it alone.`,
    "",
    ...lines,
    ...(truncated ? ["", `[… ${sorted.length - lines.length} more notes are not listed; recall them by name if you know it, and prune stale notes with memory_forget.]`] : []),
    "",
    "Keep memory small and current: memory_save to add or update a note, memory_forget to drop one that no longer holds."
  ].join("\n");
}
