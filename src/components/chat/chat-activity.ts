/**
 * Chat mode's tool calls as conversational asides, as pure functions. Where the Code
 * transcript shows a tool row ("Read src/app.ts"), Chat says what the agent did in plain
 * words ("Read notes.md", "Looked up example.com") and folds a run of them into one chip.
 *
 * Chat mode's tools are decided in the worker (`chat-mode/policy.ts`): Pi's file tools inside
 * the scratchpad, `web_fetch`, memory, the in-app browser, `ask_user_question` and MCP tools.
 * Anything else still gets a sensible generic label rather than being hidden.
 */
import type { NormalizedBlock } from "../../types";
import { displayUrl, mcpToolParts } from "../../tool-utils";
import type { IconName } from "../Icons";

export type ActivityStatus = "live" | "done" | "failed";

export interface Activity {
  key: string;
  call: NormalizedBlock;
  result?: NormalizedBlock;
  /** What a chip says, e.g. "Looked up example.com". */
  label: string;
  icon: IconName;
  status: ActivityStatus;
  /** Screenshot results carry inline previews. */
  images: boolean;
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

const str = (value: unknown) => typeof value === "string" ? value.trim() : "";

/** A path's last part: the scratchpad is the chat's own, so folders rarely matter. */
export function baseName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.slice(trimmed.lastIndexOf("/") + 1) || trimmed;
}

function quoted(text: string, max = 32): string {
  const flat = text.replace(/\s+/g, " ");
  return `“${flat.length > max ? `${flat.slice(0, max)}…` : flat}”`;
}

/** The present- and past-tense chip text for one call. */
export function activityLabel(call: NormalizedBlock, live: boolean): { label: string; icon: IconName } {
  const a = args(call);
  const name = call.toolName ?? "tool";
  const pick = (now: string, then: string) => live ? now : then;
  const join = (verb: string, subject: string) => subject ? `${verb} ${subject}` : verb;
  switch (name) {
    case "read": return { icon: "file", label: join(pick("Reading", "Read"), baseName(str(a.path))) };
    case "write": return { icon: "quill", label: join(pick("Jotting down", "Jotted down"), baseName(str(a.path))) };
    case "edit": return { icon: "pencil", label: join(pick("Touching up", "Touched up"), baseName(str(a.path))) };
    case "ls": return { icon: "folder", label: pick("Peeking at the scratchpad", "Peeked at the scratchpad") };
    case "grep": return { icon: "search", label: join(pick("Searching for", "Searched for"), str(a.pattern) ? quoted(str(a.pattern)) : "") };
    case "find": return { icon: "search", label: join(pick("Looking for", "Looked for"), str(a.pattern) ? quoted(str(a.pattern)) : "files") };
    case "web_fetch": return { icon: "globe", label: join(pick("Looking up", "Looked up"), str(a.url) ? displayUrl(str(a.url)) : "a page") };
    case "memory_save": return { icon: "memory", label: join(pick("Remembering", "Remembered"), str(a.title) ? quoted(str(a.title)) : "that") };
    case "memory_recall": return { icon: "memory", label: pick("Recalling a memory", "Recalled a memory") };
    case "memory_forget": return { icon: "memory", label: pick("Forgetting a memory", "Forgot a memory") };
    case "ask_user_question": return { icon: "question", label: pick("Asking you", "Asked you") };
    case "browser_open": return { icon: "browser", label: join(pick("Opening", "Opened"), str(a.url) ? displayUrl(str(a.url)) : "the browser") };
    case "browser_snapshot": return { icon: "browser", label: pick("Reading the page", "Read the page") };
    case "browser_act": return { icon: "cursor", label: pick("Using the page", "Used the page") };
    case "browser_screenshot": return { icon: "image", label: pick("Snapping the page", "Snapped the page") };
    case "browser_console": return { icon: "browser", label: pick("Checking the console", "Checked the console") };
    default: {
      const mcp = mcpToolParts(name);
      if (mcp) return { icon: "plug", label: join(pick("Using", "Used"), `${mcp.tool.replace(/_/g, " ")} (${mcp.server})`) };
      return { icon: "wrench", label: join(pick("Using", "Used"), name.replace(/_/g, " ")) };
    }
  }
}

/** One chip per call in a reply, in order. `live` marks the reply as still running. */
export function activitiesOf(calls: NormalizedBlock[], results: Map<string, NormalizedBlock>, live: boolean): Activity[] {
  return calls.map((call, index) => {
    const result = call.toolCallId ? results.get(call.toolCallId) : undefined;
    const status: ActivityStatus = result ? (result.isError ? "failed" : "done") : live ? "live" : "done";
    const { label, icon } = activityLabel(call, status === "live");
    return { key: call.toolCallId ?? `call:${index}`, call, result, label, icon, status, images: Boolean(result?.images?.length) };
  });
}

/** Runs this long or longer fold into a "Did N things" chip once they have all settled. */
export const FOLD_AT = 3;

export type ActivityGroup =
  | { type: "single"; activity: Activity }
  | { type: "fold"; key: string; activities: Activity[]; label: string };

/** Settled runs of `FOLD_AT`+ calls fold; live calls, failures and screenshots always show. */
export function groupActivities(activities: Activity[]): ActivityGroup[] {
  const groups: ActivityGroup[] = [];
  let run: Activity[] = [];
  const flush = () => {
    if (run.length >= FOLD_AT) groups.push({ type: "fold", key: `fold:${run[0].key}`, activities: run, label: `Did ${run.length} things` });
    else run.forEach((activity) => groups.push({ type: "single", activity }));
    run = [];
  };
  for (const activity of activities) {
    if (activity.status === "done" && !activity.images) { run.push(activity); continue; }
    flush();
    groups.push({ type: "single", activity });
  }
  flush();
  return groups;
}
