/**
 * Where a canvas's artboards sit, in world pixels. The agent doesn't place artboards; groups do:
 * each group is a row in the order it first appears, its artboards left to right in creation
 * order, and rows stack top to bottom. Adding an artboard therefore only ever extends a row or
 * adds one below; nothing already on the board moves sideways. An auto-height artboard takes its
 * measured height once its frame reports one.
 */
import type { CanvasBoard } from "./canvas-state";

/** Between artboards in a row. */
export const ARTBOARD_GAP = 64;
/** Between one row's bottom and the next row's heading. */
export const ROW_GAP = 120;
/** A row's group heading, above its labels. */
export const GROUP_BAND = 44;
/** Each artboard's name label, above its frame. */
export const LABEL_BAND = 30;
/** What an auto-height artboard is drawn at until it reports its size. */
export const AUTO_HEIGHT = 600;
const MIN_HEIGHT = 200;
const MAX_HEIGHT = 4000;

export interface PlacedArtboard {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PlacedGroup {
  label?: string;
  x: number;
  y: number;
  width: number;
}

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CanvasLayout {
  artboards: PlacedArtboard[];
  groups: PlacedGroup[];
  bounds: Bounds;
}

export function artboardHeight(height: number | undefined, measured: number | undefined): number {
  const value = height ?? measured ?? AUTO_HEIGHT;
  return Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, Math.round(value)));
}

export function layoutCanvas(board: CanvasBoard, measured: Readonly<Record<string, number>> = {}): CanvasLayout {
  const rows = new Map<string, CanvasBoard["artboards"]>();
  for (const artboard of board.artboards) {
    const key = artboard.group ?? "";
    rows.set(key, [...(rows.get(key) ?? []), artboard]);
  }
  const artboards: PlacedArtboard[] = [];
  const groups: PlacedGroup[] = [];
  let y = 0;
  let width = 0;
  for (const [label, members] of rows) {
    const top = y + (label ? GROUP_BAND : 0) + LABEL_BAND;
    let x = 0;
    let tallest = 0;
    for (const artboard of members) {
      const height = artboardHeight(artboard.height, measured[artboard.id]);
      artboards.push({ id: artboard.id, x, y: top, width: artboard.width, height });
      x += artboard.width + ARTBOARD_GAP;
      tallest = Math.max(tallest, height);
    }
    const rowWidth = Math.max(0, x - ARTBOARD_GAP);
    groups.push({ ...(label ? { label } : {}), x: 0, y, width: rowWidth });
    width = Math.max(width, rowWidth);
    y = top + tallest + ROW_GAP;
  }
  const height = Math.max(0, y - ROW_GAP);
  return { artboards, groups, bounds: { x: 0, y: 0, width, height } };
}

/** The smallest box around some placed artboards (labels included), or undefined for none. */
export function boundsOf(placed: readonly PlacedArtboard[]): Bounds | undefined {
  if (!placed.length) return undefined;
  const left = Math.min(...placed.map((item) => item.x));
  const top = Math.min(...placed.map((item) => item.y - LABEL_BAND));
  const right = Math.max(...placed.map((item) => item.x + item.width));
  const bottom = Math.max(...placed.map((item) => item.y + item.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}
