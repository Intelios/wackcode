import type { NormalizedBlock, NormalizedMessage } from "./types";
import { exploreKind, summarizeTool, type ExploreKind } from "./tool-utils";

/**
 * Folding exploration in the transcript: a run of read-only tool calls (reads, searches,
 * listings, read-only shell commands; `exploreKind`) shows as one collapsible "Explored" row.
 *
 * Pi usually makes one or two calls per assistant message, with the tool results in `tool`
 * messages between them, so a run spans messages. It belongs to the message holding its first
 * call, which renders the whole group; the later messages leave its blocks out and render
 * nothing when that was all they had. A run:
 * - is broken by prose, any other tool call, a user or system message, or an error or abort;
 * - passes over `tool` messages and empty text;
 * - takes in reasoning that sits between two of its calls (reasoning before its first call or
 *   after its last stays outside, so a model that thinks between calls still gets one group);
 * - while it is still open at the end of a live run, holds the reasoning after its last call as
 *   `trailing`: the group shows it in its lane under the head, where the next call would land, so
 *   the space under the head doesn't swap between a row outside and a chip inside as the model
 *   alternates. Once something else closes the run, that reasoning is back outside;
 * - is folded only from `MIN_GROUP_CALLS` calls: a lone read stays a plain row.
 */

export const MIN_GROUP_CALLS = 2;

/** Stable across the streamed and the saved copy of a message, which share Pi's timestamp. */
export function blockKey(message: NormalizedMessage, index: number): string {
  return `${message.timestamp ?? message.id}:${index}`;
}

export interface ExploreItem {
  /** An exploration tool call, or reasoning between two of them. */
  block: NormalizedBlock;
  /** `blockKey` of the block. */
  key: string;
  /** From the message still streaming. */
  streaming: boolean;
  /** From the message the running prompt is writing: calls without a result are in flight. */
  live: boolean;
}

export interface ExploreGroup {
  /** The first call's id: the group keeps it as it grows and once its messages are saved. */
  key: string;
  items: ExploreItem[];
  /** Live reasoning after the last call of a run that may yet take another (see above). */
  trailing?: ExploreItem[];
}

/** What an assistant message renders, in order: one of its own blocks, or a whole group. */
export type TranscriptSlot = { type: "block"; index: number } | { type: "explore"; group: ExploreGroup };

export interface TranscriptLayout {
  /** By message id; assistant messages only. */
  messages: Map<string, TranscriptSlot[]>;
  partial?: TranscriptSlot[];
}

interface Unit {
  ordinal: number;
  index: number;
  item: ExploreItem;
  call: boolean;
}

const isEmptyText = (block: NormalizedBlock) => block.type === "text" && !block.text?.trim();

/**
 * `exploreKind` by block. The layout runs on every streamed event over the whole transcript,
 * and a block object never changes once the app holds it (a streamed message arrives as new
 * objects each time), so shell commands are parsed once.
 */
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

export function layoutTranscript(
  messages: NormalizedMessage[],
  partial: NormalizedMessage | undefined,
  options: { grouping: boolean; liveMessageId?: string }
): TranscriptLayout {
  const sources = partial ? [...messages, partial] : messages;
  // By message ordinal, then block index: the group a run's first block opens, or null for
  // the run's other blocks, which that group renders.
  const marks = new Map<number, Map<number, ExploreGroup | null>>();

  if (options.grouping) {
    let run: Unit[] = [];
    // Reasoning after the run's latest call: it joins only if another call follows.
    let trailing: Unit[] = [];
    const close = (tail: Unit[] = []) => {
      if (run.filter((unit) => unit.call).length >= MIN_GROUP_CALLS) {
        const group: ExploreGroup = { key: run[0].item.block.toolCallId ?? run[0].item.key, items: run.map((unit) => unit.item) };
        if (tail.length) group.trailing = tail.map((unit) => unit.item);
        [...run, ...tail].forEach((unit, position) => {
          const own = marks.get(unit.ordinal) ?? new Map<number, ExploreGroup | null>();
          own.set(unit.index, position === 0 ? group : null);
          marks.set(unit.ordinal, own);
        });
      }
      run = [];
      trailing = [];
    };

    sources.forEach((message, ordinal) => {
      if (message.role === "tool") return;
      if (message.role !== "assistant") { close(); return; }
      const streaming = message === partial;
      const live = streaming || (options.liveMessageId !== undefined && message.id === options.liveMessageId);
      message.blocks.forEach((block, index) => {
        const unit = { ordinal, index, item: { block, key: blockKey(message, index), streaming, live } };
        if (kindOf(block)) {
          run.push(...trailing, { ...unit, call: true });
          trailing = [];
        } else if (block.type === "thinking") {
          if (run.length) trailing.push({ ...unit, call: false });
        } else if (!isEmptyText(block)) {
          close();
        }
      });
      if (message.stopReason === "error" || message.stopReason === "aborted") close();
    });
    close(trailing.every((unit) => unit.item.live) ? trailing : []);
  }

  const slotsOf = (message: NormalizedMessage, ordinal: number): TranscriptSlot[] => {
    const own = marks.get(ordinal);
    const slots: TranscriptSlot[] = [];
    message.blocks.forEach((block, index) => {
      const group = own?.get(index);
      if (group) slots.push({ type: "explore", group });
      // Empty text renders nothing, and would keep an otherwise folded message on screen.
      else if (group === undefined && !isEmptyText(block)) slots.push({ type: "block", index });
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
  /** Distinct files read as text. */
  files: number;
  /** Distinct files read as pictures: reads whose result came back with an image. */
  images: number;
  searches: number;
  lists: number;
  /** Other read-only shell commands, such as `git log`. */
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
    }
    else if (kind === "search") counts.searches += 1;
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

const isPending = (item: ExploreItem, results: Map<string, NormalizedBlock>) =>
  item.live && item.block.type === "tool-call" && !(item.block.toolCallId && results.has(item.block.toolCallId));

/**
 * The calls still in flight. They show as live chips beneath the head rather than inside it,
 * and fly into the head as they finish.
 */
export function exploreLiveItems(group: ExploreGroup, results: Map<string, NormalizedBlock>): ExploreItem[] {
  return group.items.filter((item) => isPending(item, results));
}

/** The group without its calls in flight: what has already joined the head. */
export function settledGroup(group: ExploreGroup, results: Map<string, NormalizedBlock>): ExploreGroup {
  const items = group.items.filter((item) => !isPending(item, results));
  return items.length === group.items.length ? group : { ...group, items };
}

/**
 * The shared-element id a block flies under when it joins a group: the chip or reasoning row
 * outside the group and its landing mark in the head carry the same id (`ExploreGroup.tsx`).
 */
export function flyId(item: Pick<ExploreItem, "block" | "key">): string {
  return item.block.type === "thinking" ? `think:${item.key}` : `call:${item.block.toolCallId ?? item.key}`;
}

export type ExploreMark = ExploreKind | "image" | "thought";
const MARK_ORDER: ExploreMark[] = ["file", "image", "search", "list", "command", "thought"];

/**
 * The head's icon stack: one mark per kind the group holds, in a fixed order so marks never
 * shuffle, each carrying the fly id of the newest item of its kind (the one that last landed).
 * Pictured reads count as images, as in `exploreCounts`.
 */
export function exploreMarks(group: ExploreGroup, results: Map<string, NormalizedBlock>): { mark: ExploreMark; id: string }[] {
  const latest = new Map<ExploreMark, string>();
  for (const item of group.items) {
    const { block } = item;
    let mark: ExploreMark | undefined;
    if (block.type === "thinking") mark = "thought";
    else {
      mark = kindOf(block);
      if (mark === "file" && block.toolCallId && results.get(block.toolCallId)?.images?.length) mark = "image";
    }
    if (mark) latest.set(mark, flyId(item));
  }
  return MARK_ORDER.filter((mark) => latest.has(mark)).map((mark) => ({ mark, id: latest.get(mark)! }));
}

/**
 * The group a running transcript currently ends with, if any: the last assistant slot drawn
 * since the latest user or system message. While the run waits on the model, that group's lane
 * holds the wait instead of a separate "Working…" row beneath it.
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

/** The call in flight, as the header shows it while the group runs: "Reading src/App.tsx". */
export function exploreActivity(group: ExploreGroup, results: Map<string, NormalizedBlock>): string | undefined {
  const pending = exploreLiveItems(group, results);
  const latest = pending[pending.length - 1];
  if (!latest) return undefined;
  const summary = summarizeTool(latest.block);
  return summary.subject ? `${summary.activeVerb} ${summary.subject}` : summary.activeVerb;
}
