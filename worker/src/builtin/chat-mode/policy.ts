/**
 * Chat mode's tool policy, as pure functions: which tools a Chat mode chat may use, and whether
 * a file tool's path stays inside the chat's scratchpad.
 *
 * This is a behavioural policy, not a sandbox (docs/security.md). It covers Pi's own file tools.
 * It does not cover MCP servers or the in-app browser, which the user switches on themselves,
 * and it cannot see a hard link placed inside the scratchpad.
 *
 * `resolveLikePi` mirrors Pi's `resolveToCwd` and `resolveReadPath`
 * (dist/core/tools/path-utils.js), which Pi does not export. A Pi upgrade that changes how file
 * tools resolve a path must change it here too; `policy.test.ts` pins the cases.
 */
import { accessSync, constants, lstatSync, realpathSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { ASK_USER_QUESTION_TOOL_NAME } from "../ask-user-question.js";
import { BROWSER_TOOL_NAMES } from "../browser.js";
import { MEMORY_TOOL_NAMES } from "../memory/index.js";
import { WEB_FETCH_TOOL_NAME } from "../web-fetch/index.js";

/** Pi's file tools. Each takes its target as `path`; ls, grep and find default it to the cwd. */
export const CHAT_FILE_TOOLS: ReadonlySet<string> = new Set(["read", "write", "edit", "ls", "grep", "find"]);
const PATH_OPTIONAL: ReadonlySet<string> = new Set(["ls", "grep", "find"]);

/** WackCode's own tools a Chat mode chat keeps. Each still obeys its own setting. */
export const CHAT_BUILTIN_TOOLS: ReadonlySet<string> = new Set([
  ASK_USER_QUESTION_TOOL_NAME, WEB_FETCH_TOOL_NAME, ...MEMORY_TOOL_NAMES, ...BROWSER_TOOL_NAMES
]);

export type ChatToolSource = "builtin" | "wackcode" | "mcp" | "package";

/** Whether a registered tool may be active in a Chat mode chat. Unknown sources stay out. */
export function chatToolAllowed(name: string, source: ChatToolSource): boolean {
  if (source === "builtin") return CHAT_FILE_TOOLS.has(name);
  if (source === "wackcode") return CHAT_BUILTIN_TOOLS.has(name);
  return source === "mcp";
}

const UNICODE_SPACES = /[  -   　]/g;

/** Pi's `resolveToCwd`: unicode spaces, a leading `@`, `~`, `file://`, then cwd-relative. */
export function resolveLikePi(input: string, cwd: string): string {
  let path = input.replace(UNICODE_SPACES, " ");
  if (path.startsWith("@")) path = path.slice(1);
  if (path === "~") path = homedir();
  else if (path.startsWith("~/")) path = join(homedir(), path.slice(2));
  else if (/^file:\/\//.test(path)) path = fileURLToPath(path);
  return isAbsolute(path) ? resolve(path) : resolve(cwd, path);
}

function exists(path: string): boolean {
  try {
    accessSync(path, constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

/** Pi's `resolveReadPath` candidates, in its order: the first that exists is what `read` opens. */
function readCandidates(resolved: string): string[] {
  const amPm = resolved.replace(/ (AM|PM)\./gi, " $1.");
  const nfd = resolved.normalize("NFD");
  const curly = resolved.replace(/'/g, "’");
  const nfdCurly = nfd.replace(/'/g, "’");
  return [...new Set([resolved, amPm, nfd, curly, nfdCurly])];
}

function lexists(path: string): boolean {
  try {
    lstatSync(path);
    return true;
  } catch {
    return false;
  }
}

/**
 * The real location a path names: symlinks resolved through the deepest component that exists,
 * the not-yet-created remainder appended as written. Undefined when that component cannot be
 * resolved (a dangling symlink), which the caller treats as outside.
 */
export function canonicalPath(path: string): string | undefined {
  let existing = path;
  const rest: string[] = [];
  while (!lexists(existing)) {
    const parent = dirname(existing);
    if (parent === existing) return undefined;
    rest.unshift(basename(existing));
    existing = parent;
  }
  try {
    return join(realpathSync.native(existing), ...rest);
  } catch {
    return undefined;
  }
}

function inside(root: string, path: string): boolean {
  return path === root || path.startsWith(root + sep);
}

/** A scratchpad-relative spelling Pi resolves back to the same file. */
function relativeSpelling(root: string, canonical: string): string {
  const spelled = relative(root, canonical) || ".";
  return /^[@~]/.test(spelled) || /^file:\/\//.test(spelled) ? `./${spelled}` : spelled;
}

export type Confinement =
  /** Allowed. `path` is the spelling to hand Pi; absent means leave the argument as it is. */
  | { ok: true; path?: string }
  | { ok: false; reason: string };

/**
 * Decide one file-tool call. `cwd` is the session's working directory, which for a Chat mode
 * chat is its scratchpad. Fails closed: anything that cannot be shown to be inside is refused.
 */
export function confine(toolName: string, rawPath: unknown, cwd: string): Confinement {
  const root = canonicalPath(cwd);
  if (!root || !lexists(root)) return { ok: false, reason: "This chat's scratchpad folder is not available." };
  if (rawPath === undefined || rawPath === null || rawPath === "") {
    return PATH_OPTIONAL.has(toolName) ? { ok: true } : { ok: false, reason: `${toolName} needs a path inside the scratchpad.` };
  }
  if (typeof rawPath !== "string") return { ok: false, reason: `${toolName} needs a path inside the scratchpad.` };
  const outside = { ok: false as const, reason: `This chat can only use files in its scratchpad folder, and '${rawPath}' is outside it.` };
  let resolved: string;
  try {
    resolved = resolveLikePi(rawPath, cwd);
  } catch {
    return outside;
  }
  let target = resolved;
  if (toolName === "read") {
    const found = readCandidates(resolved).find(exists);
    // Pi would fail the read anyway. Refusing here keeps its filename fallbacks from being
    // applied to a path this check never saw.
    if (!found) return inside(root, canonicalPath(resolved) ?? "") ? { ok: false, reason: `There is no file '${rawPath}' in the scratchpad.` } : outside;
    target = found;
  }
  const canonical = canonicalPath(target);
  if (!canonical || !inside(root, canonical)) return outside;
  // Hand Pi the location that was checked, spelled relative to the scratchpad. Kept only when
  // Pi's own resolution of that spelling lands on the same place (a name holding a unicode
  // space would not), so the rewrite can never point Pi somewhere else.
  const spelled = relativeSpelling(root, canonical);
  try {
    if (canonicalPath(resolveLikePi(spelled, cwd)) === canonical) return { ok: true, path: spelled };
  } catch { /* Keep the model's own spelling, which was checked above. */ }
  return { ok: true };
}

export type ChatToolDecision = { block: true; reason: string } | undefined;

/**
 * The decision for one tool call in a Chat mode chat. An allowed file call has its `path`
 * rewritten in place (Pi's documented way to patch arguments before execution).
 */
export function chatToolDecision(toolName: string, input: unknown, cwd: string | undefined, ownsMcpTool: (name: string) => boolean): ChatToolDecision {
  if (CHAT_FILE_TOOLS.has(toolName)) {
    if (!cwd) return { block: true, reason: "This chat's scratchpad folder is not available." };
    if (!input || typeof input !== "object" || Array.isArray(input)) return { block: true, reason: `${toolName} needs a path inside the scratchpad.` };
    const args = input as Record<string, unknown>;
    const result = confine(toolName, args.path, cwd);
    if (!result.ok) return { block: true, reason: result.reason };
    if (result.path !== undefined) args.path = result.path;
    return undefined;
  }
  if (CHAT_BUILTIN_TOOLS.has(toolName) || ownsMcpTool(toolName)) return undefined;
  return { block: true, reason: `'${toolName}' is not available in Chat mode.` };
}
