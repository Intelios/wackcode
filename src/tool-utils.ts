import type { NormalizedBlock, ToolCatalogEntry } from "./types";
import { displayPath } from "./chat-utils";

export interface ToolSummary {
  /** Label while the tool is executing, e.g. "Editing". */
  activeVerb: string;
  /** Label once finished, e.g. "Edited". */
  doneVerb: string;
  /** Path or command shown after the verb. */
  subject: string;
  additions?: number;
  deletions?: number;
  /** Kind controls what the expanded body shows. */
  kind: "read" | "edit" | "write" | "bash" | "search" | "other";
}

function args(block: NormalizedBlock): Record<string, unknown> {
  const value = block.arguments;
  if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>;
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value) as unknown;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed as Record<string, unknown>;
    } catch { /* still streaming */ }
  }
  return {};
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function countLines(text: string): number {
  if (!text) return 0;
  return text.split("\n").filter((line) => line.length > 0).length;
}

export function diffStats(diff: unknown): { additions: number; deletions: number } | undefined {
  if (typeof diff !== "string" || !diff) return undefined;
  let additions = 0;
  let deletions = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }
  return additions || deletions ? { additions, deletions } : undefined;
}

function detailsOf(block?: NormalizedBlock): Record<string, unknown> {
  return (block?.details ?? {}) as Record<string, unknown>;
}

export function editStats(call: NormalizedBlock, result?: NormalizedBlock): { additions: number; deletions: number } | undefined {
  const fromDiff = diffStats(detailsOf(result).diff);
  if (fromDiff) return fromDiff;
  const edits = args(call).edits;
  if (!Array.isArray(edits)) return undefined;
  let additions = 0;
  let deletions = 0;
  for (const edit of edits) {
    const entry = edit as Record<string, unknown>;
    deletions += countLines(str(entry.oldText));
    additions += countLines(str(entry.newText));
  }
  return additions || deletions ? { additions, deletions } : undefined;
}

export function summarizeTool(call: NormalizedBlock, result?: NormalizedBlock): ToolSummary {
  const toolArgs = args(call);
  const name = call.toolName ?? "tool";
  switch (name) {
    case "read":
      return { kind: "read", activeVerb: "Reading", doneVerb: "Read", subject: displayPath(str(toolArgs.path)) };
    case "edit": {
      const stats = editStats(call, result);
      return { kind: "edit", activeVerb: "Editing", doneVerb: "Edited", subject: displayPath(str(toolArgs.path)), additions: stats?.additions, deletions: stats?.deletions };
    }
    case "write": {
      const content = str(toolArgs.content);
      const isNew = detailsOf(result).created === true || detailsOf(result).existed === false;
      return { kind: "write", activeVerb: "Writing", doneVerb: isNew ? "Created" : "Wrote", subject: displayPath(str(toolArgs.path)), additions: content ? countLines(content) : undefined };
    }
    case "bash":
      return { kind: "bash", activeVerb: "Running", doneVerb: "Ran", subject: str(toolArgs.command) };
    case "grep":
      return { kind: "search", activeVerb: "Searching", doneVerb: "Searched", subject: str(toolArgs.pattern) };
    case "find":
      return { kind: "search", activeVerb: "Finding", doneVerb: "Found", subject: str(toolArgs.pattern) };
    case "ls":
      return { kind: "search", activeVerb: "Listing", doneVerb: "Listed", subject: displayPath(str(toolArgs.path) || ".") };
    case "ask_user_question": {
      const questions = toolArgs.questions;
      const first = Array.isArray(questions) ? questions[0] as Record<string, unknown> | undefined : undefined;
      return { kind: "other", activeVerb: "Asking", doneVerb: "Asked", subject: str(first?.question) };
    }
    case "plan_mode_complete":
      return { kind: "other", activeVerb: "Submitting plan", doneVerb: "Plan submitted", subject: "" };
    case "todo":
      return { kind: "other", activeVerb: "Updating todos", doneVerb: "Todos updated", subject: str(toolArgs.subject) };
    default:
      return { kind: "other", activeVerb: name, doneVerb: name, subject: "" };
  }
}

export interface ToolGroup {
  /** Stable key: "builtin" for Pi's own tools, otherwise the package source string. */
  id: string;
  label: string;
  tools: ToolCatalogEntry[];
}

/**
 * Group the catalogue for the Tools panel: Pi's own tools first, then one group per package
 * in stable alphabetical order. A package tool with no source string falls into "Other" rather
 * than disappearing. WackCode's built-in extension tools are never listed — they are part of
 * the app itself and can't be switched off.
 */
export function groupTools(catalog: ToolCatalogEntry[]): ToolGroup[] {
  const builtin: ToolCatalogEntry[] = [];
  const byPackage = new Map<string, ToolCatalogEntry[]>();
  for (const tool of [...catalog].sort((left, right) => left.name.localeCompare(right.name))) {
    if (tool.source.kind === "builtin") {
      builtin.push(tool);
      continue;
    }
    if (tool.source.kind === "wackcode") {
      continue;
    }
    const id = tool.source.packageId ?? "other";
    const existing = byPackage.get(id);
    if (existing) existing.push(tool);
    else byPackage.set(id, [tool]);
  }
  const groups: ToolGroup[] = [];
  if (builtin.length) groups.push({ id: "builtin", label: "Built-in", tools: builtin });
  for (const id of [...byPackage.keys()].sort()) {
    groups.push({ id, label: id === "other" ? "Other" : id, tools: byPackage.get(id) ?? [] });
  }
  return groups;
}

/**
 * Drop names that are no longer in the catalogue. A denylist referencing a removed package's
 * tool is harmless but accumulates, and showing a stale count in Settings is confusing.
 */
export function pruneDisabledTools(disabled: string[], catalog: ToolCatalogEntry[]): string[] {
  if (!catalog.length) return [...disabled];
  const known = new Set(catalog.map((tool) => tool.name));
  return disabled.filter((name) => known.has(name));
}

/**
 * Snapshots arrive constantly and almost always carry an identical catalogue. Compare before
 * storing so Settings does not re-render on every streamed message.
 */
export function sameToolCatalog(left: ToolCatalogEntry[], right: ToolCatalogEntry[]): boolean {
  if (left.length !== right.length) return false;
  return left.every((tool, index) => {
    const other = right[index];
    return tool.name === other.name
      && tool.available === other.available
      && tool.description === other.description
      && tool.source.kind === other.source.kind
      && tool.source.packageId === other.source.packageId;
  });
}
