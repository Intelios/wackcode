/**
 * `@` file mentions, modelled on Pi's own TUI autocomplete (pi-tui `CombinedAutocompleteProvider`):
 * the message keeps `@path` (or `@"path with spaces"`) as plain text and the model reads the file itself.
 */

/** The `@` token the caret is in: `draft.slice(start, end)` is replaced when a suggestion is chosen. */
export interface ActiveMention {
  start: number;
  end: number;
  /** What was typed after `@` (or `@"`), up to the caret. */
  query: string;
  quoted: boolean;
}

export interface MentionSuggestion {
  /** Relative to the workspace; directories end with `/`. */
  path: string;
  directory: boolean;
}

/** Characters that may sit right before a mention's `@`, besides whitespace. */
const OPENERS = new Set(["(", "[", "{", "\"", "'", "`"]);

function atTokenStart(text: string, index: number): boolean {
  if (index === 0) return true;
  const previous = text[index - 1];
  return /\s/.test(previous) || OPENERS.has(previous);
}

export function activeMention(text: string, caret: number): ActiveMention | null {
  const before = text.slice(0, caret);
  const quoteAt = before.lastIndexOf("@\"");
  if (quoteAt >= 0 && atTokenStart(text, quoteAt)) {
    const query = before.slice(quoteAt + 2);
    if (!/["\n]/.test(query)) {
      const end = text[caret] === "\"" ? caret + 1 : caret;
      return { start: quoteAt, end, query, quoted: true };
    }
  }
  const tokenStart = Math.max(before.lastIndexOf(" "), before.lastIndexOf("\n"), before.lastIndexOf("\t")) + 1;
  let start = tokenStart;
  if (OPENERS.has(text[start]) && text[start + 1] === "@") start += 1;
  if (text[start] !== "@" || !atTokenStart(text, start)) return null;
  const query = before.slice(start + 1);
  if (/["'`()[\]{}]/.test(query)) return null;
  let end = caret;
  while (end < text.length && !/\s/.test(text[end])) end += 1;
  return { start, end, query, quoted: false };
}

/** Pi's `buildCompletionValue`: quote a path that would otherwise end the token early. */
export function mentionValue(path: string, directory = false): string {
  if (!/\s/.test(path)) return `@${path}`;
  // A quoted directory stays open so the next keystrokes keep completing inside it.
  return directory ? `@"${path}` : `@"${path}"`;
}

const entryCache = new WeakMap<string[], MentionSuggestion[]>();

/** Every file plus every folder that holds one, computed once per listing. */
function entriesFor(files: string[]): MentionSuggestion[] {
  const cached = entryCache.get(files);
  if (cached) return cached;
  const directories = new Set<string>();
  for (const file of files) {
    let slash = file.indexOf("/");
    while (slash >= 0) {
      directories.add(file.slice(0, slash + 1));
      slash = file.indexOf("/", slash + 1);
    }
  }
  const entries = [
    ...[...directories].map((path) => ({ path, directory: true })),
    ...files.map((path) => ({ path, directory: false })),
  ];
  entryCache.set(files, entries);
  return entries;
}

function depth(path: string): number {
  return path.replace(/\/$/, "").split("/").length - 1;
}

function isSubsequence(query: string, target: string): boolean {
  let index = 0;
  for (const char of target) {
    if (char === query[index]) index += 1;
    if (index === query.length) return true;
  }
  return index === query.length;
}

function score(entry: MentionSuggestion, query: string): number {
  const path = entry.path.toLowerCase();
  const bare = path.replace(/\/$/, "");
  let value = 0;
  if (!query) value = depth(path) === 0 ? 1 : 0;
  else if (query.includes("/")) {
    if (path === query) value = 0;
    else if (path.startsWith(query)) value = path.slice(query.length).replace(/\/$/, "").includes("/") ? 80 : 100;
    else if (path.includes(query)) value = 50;
    else if (isSubsequence(query, bare)) value = 10;
  } else {
    const name = bare.slice(bare.lastIndexOf("/") + 1);
    const stem = name.includes(".", 1) ? name.slice(0, name.lastIndexOf(".")) : name;
    if (name === query || stem === query) value = 100;
    else if (name.startsWith(query)) value = 80;
    else if (name.includes(query)) value = 60;
    else if (bare.includes(query)) value = 40;
    // Letters scattered across a whole path match almost anything; keep fuzzy to the name.
    else if (isSubsequence(query, name)) value = 10;
  }
  // Folders edge ahead of files that match equally well, as in Pi.
  return value > 0 && entry.directory ? value + 2 : value;
}

/** The best matches for what follows `@`: folders end with `/`, an empty query lists the top level. */
export function rankMentions(files: string[], query: string, limit = 20): MentionSuggestion[] {
  const needle = query.toLowerCase();
  const scored: { entry: MentionSuggestion; score: number }[] = [];
  for (const entry of entriesFor(files)) {
    const value = score(entry, needle);
    if (value > 0) scored.push({ entry, score: value });
  }
  scored.sort((a, b) => b.score - a.score
    || depth(a.entry.path) - depth(b.entry.path)
    || a.entry.path.length - b.entry.path.length
    || a.entry.path.localeCompare(b.entry.path));
  return scored.slice(0, limit).map((item) => item.entry);
}

export interface TextSegment {
  text: string;
  mention?: boolean;
}

const MENTION_PATTERN = /@"[^"\n]+"|@[^\s"'`()[\]{}]+/g;

/**
 * Splits sent text so path-like mentions can be highlighted. Only quoted mentions and ones that
 * contain `/` or `.` count, so `@someone` stays plain; trailing sentence punctuation is left out.
 */
export function splitMentions(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  let cursor = 0;
  for (const match of text.matchAll(MENTION_PATTERN)) {
    const index = match.index;
    if (!atTokenStart(text, index)) continue;
    let token = match[0];
    if (!token.startsWith("@\"")) {
      token = token.replace(/[.,;:!?]+$/, "");
      if (!/[./]/.test(token.slice(1))) continue;
    }
    if (index > cursor) segments.push({ text: text.slice(cursor, index) });
    segments.push({ text: token, mention: true });
    cursor = index + token.length;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor) });
  return segments;
}
