/**
 * The files a Chat mode reply made, as pure functions. A chat's scratchpad is its
 * `workspacePath`; the worker confines every file tool to it (`chat-mode/policy.ts`), so a
 * relative path resolves against it. A path that still points elsewhere is not offered: the
 * chip only ever reveals something inside this chat's own folder.
 */
import type { NormalizedBlock } from "../../types";
import { baseName } from "./chat-activity";

export interface ScratchpadFile {
  /** Absolute path inside the scratchpad. */
  path: string;
  name: string;
  /** The folder inside the scratchpad, "" at its root. */
  folder: string;
}

function argPath(call: NormalizedBlock): string {
  const value = call.arguments;
  let parsed: unknown = value;
  if (typeof value === "string") {
    try { parsed = JSON.parse(value); } catch { return ""; }
  }
  const path = parsed && typeof parsed === "object" ? (parsed as Record<string, unknown>).path : undefined;
  return typeof path === "string" ? path.trim() : "";
}

function normalize(path: string): string {
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") parts.pop(); else parts.push(part);
  }
  return `/${parts.join("/")}`;
}

/** Resolves a tool path against the scratchpad, or undefined when it lands outside it. */
export function resolveScratchpadPath(path: string, scratchpad: string): string | undefined {
  const root = normalize(scratchpad);
  const cleaned = path.replace(/^@/, "").replace(/^file:\/\//, "");
  if (!cleaned || cleaned.startsWith("~")) return undefined;
  const full = normalize(cleaned.startsWith("/") ? cleaned : `${root}/${cleaned}`);
  return full === root || !full.startsWith(`${root}/`) ? undefined : full;
}

/** Each file a reply successfully wrote or edited, once, in the order first touched. */
export function scratchpadFiles(calls: NormalizedBlock[], results: Map<string, NormalizedBlock>, scratchpad: string): ScratchpadFile[] {
  const seen = new Set<string>();
  const files: ScratchpadFile[] = [];
  const root = normalize(scratchpad);
  for (const call of calls) {
    if (call.toolName !== "write" && call.toolName !== "edit") continue;
    const result = call.toolCallId ? results.get(call.toolCallId) : undefined;
    if (!result || result.isError) continue;
    const path = resolveScratchpadPath(argPath(call), scratchpad);
    if (!path || seen.has(path)) continue;
    seen.add(path);
    const inside = path.slice(root.length + 1);
    const slash = inside.lastIndexOf("/");
    files.push({ path, name: baseName(path), folder: slash < 0 ? "" : inside.slice(0, slash) });
  }
  return files;
}
