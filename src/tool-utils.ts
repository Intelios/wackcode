import type { NormalizedBlock, SubagentDetails, SubagentResult, SubagentStatus, ToolCatalogEntry } from "./types";
import { displayPath } from "./chat-utils";

/** The built-in sub-agents tool. Its calls render as a card rather than a tool row. */
export const SUBAGENT_TOOL_NAME = "subagent";
/** The one WackCode built-in tool the user can switch off, through the tool denylist. */
export const WEB_FETCH_TOOL_NAME = "web_fetch";

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

/** A fetched URL as a row subject: host (with any port) and path, without the scheme, "www." or the query. */
export function displayUrl(value: string): string {
  try {
    const url = new URL(value);
    return `${url.host.replace(/^www\./, "")}${url.pathname === "/" ? "" : url.pathname}`;
  } catch {
    return value;
  }
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
    case WEB_FETCH_TOOL_NAME:
      return { kind: "other", activeVerb: "Fetching", doneVerb: "Fetched", subject: displayUrl(str(toolArgs.url)) };
    case SUBAGENT_TOOL_NAME: {
      const tasks = Array.isArray(toolArgs.tasks) ? toolArgs.tasks.length : 0;
      return {
        kind: "other",
        activeVerb: tasks ? `Running ${tasks} sub-agents` : "Running sub-agent",
        doneVerb: tasks ? `Ran ${tasks} sub-agents` : "Ran sub-agent",
        subject: tasks ? "" : str(toolArgs.agent)
      };
    }
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
 * the app itself. The one that can be switched off, web_fetch, has its switch on its Built-ins card.
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


const SUBAGENT_STATUSES: SubagentStatus[] = ["queued", "running", "done", "failed", "aborted"];

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function subagentResult(value: unknown): SubagentResult | undefined {
  if (!value || typeof value !== "object") return undefined;
  const entry = value as Record<string, unknown>;
  if (typeof entry.agent !== "string" || typeof entry.task !== "string") return undefined;
  const status = SUBAGENT_STATUSES.includes(entry.status as SubagentStatus) ? entry.status as SubagentStatus : undefined;
  if (!status) return undefined;
  const usage = (entry.usage && typeof entry.usage === "object" ? entry.usage : {}) as Record<string, unknown>;
  const activity = Array.isArray(entry.activity)
    ? entry.activity.flatMap((item) => {
        const call = item as Record<string, unknown> | null;
        return call && typeof call.tool === "string" ? [{ tool: call.tool, subject: str(call.subject) }] : [];
      })
    : [];
  return {
    agent: entry.agent,
    task: entry.task,
    readOnly: entry.readOnly === true,
    model: typeof entry.model === "string" ? entry.model : undefined,
    status,
    activity,
    output: typeof entry.output === "string" ? entry.output : undefined,
    outputTruncated: entry.outputTruncated === true,
    error: typeof entry.error === "string" ? entry.error : undefined,
    usage: {
      input: num(usage.input),
      output: num(usage.output),
      cacheRead: num(usage.cacheRead),
      cacheWrite: num(usage.cacheWrite),
      cost: num(usage.cost),
      turns: num(usage.turns)
    },
    startedAt: typeof entry.startedAt === "number" ? entry.startedAt : undefined,
    endedAt: typeof entry.endedAt === "number" ? entry.endedAt : undefined
  };
}

/**
 * The card's data from a `subagent` result or live update. Details are stored in session
 * files, so anything that isn't a version this app understands is treated as absent and the
 * call falls back to a plain tool row.
 */
export function parseSubagentDetails(value: unknown): SubagentDetails | undefined {
  if (!value || typeof value !== "object") return undefined;
  const details = value as Record<string, unknown>;
  if (details.v !== 1 || !Array.isArray(details.results) || details.results.length === 0) return undefined;
  const results = details.results.map(subagentResult);
  if (results.some((result) => !result)) return undefined;
  return { v: 1, mode: details.mode === "parallel" ? "parallel" : "single", results: results as SubagentResult[] };
}

/** A card for a call that has not reported progress yet, built from its arguments. */
export function pendingSubagentDetails(call: NormalizedBlock): SubagentDetails | undefined {
  const toolArgs = args(call);
  const queued = (agent: unknown, task: unknown): SubagentResult | undefined =>
    typeof agent === "string" && agent && typeof task === "string"
      ? { agent, task, readOnly: false, status: "queued", activity: [], usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, turns: 0 } }
      : undefined;
  if (Array.isArray(toolArgs.tasks)) {
    const results = toolArgs.tasks.map((item) => {
      const task = item as Record<string, unknown> | null;
      return queued(task?.agent, task?.task);
    });
    if (!results.length || results.some((result) => !result)) return undefined;
    return { v: 1, mode: "parallel", results: results as SubagentResult[] };
  }
  const single = queued(toolArgs.agent, toolArgs.task);
  return single ? { v: 1, mode: "single", results: [single] } : undefined;
}
