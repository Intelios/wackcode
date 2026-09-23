/**
 * The `subagent` tool's result: the details the transcript card renders (live and after a
 * reload) and the text the parent model reads. Details are capped hard because they ride
 * every snapshot of the chat and are stored in its session file.
 */
import type { Usage } from "@earendil-works/pi-ai";
import type { SubagentActivity, SubagentDetails, SubagentResult, SubagentUsage } from "../../protocol.js";
import type { SubagentMode, SubagentTaskInput } from "./schema.js";

/** Tool calls kept per child on the card. */
export const MAX_ACTIVITY = 12;
/** A child's answer on the card. */
export const MAX_CARD_OUTPUT = 16 * 1024;
/** A child's answer as the parent model receives it. */
export const MAX_MODEL_OUTPUT = 50 * 1024;
/** Card refresh interval while children run. */
export const UPDATE_INTERVAL_MS = 150;

const MAX_SUBJECT = 160;

export function emptyUsage(): Usage {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
  };
}

/** Add one assistant message's usage (or a child's total) into `total`. */
export function addUsage(total: Usage, usage: Partial<Usage> | undefined): Usage {
  if (!usage) return total;
  const number = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : 0);
  total.input += number(usage.input);
  total.output += number(usage.output);
  total.cacheRead += number(usage.cacheRead);
  total.cacheWrite += number(usage.cacheWrite);
  total.totalTokens += number(usage.totalTokens);
  total.cost.input += number(usage.cost?.input);
  total.cost.output += number(usage.cost?.output);
  total.cost.cacheRead += number(usage.cost?.cacheRead);
  total.cost.cacheWrite += number(usage.cost?.cacheWrite);
  total.cost.total += number(usage.cost?.total);
  return total;
}

export function cardUsage(usage: Usage, turns: number): SubagentUsage {
  return {
    input: usage.input,
    output: usage.output,
    cacheRead: usage.cacheRead,
    cacheWrite: usage.cacheWrite,
    cost: usage.cost.total,
    turns,
  };
}

export function createDetails(mode: SubagentMode, tasks: { input: SubagentTaskInput; readOnly: boolean }[]): SubagentDetails {
  return {
    v: 1,
    mode,
    results: tasks.map(({ input, readOnly }) => ({
      agent: input.agent,
      task: input.task,
      readOnly,
      status: "queued",
      activity: [],
      usage: cardUsage(emptyUsage(), 0),
    })),
  };
}

function oneLine(value: unknown): string {
  const text = typeof value === "string" ? value.replace(/\s+/g, " ").trim() : "";
  return text.length > MAX_SUBJECT ? `${text.slice(0, MAX_SUBJECT - 1)}…` : text;
}

/** What the card shows for one tool call: the path, command or pattern it acted on. */
export function summarizeActivity(tool: string, args: unknown): SubagentActivity {
  const input = (args && typeof args === "object" ? args : {}) as Record<string, unknown>;
  switch (tool) {
    case "bash":
      return { tool, subject: oneLine(input.command) };
    case "grep":
    case "find":
      return { tool, subject: oneLine(input.pattern) };
    case "ls":
      return { tool, subject: oneLine(input.path) || "." };
    default:
      return { tool, subject: oneLine(input.path) };
  }
}

export function recordActivity(result: SubagentResult, activity: SubagentActivity): void {
  result.activity = [...result.activity, activity].slice(-MAX_ACTIVITY);
}

export function truncate(text: string, max: number): { text: string; truncated: boolean } {
  if (text.length <= max) return { text, truncated: false };
  return { text: `${text.slice(0, max)}\n\n[… ${text.length - max} more characters]`, truncated: true };
}

const STATUS_WORDS: Record<SubagentResult["status"], string> = {
  queued: "queued",
  running: "running",
  done: "completed",
  failed: "failed",
  aborted: "stopped",
};

function describeFailure(result: SubagentResult): string {
  if (result.status === "aborted") return `${result.agent} was stopped before it finished.`;
  return `${result.agent} failed: ${result.error ?? "no reason was given."}`;
}

/**
 * The text the parent model reads. One child: its answer. Several: a section per child, like
 * Pi's own subagent example, so the model can tell whose findings are whose.
 */
export function modelContent(details: SubagentDetails, outputs: (string | undefined)[]): string {
  const body = (result: SubagentResult, index: number) => {
    const output = outputs[index]?.trim();
    if (result.status === "done") return output ? truncate(output, MAX_MODEL_OUTPUT).text : "(The sub-agent returned no text.)";
    const failure = describeFailure(result);
    return output ? `${failure}\n\nPartial output:\n${truncate(output, MAX_MODEL_OUTPUT).text}` : failure;
  };
  if (details.mode === "single") return body(details.results[0], 0);
  const completed = details.results.filter((result) => result.status === "done").length;
  const sections = details.results.map(
    (result, index) => `### [${result.agent}] ${STATUS_WORDS[result.status]}\n${body(result, index)}`,
  );
  return [`${completed}/${details.results.length} sub-agents completed.`, ...sections].join("\n\n");
}

/** The short text a live update carries next to its details. */
export function progressText(details: SubagentDetails): string {
  const count = (status: SubagentResult["status"]) => details.results.filter((result) => result.status === status).length;
  const parts = [`${count("running")} running`];
  const queued = count("queued");
  if (queued) parts.push(`${queued} queued`);
  const finished = details.results.length - count("running") - queued;
  if (finished) parts.push(`${finished} finished`);
  return `Sub-agents: ${parts.join(", ")}`;
}

/** A structured copy, so a queued update can't observe later mutation of the live object. */
export function snapshotDetails(details: SubagentDetails): SubagentDetails {
  return {
    ...details,
    results: details.results.map((result) => ({ ...result, activity: [...result.activity], usage: { ...result.usage } })),
  };
}

/**
 * Coalesces card updates: children can report a tool call every few milliseconds, but the
 * desktop only needs a few frames a second. `flush` sends anything pending right away.
 */
export function createThrottle(send: () => void, intervalMs = UPDATE_INTERVAL_MS) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last = 0;
  const fire = () => {
    timer = undefined;
    last = Date.now();
    send();
  };
  return {
    schedule() {
      if (timer) return;
      const wait = Math.max(0, intervalMs - (Date.now() - last));
      timer = setTimeout(fire, wait);
    },
    flush() {
      if (timer) clearTimeout(timer);
      fire();
    },
    cancel() {
      if (timer) clearTimeout(timer);
      timer = undefined;
    },
  };
}
