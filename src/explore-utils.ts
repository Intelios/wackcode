import type { NormalizedBlock, NormalizedMessage } from "./types";
import { CANVAS_TOOL_NAME } from "./canvas-state";
import { COMPARE_IMAGES_TOOL_NAME, completedImageComparison } from "./image-comparisons";
import {
  callArguments,
  completedPlan,
  editStats,
  exploreKind,
  isGitCommit,
  mcpToolParts,
  parseSkillPreviewDetails,
  parseVisualArgs,
  SHOW_VISUAL_TOOL_NAME,
  SKILL_CREATOR_TOOL_NAME,
  SUBAGENT_TOOL_NAME,
  summarizeTool,
  WEB_FETCH_TOOL_NAME,
  WEB_SEARCH_TOOL_NAME,
  type ExploreKind
} from "./tool-utils";

/** Runs shorter than this (counting coalesced tool steps) stay as plain rows or chips. */
export const MIN_GROUP_CALLS = 2;
export const MIN_GROUP_STEPS = 2;

/** Maximum steps visible in the live tail lane below a running collapsed group. */
export const MAX_LIVE_VISIBLE_STEPS = 3;

/** Maximum length for short between-call prose folded into a work group as a note. */
export const MAX_BRIDGING_NARRATION_CHARS = 220;

/** Stable across the streamed and the saved copy of a message, which share Pi's timestamp. */
export function blockKey(message: NormalizedMessage, index: number): string {
  return `${message.timestamp ?? message.id}:${index}`;
}

export interface ExploreItem {
  /** A groupable tool call, reasoning between calls, or short bridging prose note. */
  block: NormalizedBlock;
  /** `blockKey` of the block. */
  key: string;
  /** From the message still streaming. */
  streaming: boolean;
  /** From the message the running prompt is writing: calls without a result are in flight. */
  live: boolean;
}

export interface ExploreGroup {
  /** The first item's call id (or block key): kept stable as the group grows and once saved. */
  key: string;
  items: ExploreItem[];
  /** Live reasoning after the last call of an open run that may yet take another. */
  trailing?: ExploreItem[];
}

export interface CoalescedReadRange {
  start: number;
  end?: number;
}

export interface CoalescedCallStep {
  type: "call";
  key: string;
  flyId: string;
  kind: "read" | "edit" | "write" | "other";
  items: ExploreItem[];
  path?: string;
  ranges?: CoalescedReadRange[];
  live: boolean;
  failed: boolean;
  images: boolean;
  additions: number;
  deletions: number;
  created?: boolean;
}

export type CoalescedStep =
  | CoalescedCallStep
  | { type: "thinking"; key: string; flyId: string; item: ExploreItem; live: boolean }
  | { type: "note"; key: string; item: ExploreItem; text: string };

/** What an assistant message renders, in order: one block, a coalesced single-file run, or a group. */
export type TranscriptSlot =
  | { type: "block"; index: number }
  | { type: "coalesced"; step: CoalescedCallStep; groupKey: string }
  | { type: "explore"; group: ExploreGroup };

export interface TranscriptLayout {
  /** By message id; assistant messages only. */
  messages: Map<string, TranscriptSlot[]>;
  partial?: TranscriptSlot[];
}

interface Unit {
  ordinal: number;
  index: number;
  item: ExploreItem;
  kind: "call" | "thinking" | "note";
}

const isEmptyText = (block: NormalizedBlock) => block.type === "text" && !block.text?.trim();
const nonEmptyStr = (value: unknown) => typeof value === "string" && value.trim() ? value.trim() : undefined;

const kindCache = new WeakMap<NormalizedBlock, ExploreKind | null>();
function kindOf(block: NormalizedBlock): ExploreKind | undefined {
  if (block.type !== "tool-call") return undefined;
  let kind = kindCache.get(block);
  if (kind === undefined) {
    kind = exploreKind(block) ?? null;
    kindCache.set(block, kind);
  }
  return kind ?? undefined;
}

const commitCache = new WeakMap<NormalizedBlock, boolean>();
function isCommitBlock(block: NormalizedBlock): boolean {
  if (block.type !== "tool-call" || block.toolName !== "bash") return false;
  let cached = commitCache.get(block);
  if (cached === undefined) {
    const cmd = callArguments(block).command;
    cached = typeof cmd === "string" && isGitCommit(cmd);
    commitCache.set(block, cached);
  }
  return cached;
}

/**
 * True when a text block is brief, plain, single-paragraph transitional narration between work
 * calls (never headings, lists, fences, tables, or multi-paragraph conclusions).
 */
export function isBridgingNarration(raw: string): boolean {
  const text = raw.trim();
  if (!text || text.length > MAX_BRIDGING_NARRATION_CHARS) return false;
  if (/\r?\n\s*\r?\n/.test(text)) return false;
  if (text.includes("```") || text.includes("~~~")) return false;
  const lines = text.split(/\r?\n/);
  if (lines.length > 2) return false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (/^(#{1,6}\s|>\s?|[-*+]\s|\d+[.)]\s|\|.+\|)/.test(trimmed)) return false;
  }
  return true;
}

/**
 * Calls that render interactive/outcome cards or milestone rows (commits) always break work runs
 * and stand alone in the transcript. Mirrors `renderSpecialCall` and `isGitCommit`.
 */
export function isStandaloneCall(
  block: NormalizedBlock,
  options: { result?: NormalizedBlock; hasResults?: boolean; live?: boolean; running?: boolean } = {}
): boolean {
  if (block.type !== "tool-call") return false;
  const { result, hasResults = false, live = false, running = false } = options;
  switch (block.toolName) {
    case "bash":
      return isCommitBlock(block);
    case SUBAGENT_TOOL_NAME:
      return true;
    case CANVAS_TOOL_NAME:
      return live && running && !result;
    case "plan_mode_complete":
      return !hasResults || completedPlan(result) !== undefined;
    case SHOW_VISUAL_TOOL_NAME:
      return !hasResults ? Boolean(parseVisualArgs(block)) : Boolean(result && !result.isError && parseVisualArgs(block));
    case COMPARE_IMAGES_TOOL_NAME:
      return !hasResults || completedImageComparison(result) !== undefined;
    case SKILL_CREATOR_TOOL_NAME:
      return !hasResults || Boolean(result && !result.isError && parseSkillPreviewDetails(result.details) !== undefined);
    case "ask_user_question":
      return !hasResults || !result;
    default:
      return false;
  }
}

export function readCallRange(call: NormalizedBlock, result?: NormalizedBlock): CoalescedReadRange | undefined {
  const args = callArguments(call);
  const offset = typeof args.offset === "number" && Number.isFinite(args.offset) && args.offset > 0 ? Math.floor(args.offset) : undefined;
  const limit = typeof args.limit === "number" && Number.isFinite(args.limit) && args.limit > 0 ? Math.floor(args.limit) : undefined;
  const notice = result?.text ? /Showing lines (\d+)-(\d+)/.exec(result.text) : null;
  if (notice) return { start: Number(notice[1]), end: Number(notice[2]) };
  if (offset !== undefined) {
    return { start: offset, ...(limit !== undefined ? { end: offset + limit - 1 } : {}) };
  }
  if (limit !== undefined) return { start: 1, end: limit };
  return undefined;
}

function mergeReadRanges(ranges: CoalescedReadRange[]): CoalescedReadRange[] {
  if (ranges.length <= 1) return ranges;
  const sorted = [...ranges].sort((a, b) => a.start - b.start || (a.end ?? a.start) - (b.end ?? b.start));
  const merged: CoalescedReadRange[] = [{ ...sorted[0] }];
  for (let i = 1; i < sorted.length; i += 1) {
    const next = sorted[i];
    const prev = merged[merged.length - 1];
    if (prev.end !== undefined && next.start <= prev.end + 1) {
      prev.end = next.end !== undefined ? Math.max(prev.end, next.end) : prev.end;
    } else if (next.start === prev.start && prev.end === undefined && next.end === undefined) {
      continue;
    } else {
      merged.push({ ...next });
    }
  }
  return merged;
}

export function formatReadRanges(ranges?: CoalescedReadRange[]): string {
  if (!ranges?.length) return "";
  return ranges.map((r) => (r.end !== undefined && r.end !== r.start ? `${r.start}–${r.end}` : `${r.start}`)).join(", ");
}

const EMPTY_RESULTS = new Map<string, NormalizedBlock>();

function isItemPending(item: ExploreItem, results: Map<string, NormalizedBlock>): boolean {
  return item.live && item.block.type === "tool-call" && !(item.block.toolCallId && results.has(item.block.toolCallId));
}

function isWriteNew(call: NormalizedBlock, result?: NormalizedBlock): boolean {
  const details = (result?.details ?? {}) as Record<string, unknown>;
  return details.created === true || details.existed === false;
}

/**
 * Coalesces adjacent same-path `read` calls (merging line ranges) and adjacent same-path
 * `edit`/`write` calls (combining diff stats) inside an item list. Never merges across
 * thinking or note dividers so chronology remains faithful.
 */
export function coalesceWorkItems(items: ExploreItem[], results: Map<string, NormalizedBlock> = EMPTY_RESULTS): CoalescedStep[] {
  const steps: CoalescedStep[] = [];
  for (const item of items) {
    const { block } = item;
    if (block.type === "thinking") {
      steps.push({
        type: "thinking",
        key: item.key,
        flyId: flyId(item),
        item,
        live: item.live && block.durationMs === undefined
      });
      continue;
    }
    if (block.type === "text") {
      steps.push({ type: "note", key: item.key, item, text: block.text?.trim() ?? "" });
      continue;
    }
    if (block.type !== "tool-call") continue;

    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    const args = callArguments(block);
    const path = nonEmptyStr(args.path);
    const name = block.toolName;
    const pending = isItemPending(item, results);
    const failed = result?.isError === true;
    const images = Boolean(result?.images?.length);
    const prev = steps[steps.length - 1];

    if (name === "read") {
      const range = readCallRange(block, result);
      if (path && prev?.type === "call" && prev.kind === "read" && prev.path === path && prev.images === images) {
        prev.items.push(item);
        if (range) prev.ranges = mergeReadRanges([...(prev.ranges ?? []), range]);
        prev.live = prev.live || pending;
        prev.failed = prev.failed || failed;
        continue;
      }
      steps.push({
        type: "call",
        key: block.toolCallId ?? item.key,
        flyId: flyId(item),
        kind: "read",
        items: [item],
        ...(path ? { path } : {}),
        ...(range ? { ranges: [range] } : {}),
        live: pending,
        failed,
        images,
        additions: 0,
        deletions: 0
      });
      continue;
    }

    if (name === "edit" || name === "write") {
      const stats = failed ? undefined : (name === "edit" ? editStats(block, result) : summarizeTool(block, result));
      const add = stats?.additions ?? 0;
      const del = stats?.deletions ?? 0;
      const created = name === "write" ? isWriteNew(block, result) : false;

      if (path && prev?.type === "call" && (prev.kind === "edit" || prev.kind === "write") && prev.path === path) {
        prev.items.push(item);
        prev.live = prev.live || pending;
        prev.failed = prev.failed || failed;
        if (!failed) {
          if (name === "write") {
            prev.additions = add;
            if (prev.created) prev.deletions = 0;
          } else {
            prev.additions += add;
            prev.deletions += del;
            if (!prev.created) prev.kind = "edit";
          }
        }
        continue;
      }

      steps.push({
        type: "call",
        key: block.toolCallId ?? item.key,
        flyId: flyId(item),
        kind: name === "write" ? "write" : "edit",
        items: [item],
        ...(path ? { path } : {}),
        live: pending,
        failed,
        images,
        additions: add,
        deletions: del,
        ...(created ? { created: true } : {})
      });
      continue;
    }

    steps.push({
      type: "call",
      key: block.toolCallId ?? item.key,
      flyId: flyId(item),
      kind: "other",
      items: [item],
      live: pending,
      failed,
      images,
      additions: 0,
      deletions: 0
    });
  }
  return steps;
}

type SlotMark =
  | { type: "explore"; group: ExploreGroup }
  | { type: "coalesced"; step: CoalescedCallStep; groupKey: string }
  | null;

export function layoutTranscript(
  messages: NormalizedMessage[],
  partial: NormalizedMessage | undefined,
  options: { grouping: boolean; liveMessageId?: string; results?: Map<string, NormalizedBlock>; running?: boolean }
): TranscriptLayout {
  const sources = partial ? [...messages, partial] : messages;
  const marks = new Map<number, Map<number, SlotMark>>();
  const results = options.results;
  const hasResults = results !== undefined;

  if (options.grouping) {
    let run: Unit[] = [];
    // Reasoning or short bridging prose after the run's latest call: it joins only if another call follows.
    let trailing: Unit[] = [];

    const setMark = (unit: Unit, mark: SlotMark) => {
      const own = marks.get(unit.ordinal) ?? new Map<number, SlotMark>();
      own.set(unit.index, mark);
      marks.set(unit.ordinal, own);
    };

    const close = (tail: Unit[] = []) => {
      const callUnits = run.filter((unit) => unit.kind === "call");
      if (callUnits.length > 0) {
        const items = run.map((unit) => unit.item);
        const coalesced = coalesceWorkItems(items, results);
        const callSteps = coalesced.filter((step): step is CoalescedCallStep => step.type === "call");
        const groupKey = run[0].item.block.toolCallId ?? run[0].item.key;

        if (callSteps.length >= MIN_GROUP_STEPS) {
          const group: ExploreGroup = { key: groupKey, items };
          const liveThinkingTail = tail.every((unit) => unit.item.live && unit.kind === "thinking") ? tail : [];
          if (liveThinkingTail.length) group.trailing = liveThinkingTail.map((unit) => unit.item);
          [...run, ...liveThinkingTail].forEach((unit, position) => {
            setMark(unit, position === 0 ? { type: "explore", group } : null);
          });
        } else if (callSteps.length === 1 && callSteps[0].items.length > 1) {
          // Repeated calls on the exact same file coalesce into one bare row/chip cleanly.
          // Only drop units that belong to the single coalesced step; leave any trailing thinking intact.
          let first = true;
          for (const unit of run) {
            if (unit.kind !== "call") continue;
            setMark(unit, first ? { type: "coalesced", step: callSteps[0], groupKey } : null);
            first = false;
          }
        }
      }
      run = [];
      trailing = [];
    };

    sources.forEach((message, ordinal) => {
      if (message.role === "tool") return;
      if (message.role !== "assistant" || message.compaction) { close(); return; }
      const streaming = message === partial;
      const live = streaming || (options.liveMessageId !== undefined && message.id === options.liveMessageId);
      const running = options.running ?? (options.liveMessageId !== undefined || partial !== undefined);
      message.blocks.forEach((block, index) => {
        const item: ExploreItem = { block, key: blockKey(message, index), streaming, live };
        const unitBase = { ordinal, index, item };
        if (block.type === "tool-call") {
          const result = block.toolCallId ? results?.get(block.toolCallId) : undefined;
          if (isStandaloneCall(block, { result, hasResults, live, running })) {
            close();
          } else {
            run.push(...trailing, { ...unitBase, kind: "call" });
            trailing = [];
          }
        } else if (block.type === "thinking") {
          if (run.length) trailing.push({ ...unitBase, kind: "thinking" });
        } else if (block.type === "text" && !isEmptyText(block)) {
          const alreadyHasNote = trailing.some((u) => u.kind === "note");
          if (run.length && !alreadyHasNote && isBridgingNarration(block.text ?? "")) {
            trailing.push({ ...unitBase, kind: "note" });
          } else {
            close();
          }
        }
      });
      if (message.stopReason === "error" || message.stopReason === "aborted") close();
    });
    close(trailing.every((unit) => unit.item.live && unit.kind === "thinking") ? trailing : []);
  }

  const slotsOf = (message: NormalizedMessage, ordinal: number): TranscriptSlot[] => {
    const own = marks.get(ordinal);
    const slots: TranscriptSlot[] = [];
    message.blocks.forEach((block, index) => {
      const mark = own?.get(index);
      if (mark) slots.push(mark);
      else if (mark === undefined && !isEmptyText(block)) slots.push({ type: "block", index });
    });
    return slots;
  };

  const layout: TranscriptLayout = { messages: new Map() };
  messages.forEach((message, ordinal) => {
    if (message.role === "assistant") layout.messages.set(message.id, slotsOf(message, ordinal));
  });
  if (partial) layout.partial = slotsOf(partial, messages.length);
  return layout;
}

export interface ExploreCounts {
  files: number;
  images: number;
  searches: number;
  lists: number;
  commands: number;
  failed: number;
}

export function exploreCounts(group: ExploreGroup, results: Map<string, NormalizedBlock>): ExploreCounts {
  const counts: ExploreCounts = { files: 0, images: 0, searches: 0, lists: 0, commands: 0, failed: 0 };
  const files = new Set<string>();
  const images = new Set<string>();
  for (const { block } of group.items) {
    if (block.type !== "tool-call") continue;
    const kind = kindOf(block);
    if (kind === "file") {
      const path = (block.arguments as { path?: unknown } | undefined)?.path;
      const pictured = block.toolCallId !== undefined && Boolean(results.get(block.toolCallId)?.images?.length);
      (pictured ? images : files).add(typeof path === "string" ? `path:${path}` : `call:${JSON.stringify(block.arguments ?? null)}`);
    } else if (kind === "search") counts.searches += 1;
    else if (kind === "list") counts.lists += 1;
    else if (kind === "command") counts.commands += 1;
    if (block.toolCallId && results.get(block.toolCallId)?.isError) counts.failed += 1;
  }
  counts.files = files.size;
  counts.images = images.size;
  return counts;
}

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** "3 files, 1 image, 2 searches, 1 list". */
export function exploreLabel(counts: ExploreCounts): string {
  return [
    counts.files ? plural(counts.files, "file", "files") : "",
    counts.images ? plural(counts.images, "image", "images") : "",
    counts.searches ? plural(counts.searches, "search", "searches") : "",
    counts.lists ? plural(counts.lists, "list", "lists") : "",
    counts.commands ? plural(counts.commands, "command", "commands") : ""
  ].filter(Boolean).join(", ");
}

const NOUNS: [keyof Omit<ExploreCounts, "failed">, string, string][] = [
  ["files", "file", "files"],
  ["images", "image", "images"],
  ["searches", "search", "searches"],
  ["lists", "list", "lists"],
  ["commands", "command", "commands"]
];

/** `exploreLabel` in pieces, so the head can roll each number as calls join. */
export function exploreLabelParts(counts: ExploreCounts): { key: string; count: number; noun: string }[] {
  return NOUNS.filter(([key]) => counts[key] > 0).map(([key, one, many]) => ({ key, count: counts[key], noun: counts[key] === 1 ? one : many }));
}

export type WorkBucketKey =
  | "create"
  | "edit"
  | "write"
  | "command"
  | "browser"
  | "computer"
  | "mcp"
  | "canvas"
  | "todo"
  | "ask"
  | "read"
  | "image"
  | "search"
  | "list"
  | "web_search"
  | "web_fetch"
  | "memory"
  | "other";

export interface WorkSummaryPart {
  key: string;
  tier: 1 | 2 | 3;
  leadVerb: string;
  trailVerb: string;
  count?: number;
  noun: string;
  leadPhrase?: string;
  trailPhrase?: string;
}

export interface WorkSummary {
  isPureExploration: boolean;
  /** Pure-exploration counts & parts (for backward-compatible read-only heads). */
  exploreCounts: ExploreCounts;
  exploreParts: { key: string; count: number; noun: string }[];
  /** Priority-sorted tiered parts for mixed/mutation work groups. */
  parts: WorkSummaryPart[];
  overflowCount: number;
  notes: number;
  additions: number;
  deletions: number;
  failed: number;
}

export function isPureExplorationGroup(group: ExploreGroup): boolean {
  for (const item of group.items) {
    if (item.block.type === "text") return false;
    if (item.block.type === "tool-call" && !kindOf(item.block)) return false;
  }
  return true;
}

function workBucketForStep(step: CoalescedCallStep, results: Map<string, NormalizedBlock>): WorkBucketKey {
  const firstCall = step.items[0].block;
  const name = firstCall.toolName ?? "";
  if (step.kind === "write") {
    return step.created ? "create" : "write";
  }
  if (step.kind === "edit") return "edit";
  if (step.kind === "read" || name === "read_image" || name === "list_images") {
    return step.images ? "image" : "read";
  }
  if (name === "bash") {
    const k = kindOf(firstCall);
    if (k === "file") return "read";
    if (k === "search") return "search";
    if (k === "list") return "list";
    return "command";
  }
  if (name === "grep" || name === "find") return "search";
  if (name === "ls") return "list";
  if (name === WEB_SEARCH_TOOL_NAME) return "web_search";
  if (name === WEB_FETCH_TOOL_NAME) return "web_fetch";
  if (name === "memory_save" || name === "memory_recall" || name === "memory_forget") return "memory";
  if (name.startsWith("browser_")) return "browser";
  if (name.startsWith("computer_")) return "computer";
  if (name.startsWith("canvas")) return "canvas";
  if (name === "todo") return "todo";
  if (name === "ask_user_question") return "ask";
  if (mcpToolParts(name)) return "mcp";
  // Unused param guard kept clean; results used below for failure/memory check if needed.
  void results;
  return "other";
}

interface BucketAcc {
  key: WorkBucketKey;
  tier: 1 | 2 | 3;
  order: number;
  paths: Set<string>;
  pathless: number;
  count: number;
  memoryRead?: boolean;
  memoryWrite?: boolean;
}

const BUCKET_RANK: Record<WorkBucketKey, number> = {
  create: 0,
  edit: 1,
  write: 2,
  command: 3,
  browser: 4,
  computer: 5,
  mcp: 6,
  canvas: 7,
  todo: 8,
  ask: 9,
  read: 10,
  image: 11,
  search: 12,
  list: 13,
  web_search: 14,
  web_fetch: 15,
  memory: 16,
  other: 17
};

const BUCKET_TIER: Record<WorkBucketKey, 1 | 2 | 3> = {
  create: 1,
  edit: 1,
  write: 1,
  command: 2,
  browser: 2,
  computer: 2,
  mcp: 2,
  canvas: 2,
  todo: 2,
  ask: 2,
  read: 3,
  image: 3,
  search: 3,
  list: 3,
  web_search: 3,
  web_fetch: 3,
  memory: 3,
  other: 3
};

function formatBucketPart(acc: BucketAcc): WorkSummaryPart {
  const total = acc.paths.size || acc.pathless ? acc.paths.size + acc.pathless : acc.count;
  const word = (one: string, many: string) => total === 1 ? one : many;
  switch (acc.key) {
    case "create":
      return { key: "create", tier: 1, leadVerb: "Created", trailVerb: "created", count: total, noun: word("file", "files") };
    case "edit":
      return { key: "edit", tier: 1, leadVerb: "Edited", trailVerb: "edited", count: total, noun: word("file", "files") };
    case "write":
      return { key: "write", tier: 1, leadVerb: "Wrote", trailVerb: "wrote", count: total, noun: word("file", "files") };
    case "command":
      return { key: "command", tier: 2, leadVerb: "Ran", trailVerb: "ran", count: total, noun: word("command", "commands") };
    case "browser":
      return { key: "browser", tier: 2, leadVerb: "Browsed", trailVerb: "browsed", count: total, noun: word("step", "steps") };
    case "computer":
      if (total === 1) return { key: "computer", tier: 2, leadVerb: "Used", trailVerb: "used", noun: "", leadPhrase: "Used computer", trailPhrase: "used computer" };
      return { key: "computer", tier: 2, leadVerb: "Used", trailVerb: "used", count: total, noun: "app actions" };
    case "mcp":
      return { key: "mcp", tier: 2, leadVerb: "Called", trailVerb: "called", count: total, noun: word("MCP tool", "MCP tools") };
    case "canvas":
      return total === 1
        ? { key: "canvas", tier: 2, leadVerb: "Updated", trailVerb: "updated", noun: "", leadPhrase: "Updated canvas", trailPhrase: "updated canvas" }
        : { key: "canvas", tier: 2, leadVerb: "Updated", trailVerb: "updated", count: total, noun: "canvas steps" };
    case "todo":
      return { key: "todo", tier: 2, leadVerb: "Updated", trailVerb: "updated", noun: "", leadPhrase: "Updated todos", trailPhrase: "updated todos" };
    case "ask":
      return total === 1
        ? { key: "ask", tier: 2, leadVerb: "Asked", trailVerb: "asked", noun: "", leadPhrase: "Asked you", trailPhrase: "asked you" }
        : { key: "ask", tier: 2, leadVerb: "Asked", trailVerb: "asked", count: total, noun: "questions" };
    case "read":
      return { key: "read", tier: 3, leadVerb: "Read", trailVerb: "read", count: total, noun: word("file", "files") };
    case "image":
      return { key: "image", tier: 3, leadVerb: "Viewed", trailVerb: "viewed", count: total, noun: word("image", "images") };
    case "search":
      return { key: "search", tier: 3, leadVerb: "Searched", trailVerb: "searched", count: total, noun: word("search", "searches") };
    case "list":
      return { key: "list", tier: 3, leadVerb: "Listed", trailVerb: "listed", count: total, noun: word("folder", "folders") };
    case "web_search":
      if (total === 1) return { key: "web_search", tier: 3, leadVerb: "Searched", trailVerb: "searched", noun: "", leadPhrase: "Searched the web", trailPhrase: "searched the web" };
      return { key: "web_search", tier: 3, leadVerb: "Searched the web", trailVerb: "searched the web", count: total, noun: "times" };
    case "web_fetch":
      return { key: "web_fetch", tier: 3, leadVerb: "Fetched", trailVerb: "fetched", count: total, noun: word("page", "pages") };
    case "memory": {
      const phrase = acc.memoryRead && acc.memoryWrite ? "read and updated memory" : acc.memoryWrite ? "updated memory" : "read memory";
      return {
        key: "memory",
        tier: 3,
        leadVerb: "",
        trailVerb: "",
        noun: "",
        leadPhrase: phrase[0].toUpperCase() + phrase.slice(1),
        trailPhrase: phrase
      };
    }
    case "other":
      return { key: "other", tier: 3, leadVerb: "Completed", trailVerb: "completed", count: total, noun: word("step", "steps") };
  }
}

/**
 * Tiered, path-deduplicated summary for any unified work group.
 * Pure read-only exploration preserves `"Explored N files, M searches"`; mixed/mutation work
 * prioritizes mutations (`Edited`) > commands/integrations > reads/searches and totals `+A −D`.
 */
export function summarizeWorkGroup(group: ExploreGroup, results: Map<string, NormalizedBlock>): WorkSummary {
  const pure = isPureExplorationGroup(group);
  const eCounts = exploreCounts(group, results);
  const eParts = exploreLabelParts(eCounts);
  const steps = coalesceWorkItems(group.items, results);

  const buckets = new Map<WorkBucketKey, BucketAcc>();
  let order = 0;
  let notes = 0;
  let additions = 0;
  let deletions = 0;
  let failed = 0;
  const createdPaths = new Set<string>();

  for (const step of steps) {
    if (step.type === "note") {
      notes += 1;
      continue;
    }
    if (step.type !== "call") continue;
    additions += step.additions;
    deletions += step.deletions;
    for (const item of step.items) {
      if (item.block.toolCallId && results.get(item.block.toolCallId)?.isError) failed += 1;
    }

    let bKey = workBucketForStep(step, results);
    if (bKey === "create" && step.path) createdPaths.add(step.path);
    // If a file created earlier in the same group is later edited, keep it counted once under edits/creates.
    if (bKey === "edit" && step.path && createdPaths.has(step.path)) bKey = "create";
    // When both `edit` and existing-file `write` happen, merge existing-file writes cleanly into `edit`.
    const existing = buckets.get(bKey);
    const acc = existing ?? { key: bKey, tier: BUCKET_TIER[bKey], order: order++, paths: new Set<string>(), pathless: 0, count: 0 };
    if (!existing) buckets.set(bKey, acc);

    acc.count += 1;
    if (bKey === "create" || bKey === "edit" || bKey === "write" || bKey === "read" || bKey === "image") {
      if (step.path) acc.paths.add(step.path);
      else acc.pathless += 1;
    }
    if (bKey === "memory") {
      const name = step.items[0].block.toolName;
      if (name === "memory_recall") acc.memoryRead = true;
      else acc.memoryWrite = true;
    }
  }

  // Merge non-created `write` into `edit` if `edit` is also present so we don't say "Edited 1 file, wrote 1 file".
  const editAcc = buckets.get("edit");
  const writeAcc = buckets.get("write");
  if (editAcc && writeAcc) {
    for (const path of writeAcc.paths) editAcc.paths.add(path);
    editAcc.pathless += writeAcc.pathless;
    editAcc.count += writeAcc.count;
    buckets.delete("write");
  }

  const countOf = (acc: BucketAcc) => (acc.paths.size || acc.pathless ? acc.paths.size + acc.pathless : acc.count);
  const sorted = [...buckets.values()].sort((a, b) =>
    a.tier - b.tier || countOf(b) - countOf(a) || BUCKET_RANK[a.key] - BUCKET_RANK[b.key] || a.order - b.order);

  const maxParts = sorted.length > 3 ? 2 : 3;
  const shown = sorted.slice(0, maxParts);
  const hidden = sorted.slice(maxParts);
  const overflowCount = hidden.reduce((sum, acc) => sum + countOf(acc), 0);

  return {
    isPureExploration: pure,
    exploreCounts: eCounts,
    exploreParts: eParts,
    parts: shown.map(formatBucketPart),
    overflowCount,
    notes,
    additions,
    deletions,
    failed
  };
}

export function workSummaryText(summary: WorkSummary): string {
  if (summary.isPureExploration) {
    const label = exploreLabel(summary.exploreCounts);
    return label ? `Explored ${label}` : "Explored";
  }
  const clauses = summary.parts.map((part, index) => {
    if (index === 0 && part.leadPhrase) return part.leadPhrase;
    if (index > 0 && part.trailPhrase) return part.trailPhrase;
    const verb = index === 0 ? part.leadVerb : part.trailVerb;
    const body = part.count !== undefined ? `${part.count} ${part.noun}` : part.noun;
    return [verb, body].filter(Boolean).join(" ");
  });
  if (summary.overflowCount > 0) {
    clauses.push(`and ${summary.overflowCount} ${summary.overflowCount === 1 ? "more step" : "more steps"}`);
  }
  const head = clauses.join(", ") || "Worked";
  const suffix: string[] = [];
  if (summary.notes > 0) suffix.push(`${summary.notes} ${summary.notes === 1 ? "note" : "notes"}`);
  return suffix.length ? `${head} · ${suffix.join(" · ")}` : head;
}

/**
 * The calls still in flight. They show inside the live lane beneath the head rather than inside it,
 * and fly into the head as they finish.
 */
export function exploreLiveItems(group: ExploreGroup, results: Map<string, NormalizedBlock>): ExploreItem[] {
  return group.items.filter((item) => isItemPending(item, results));
}

/** The group without its calls in flight: what has already joined the head. */
export function settledGroup(group: ExploreGroup, results: Map<string, NormalizedBlock>): ExploreGroup {
  const items = group.items.filter((item) => !isItemPending(item, results));
  return items.length === group.items.length ? group : { ...group, items };
}

export interface LiveTailWindow {
  steps: CoalescedCallStep[];
  hiddenCount: number;
}

/**
 * Caps live steps rendered directly under a running group's head to `maxVisible`:
 * always prioritizes in-flight steps and pins any failed calls while capping at 3.
 */
export function liveTailWindow(
  group: ExploreGroup,
  results: Map<string, NormalizedBlock>,
  maxVisible = MAX_LIVE_VISIBLE_STEPS,
  options: { includeFailed?: boolean } = {}
): LiveTailWindow {
  const { includeFailed = true } = options;
  const allSteps = coalesceWorkItems(group.items, results).filter((s): s is CoalescedCallStep => s.type === "call");
  const hasLive = group.items.some((i) => i.live);
  const relevant = allSteps.filter((step) => step.live || (includeFailed && hasLive && step.failed));
  if (relevant.length <= maxVisible) return { steps: relevant, hiddenCount: 0 };

  // Keep failed steps first, then fill remaining slots with the newest in-flight steps in chronological order.
  const chosen = new Set<CoalescedCallStep>();
  const failedSteps = relevant.filter((s) => s.failed);
  for (const s of failedSteps.slice(-maxVisible)) chosen.add(s);
  for (let i = relevant.length - 1; i >= 0 && chosen.size < maxVisible; i -= 1) {
    chosen.add(relevant[i]);
  }
  const steps = relevant.filter((s) => chosen.has(s));
  return { steps, hiddenCount: relevant.length - steps.length };
}

/**
 * The shared-element id a block flies under when it joins a group: the chip or reasoning row
 * outside the group and its landing mark in the head carry the same id (`ExploreGroup.tsx`).
 */
export function flyId(item: Pick<ExploreItem, "block" | "key">): string {
  return item.block.type === "thinking" ? `think:${item.key}` : `call:${item.block.toolCallId ?? item.key}`;
}

export type ExploreMark =
  | ExploreKind
  | "edit"
  | "write"
  | "image"
  | "web"
  | "browser"
  | "computer"
  | "memory"
  | "mcp"
  | "other"
  | "note"
  | "thought";

const MARK_ORDER: ExploreMark[] = [
  "write",
  "edit",
  "file",
  "image",
  "search",
  "list",
  "command",
  "web",
  "browser",
  "computer",
  "memory",
  "mcp",
  "other",
  "note",
  "thought"
];

function markForItem(item: ExploreItem, results: Map<string, NormalizedBlock>): ExploreMark | undefined {
  const { block } = item;
  if (block.type === "thinking") return "thought";
  if (block.type === "text") return "note";
  if (block.type !== "tool-call") return undefined;
  const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
  const name = block.toolName ?? "";
  if (name === "write") return "write";
  if (name === "edit") return "edit";
  const explore = kindOf(block);
  if (explore) return explore === "file" && result?.images?.length ? "image" : explore;
  if (name === "read_image" || name === "list_images") return result?.images?.length ? "image" : "file";
  if (name === "bash") return "command";
  if (name === WEB_FETCH_TOOL_NAME || name === WEB_SEARCH_TOOL_NAME) return "web";
  if (name.startsWith("browser_")) return "browser";
  if (name.startsWith("computer_")) return "computer";
  if (name === "memory_save" || name === "memory_recall" || name === "memory_forget") return "memory";
  if (mcpToolParts(name)) return "mcp";
  return "other";
}

/**
 * The head's icon stack: one mark per kind the group holds, in a fixed order so marks never
 * shuffle, each carrying the fly id of the newest item of its kind.
 */
export function exploreMarks(group: ExploreGroup, results: Map<string, NormalizedBlock>): { mark: ExploreMark; id: string }[] {
  const latest = new Map<ExploreMark, string>();
  for (const item of group.items) {
    const mark = markForItem(item, results);
    if (mark) latest.set(mark, flyId(item));
  }
  return MARK_ORDER.filter((mark) => latest.has(mark)).map((mark) => ({ mark, id: latest.get(mark)! }));
}

/**
 * The group a running transcript currently ends with, if any: the last assistant slot drawn
 * since the latest user or system message.
 */
export function tailGroupKey(messages: NormalizedMessage[], layout: TranscriptLayout): string | undefined {
  const last = (slots?: TranscriptSlot[]) => slots?.length ? slots[slots.length - 1] : undefined;
  let slot = last(layout.partial);
  for (let index = messages.length - 1; !slot && index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "user" || message.role === "system") break;
    if (message.role === "assistant") slot = last(layout.messages.get(message.id));
  }
  return slot?.type === "explore" ? slot.group.key : undefined;
}

const MULTI_ACTIVE: Partial<Record<string, [string, string]>> = {
  read: ["Reading", "files"],
  edit: ["Editing", "files"],
  write: ["Writing", "files"],
  bash: ["Running", "commands"],
  grep: ["Searching", "locations"],
  find: ["Finding", "files"],
  web_fetch: ["Fetching", "pages"],
  web_search: ["Searching the web", "times"]
};

/** The call(s) in flight, as the header shows while running: "Reading src/App.tsx" or "Editing 2 files". */
export function exploreActivity(group: ExploreGroup, results: Map<string, NormalizedBlock>): string | undefined {
  const pending = exploreLiveItems(group, results);
  if (!pending.length) return undefined;
  if (pending.length > 1) {
    const first = pending[0].block.toolName;
    if (first && pending.every((item) => item.block.toolName === first)) {
      const pair = MULTI_ACTIVE[first];
      if (pair) return `${pair[0]} ${pending.length} ${pair[1]}`;
    }
  }
  const latest = pending[pending.length - 1];
  const summary = summarizeTool(latest.block);
  return summary.subject ? `${summary.activeVerb} ${summary.subject}` : summary.activeVerb;
}
