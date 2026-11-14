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
import { COMPARE_IMAGES_TOOL_NAME } from "../../image-comparisons";

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
    case "web_search": return { icon: "search", label: join(pick("Searching the web for", "Searched the web for"), str(a.query) ? quoted(str(a.query)) : "something") };
    case "memory_save": return { icon: "memory", label: join(pick("Remembering", "Remembered"), str(a.title) ? quoted(str(a.title)) : "that") };
    case "memory_recall": return { icon: "memory", label: pick("Recalling a memory", "Recalled a memory") };
    case "memory_forget": return { icon: "memory", label: pick("Forgetting a memory", "Forgot a memory") };
    case "ask_user_question": return { icon: "question", label: pick("Asking you", "Asked you") };
    // Only a visual that is still being drawn, or one that failed, reaches a chip: a settled one
    // is its own card (`ChatBubbles.tsx`).
    case "preview_visual": return { icon: "visual", label: pick("Previewing a visual", "Previewed a visual") };
    case "show_visual": return { icon: "visual", label: join(pick("Drawing", "Drew"), str(a.title) ? quoted(str(a.title)) : "a visual") };
    case COMPARE_IMAGES_TOOL_NAME: return { icon: "split", label: join(pick("Comparing", "Compared"), str(a.title) ? quoted(str(a.title)) : "two images") };
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

/** Runs this long or longer fold into a specific summary chip once they have all settled. */
export const FOLD_AT = 3;

export type ActivityGroup =
  | { type: "single"; activity: Activity }
  | { type: "fold"; key: string; activities: Activity[]; label: string };

type ChatBucket = "write" | "edit" | "web_fetch" | "web_search" | "browser" | "mcp" | "read" | "search" | "memory" | "other";

const CHAT_BUCKET_TIER: Record<ChatBucket, 1 | 2 | 3> = {
  write: 1,
  edit: 1,
  web_fetch: 2,
  web_search: 2,
  browser: 2,
  mcp: 2,
  read: 3,
  search: 3,
  memory: 3,
  other: 3
};

/**
 * Conversational, path-deduplicated summary for a folded run of Chat mode calls
 * (e.g. "Jotted down 2 files, looked up 2 pages", keeping Chat's casual register).
 */
export function summarizeActivityFold(activities: Activity[]): string {
  interface Acc {
    key: ChatBucket;
    tier: 1 | 2 | 3;
    order: number;
    paths: Set<string>;
    singleName?: string;
    count: number;
    memoryRead?: boolean;
    memoryWrite?: boolean;
  }
  const buckets = new Map<ChatBucket, Acc>();
  let order = 0;

  const touch = (key: ChatBucket): Acc => {
    let acc = buckets.get(key);
    if (!acc) {
      acc = { key, tier: CHAT_BUCKET_TIER[key], order: order++, paths: new Set(), count: 0 };
      buckets.set(key, acc);
    }
    acc.count += 1;
    return acc;
  };

  for (const activity of activities) {
    const a = args(activity.call);
    const name = activity.call.toolName ?? "tool";
    switch (name) {
      case "write":
      case "edit":
      case "read": {
        const acc = touch(name);
        const rawPath = str(a.path);
        if (rawPath) {
          acc.paths.add(rawPath);
          if (acc.paths.size === 1) acc.singleName = baseName(rawPath);
          else acc.singleName = undefined;
        }
        break;
      }
      case "web_fetch":
        touch("web_fetch");
        break;
      case "web_search":
        touch("web_search");
        break;
      case "ls":
      case "grep":
      case "find":
        touch("search");
        break;
      case "memory_recall": {
        const acc = touch("memory");
        acc.memoryRead = true;
        break;
      }
      case "memory_save":
      case "memory_forget": {
        const acc = touch("memory");
        acc.memoryWrite = true;
        break;
      }
      case "browser_open":
      case "browser_snapshot":
      case "browser_act":
      case "browser_screenshot":
      case "browser_console":
        touch("browser");
        break;
      default:
        touch(mcpToolParts(name) ? "mcp" : "other");
        break;
    }
  }

  const sizeOf = (acc: Acc) => acc.paths.size || acc.count;
  const sorted = [...buckets.values()].sort((a, b) => a.tier - b.tier || sizeOf(b) - sizeOf(a) || a.order - b.order);

  const phrase = (acc: Acc): string => {
    const n = sizeOf(acc);
    switch (acc.key) {
      case "read":
        return acc.singleName && acc.count === n && n === 1 ? `read ${acc.singleName}` : `read ${n} ${n === 1 ? "file" : "files"}`;
      case "write":
        return acc.singleName && n === 1 ? `jotted down ${acc.singleName}` : `jotted down ${n} ${n === 1 ? "file" : "files"}`;
      case "edit":
        return acc.singleName && n === 1 ? `updated ${acc.singleName}` : `updated ${n} ${n === 1 ? "file" : "files"}`;
      case "web_fetch":
        return `looked up ${n} ${n === 1 ? "page" : "pages"}`;
      case "web_search":
        return n === 1 ? "searched the web" : `searched the web ${n} times`;
      case "search":
        return n === 1 ? "checked files" : `checked ${n} times`;
      case "memory":
        return acc.memoryRead && acc.memoryWrite ? "used memory" : acc.memoryWrite ? "updated memory" : "recalled memory";
      case "browser":
        return `browsed ${n} ${n === 1 ? "step" : "steps"}`;
      case "mcp":
        return `used ${n} ${n === 1 ? "tool" : "tools"}`;
      case "other":
        return `did ${n} ${n === 1 ? "thing" : "things"}`;
    }
  };

  const shown = sorted.slice(0, 2).map(phrase);
  const overflow = sorted.slice(2).reduce((sum, acc) => sum + sizeOf(acc), 0);
  const text = overflow > 0 ? `${shown.join(", ")}, +${overflow} more` : shown.join(", ");
  return text ? text[0].toUpperCase() + text.slice(1) : `Did ${activities.length} things`;
}

/** Settled runs of `FOLD_AT`+ calls fold; live calls, failures and screenshots always show. */
export function groupActivities(activities: Activity[]): ActivityGroup[] {
  const groups: ActivityGroup[] = [];
  let run: Activity[] = [];
  const flush = () => {
    if (run.length >= FOLD_AT) groups.push({ type: "fold", key: `fold:${run[0].key}`, activities: run, label: summarizeActivityFold(run) });
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
