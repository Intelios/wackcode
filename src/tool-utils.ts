import type { NormalizedBlock } from "./types";
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
  kind: "read" | "edit" | "write" | "bash" | "other";
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
    default:
      return { kind: "other", activeVerb: name, doneVerb: name, subject: "" };
  }
}
