/**
 * Syntax highlighting: tokenizes code for chat code fences and both diff surfaces, via `lowlight`
 * (bundled highlight.js grammars — nothing is fetched at runtime). The token spans carry
 * highlight.js class names; styles.css maps them onto the accent-derived `--wc-code-*` tokens, so
 * this module stays colour-blind.
 *
 * Every result is memoised by content: streamed transcripts re-render the same closed fences on
 * every smooth-text frame and the Changes panel re-renders hunks constantly, so repeats must be
 * effectively free. Caches are bounded (LRU) because transcripts are unbounded.
 */

import { createElement, Fragment, type ReactNode } from "react";
import { createLowlight, common } from "lowlight";
import type { Element, Root, RootContent } from "hast";

const lowlight = createLowlight(common);
const COMMON_NAMES = Object.keys(common);

/** Fence tags that mean "not really code": never tokenised and never auto-detected. */
const PLAIN_TAGS = new Set(["console", "log", "output", "terminal", "nohighlight", "none"]);

/** File extensions → highlight.js language names, within the bundled `common` grammars. */
const EXTENSIONS: Record<string, string> = {
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  json: "json", jsonc: "json",
  css: "css", scss: "scss", less: "less",
  html: "xml", htm: "xml", xml: "xml", svg: "xml", vue: "xml", xsl: "xml", plist: "xml",
  md: "markdown", mdx: "markdown", markdown: "markdown",
  py: "python", pyi: "python", pyw: "python",
  rb: "ruby", rake: "ruby", gemspec: "ruby",
  rs: "rust",
  go: "go",
  java: "java", kt: "kotlin", kts: "kotlin", swift: "swift",
  c: "c", h: "c",
  cpp: "cpp", cc: "cpp", cxx: "cpp", "c++": "cpp", hpp: "cpp", hh: "cpp", hxx: "cpp",
  cs: "csharp", m: "objectivec", mm: "objectivec",
  php: "php", phtml: "php",
  sh: "shell", bash: "shell", zsh: "shell", ksh: "shell",
  sql: "sql",
  yaml: "yaml", yml: "yaml",
  toml: "ini", ini: "ini", cfg: "ini", conf: "ini", properties: "ini", env: "ini",
  lua: "lua",
  r: "r",
  pl: "perl", pm: "perl",
  diff: "diff", patch: "diff",
  vb: "vbnet",
  graphql: "graphql", gql: "graphql",
  wasm: "wasm"
};

/** The highlighting language for a file path, or undefined when we can't tell. */
export function languageForPath(path: string | null | undefined): string | undefined {
  if (!path) return undefined;
  const base = path.split("/").pop() ?? path;
  if (/^makefile$/i.test(base)) return "makefile";
  const dot = base.lastIndexOf(".");
  // A dotfile (`.zshrc`) or an extensionless path tells us nothing.
  if (dot <= 0) return undefined;
  return EXTENSIONS[base.slice(dot + 1).toLowerCase()];
}

/**
 * Tokenize `code` into a hast tree, or undefined to render it as plain text. `allowAuto` enables
 * highlight.js auto-detection for unknown fence tags; diff content only ever uses the file's own
 * language, so it opts out.
 */
function tokenize(code: string, lang: string | undefined, allowAuto: boolean): Root | undefined {
  if (!lang || PLAIN_TAGS.has(lang)) return undefined;
  try {
    if (lowlight.registered(lang)) return lowlight.highlight(lang, code);
    if (!allowAuto) return undefined;
    // Bound auto-detection to the bundled grammars; hljs never throws on ordinary input but a
    // pathological fence shouldn't take the transcript down either.
    return lowlight.highlightAuto(code, { subset: COMMON_NAMES });
  } catch {
    return undefined;
  }
}

function classNameOf(node: Element): string | undefined {
  const value = node.properties?.className;
  const joined = (Array.isArray(value) ? value : [])
    .filter((entry): entry is string => typeof entry === "string").join(" ");
  return joined || undefined;
}

function toNodes(children: RootContent[], key: string): ReactNode[] {
  return children.map((child, index) => {
    if (child.type === "text") return child.value;
    if (child.type === "element") {
      const childKey = `${key}/${index}`;
      return createElement(child.tagName, { key: childKey, className: classNameOf(child) }, ...toNodes(child.children, childKey));
    }
    return null;
  });
}

const MAX_ENTRIES = 200;

function trimCache(cache: Map<string, unknown>): void {
  while (cache.size > MAX_ENTRIES) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    cache.delete(oldest);
  }
}

const blockCache = new Map<string, ReactNode[]>();

/**
 * React nodes for a fenced code block's content. `tag` is the fence info string (`ts` from
 * ```ts); known languages highlight, unknown ones auto-detect within the bundled grammars, and
 * absent or non-code tags come back as plain text.
 */
export function highlightBlock(code: string, tag?: string | null): ReactNode[] {
  const lang = tag?.trim().toLowerCase();
  const key = `${lang ?? ""}\u0000${code}`;
  const hit = blockCache.get(key);
  if (hit) {
    blockCache.delete(key);
    blockCache.set(key, hit);
    return hit;
  }
  const root = tokenize(code, lang, true);
  const nodes = root ? toNodes(root.children, "b") : [code];
  blockCache.set(key, nodes);
  trimCache(blockCache);
  return nodes;
}

/**
 * Split hast content into per-line React nodes (one entry per `\n`), duplicating any element
 * that spans a break onto both lines so tokens survive diff line splits.
 */
function splitByLine(children: RootContent[], lines: ReactNode[][], key: string): void {
  for (let index = 0; index < children.length; index += 1) {
    const child = children[index];
    const childKey = `${key}/${index}`;
    if (child.type === "text") {
      const parts = child.value.split("\n");
      for (let part = 0; part < parts.length; part += 1) {
        if (part > 0) lines.push([]);
        if (parts[part]) lines[lines.length - 1].push(parts[part]);
      }
    } else if (child.type === "element") {
      const inner: ReactNode[][] = [[]];
      splitByLine(child.children, inner, childKey);
      const className = classNameOf(child);
      for (let line = 0; line < inner.length; line += 1) {
        if (line > 0) lines.push([]);
        if (inner[line].length) {
          lines[lines.length - 1].push(createElement(child.tagName, { key: `${childKey}#${line}`, className }, ...inner[line]));
        }
      }
    }
  }
}

const diffCache = new Map<string, ReactNode[][]>();

/**
 * Per-line token nodes for `content` — the code text of a run of diff lines joined with `\n`.
 * Returns exactly `count` lines, or undefined if anything doesn't line up (then callers fall
 * back to plain text).
 */
function highlightDiffContent(content: string, lang: string, count: number): ReactNode[][] | undefined {
  const key = `${lang}\u0000${content}`;
  const hit = diffCache.get(key);
  if (hit) {
    diffCache.delete(key);
    diffCache.set(key, hit);
    return hit;
  }
  const root = tokenize(content, lang, false);
  const lines: ReactNode[][] = [[]];
  if (root) {
    splitByLine(root.children, lines, "d");
  } else {
    content.split("\n").forEach((line, index) => {
      if (index) lines.push([]);
      lines[lines.length - 1].push(line);
    });
  }
  if (lines.length !== count) return undefined;
  diffCache.set(key, lines);
  trimCache(diffCache);
  return lines;
}

/**
 * Per-line content for a diff: the `+`/`-`/space lines are tokenized with the file's `lang`,
 * keeping the diff marker as a plain leading character so it inherits the line's add/del ink;
 * everything else (hunk headers, `\ No newline at end of file`) stays plain. Falls back to the
 * plain texts when the language is unknown. `kinds` marks lines that are not code as `"meta"`.
 */
export function highlightDiffLines(texts: string[], kinds: string[], lang: string | null | undefined): ReactNode[] {
  const plain = texts.map((text) => text || " ");
  if (!lang) return plain;
  const code: number[] = [];
  const parts: string[] = [];
  for (const [index, text] of texts.entries()) {
    if (kinds[index] === "meta" || !text || !"+- ".includes(text[0])) continue;
    code.push(index);
    parts.push(text.slice(1));
  }
  if (!parts.length) return plain;
  const perLine = highlightDiffContent(parts.join("\n"), lang, parts.length);
  if (!perLine) return plain;
  let next = 0;
  return texts.map((text, index) => {
    if (code[next] !== index) return text || " ";
    const at = next;
    next += 1;
    return createElement(Fragment, null, text[0], perLine[at]);
  });
}
