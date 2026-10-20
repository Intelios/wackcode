/**
 * A chat's canvases, folded out of its transcript: every accepted `canvas` call, in order.
 *
 * The call's arguments carry the markup and its result's `details` say which artboard each
 * argument became (`applied`) and what it removed. This mirrors the worker's fold
 * (`worker/src/builtin/canvas/state.ts`) rule for rule; both test suites run the same cases
 * (`fold-cases.json`), so keep them identical. A variant (`basedOn`) takes its base's current
 * markup with its own `css` appended, so revising a base restyles every variant.
 */
import type { NormalizedBlock, NormalizedMessage } from "./types";

export const CANVAS_TOOL_NAME = "canvas";
export const CANVAS_GUIDE_TOOL_NAME = "canvas_guide";
export const CANVAS_LOOK_TOOL_NAME = "canvas_look";
export const CANVAS_TOOL_NAMES = [CANVAS_TOOL_NAME, CANVAS_GUIDE_TOOL_NAME, CANVAS_LOOK_TOOL_NAME] as const;

export interface CanvasArtboard {
  id: string;
  name: string;
  group?: string;
  width: number;
  /** Absent: as tall as its content, measured once drawn. */
  height?: number;
  html?: string;
  basedOn?: string;
  css?: string;
  notes?: string;
}

export interface CanvasBoard {
  id: string;
  title: string;
  /** Creation order; a revision keeps an artboard's place. */
  artboards: CanvasArtboard[];
  /** What the latest accepted call added or revised. */
  lastChanged: string[];
}

interface CanvasApplied {
  index: number;
  id: string;
  op: "add" | "update";
  basedOn?: string;
  /** A new variant that gave no width takes its base's size (the worker records it here). */
  width?: number;
  height?: number;
}

export interface CanvasDetails {
  v: 1;
  canvas: string;
  title: string;
  applied: CanvasApplied[];
  removed: string[];
}

export function isCanvasDetails(value: unknown): value is CanvasDetails {
  if (!value || typeof value !== "object") return false;
  const details = value as Record<string, unknown>;
  return details.v === 1 && typeof details.canvas === "string" && typeof details.title === "string"
    && Array.isArray(details.applied) && Array.isArray(details.removed);
}

type Fields = Partial<Omit<CanvasArtboard, "id">>;
const FIELDS = ["name", "group", "width", "height", "html", "basedOn", "css", "notes"] as const;
const clean = (value: string) => value.replace(/\s+/g, " ").trim();

function fieldsOf(raw: unknown): Fields {
  if (!raw || typeof raw !== "object") return {};
  const input = raw as Record<string, unknown>;
  const fields: Fields = {};
  for (const key of FIELDS) {
    const value = input[key];
    if (key === "width" || key === "height") {
      if (typeof value === "number" && Number.isFinite(value)) fields[key] = Math.round(value);
    } else if (typeof value === "string") {
      fields[key] = key === "name" || key === "group" || key === "notes" ? clean(value) : value;
    }
  }
  return fields;
}

function merge(existing: CanvasArtboard | undefined, id: string, fields: Fields): CanvasArtboard {
  const next: CanvasArtboard = existing ? { ...existing } : { id, name: id, width: 1440 };
  for (const key of FIELDS) {
    const value = fields[key];
    if (value !== undefined) (next as unknown as Record<string, unknown>)[key] = value;
  }
  if (fields.html !== undefined && fields.basedOn === undefined) delete next.basedOn;
  if (fields.basedOn !== undefined && fields.html === undefined) delete next.html;
  return next;
}

/** One accepted call folded into `boards` (replacing the board it touched, never mutating it). */
export function applyCanvasCall(boards: Map<string, CanvasBoard>, args: unknown, details: CanvasDetails): void {
  const entries = Array.isArray((args as { artboards?: unknown } | null)?.artboards) ? (args as { artboards: unknown[] }).artboards : [];
  const previous = boards.get(details.canvas);
  const artboards = [...(previous?.artboards ?? [])];
  for (const applied of details.applied) {
    if (!applied || typeof applied.id !== "string" || typeof applied.index !== "number") continue;
    const fields = fieldsOf(entries[applied.index]);
    if (typeof applied.basedOn === "string") fields.basedOn = applied.basedOn;
    if (typeof applied.width === "number") fields.width = applied.width;
    if (typeof applied.height === "number") fields.height = applied.height;
    const at = artboards.findIndex((artboard) => artboard.id === applied.id);
    const next = merge(at >= 0 ? artboards[at] : undefined, applied.id, fields);
    if (at >= 0) artboards[at] = next;
    else artboards.push(next);
  }
  const removed = new Set(details.removed);
  boards.set(details.canvas, {
    id: details.canvas,
    title: details.title,
    artboards: artboards.filter((artboard) => !removed.has(artboard.id)),
    lastChanged: details.applied.map((applied) => applied.id).filter((id) => !removed.has(id)),
  });
}

/** Every canvas on the displayed branch, in creation order. Refused calls drop out. */
export function canvasesFromMessages(messages: readonly NormalizedMessage[]): Map<string, CanvasBoard> {
  const args = new Map<string, unknown>();
  const boards = new Map<string, CanvasBoard>();
  for (const message of messages) {
    for (const block of message.blocks) {
      if (block.type === "tool-call" && block.toolName === CANVAS_TOOL_NAME && block.toolCallId) {
        args.set(block.toolCallId, block.arguments);
      } else if (block.type === "tool-result" && block.toolName === CANVAS_TOOL_NAME && !block.isError && isCanvasDetails(block.details)) {
        applyCanvasCall(boards, args.get(block.toolCallId ?? ""), block.details);
      }
    }
  }
  return boards;
}

/** An artboard's document: its own markup, or its base's, with its stylesheet after it. */
export function resolveArtboardHtml(board: CanvasBoard, artboard: CanvasArtboard): string {
  const base = artboard.basedOn ? board.artboards.find((candidate) => candidate.id === artboard.basedOn) : undefined;
  const html = (base ? base.html : artboard.html) ?? "";
  return artboard.css?.trim() ? `${html}\n<style>${artboard.css}</style>` : html;
}

/** What one accepted call did, for its card under the turn (`CanvasCard`). */
export interface CanvasTouch {
  toolCallId: string;
  canvas: string;
  title: string;
  added: string[];
  updated: string[];
  removed: string[];
}

export function canvasTouch(call: NormalizedBlock, result: NormalizedBlock | undefined): CanvasTouch | undefined {
  if (!call.toolCallId || !result || result.isError || !isCanvasDetails(result.details)) return undefined;
  const { details } = result;
  return {
    toolCallId: call.toolCallId,
    canvas: details.canvas,
    title: details.title,
    added: details.applied.filter((item) => item.op === "add").map((item) => item.id),
    updated: details.applied.filter((item) => item.op === "update").map((item) => item.id),
    removed: [...details.removed],
  };
}

/** One card per canvas a turn touched: a turn that draws twice on one canvas sums its calls. */
export function mergeTouches(touches: readonly CanvasTouch[]): CanvasTouch[] {
  const byCanvas = new Map<string, CanvasTouch>();
  for (const touch of touches) {
    const seen = byCanvas.get(touch.canvas);
    if (!seen) { byCanvas.set(touch.canvas, { ...touch, added: [...touch.added], updated: [...touch.updated], removed: [...touch.removed] }); continue; }
    seen.title = touch.title;
    for (const id of touch.added) if (!seen.added.includes(id)) seen.added.push(id);
    for (const id of touch.updated) if (!seen.added.includes(id) && !seen.updated.includes(id)) seen.updated.push(id);
    for (const id of touch.removed) {
      seen.added = seen.added.filter((item) => item !== id);
      seen.updated = seen.updated.filter((item) => item !== id);
      if (!seen.removed.includes(id)) seen.removed.push(id);
    }
  }
  return [...byCanvas.values()];
}

/** The plain-text mention an artboard's "Mention in chat" puts in the draft. */
export function artboardMention(board: CanvasBoard, artboard: CanvasArtboard): string {
  return `“${artboard.name}” (canvas “${board.title}”)`;
}

/** The canvas the latest accepted call drew on: the one the header's Canvas button opens. */
export function latestCanvasId(messages: readonly NormalizedMessage[]): string | undefined {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const blocks = messages[index].blocks;
    for (let block = blocks.length - 1; block >= 0; block -= 1) {
      const entry = blocks[block];
      if (entry.type === "tool-result" && entry.toolName === CANVAS_TOOL_NAME && !entry.isError && isCanvasDetails(entry.details)) return entry.details.canvas;
    }
  }
  return undefined;
}

/** What the agent is doing on a canvas right now: a `canvas` or `canvas_look` call still in flight. */
export function canvasActivity(messages: readonly NormalizedMessage[], partial?: NormalizedMessage): "drawing" | "looking" | undefined {
  const pending = (block: NormalizedBlock) => block.type === "tool-call" && (block.toolName === CANVAS_TOOL_NAME || block.toolName === CANVAS_LOOK_TOOL_NAME);
  const last = (blocks: readonly NormalizedBlock[], test: (block: NormalizedBlock) => boolean) => [...blocks].reverse().find(test);
  const streaming = partial ? last(partial.blocks, pending) : undefined;
  if (streaming) return streaming.toolName === CANVAS_LOOK_TOOL_NAME ? "looking" : "drawing";
  const settled = new Set<string>();
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "user") break;
    for (const block of message.blocks) {
      if (block.type === "tool-result" && block.toolCallId) settled.add(block.toolCallId);
    }
    const open = last(message.blocks, (block) => pending(block) && block.toolCallId !== undefined && !settled.has(block.toolCallId));
    if (open) return open.toolName === CANVAS_LOOK_TOOL_NAME ? "looking" : "drawing";
  }
  return undefined;
}
