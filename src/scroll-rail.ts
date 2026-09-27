/**
 * Pure geometry for the transcript's scroll rail, which replaces the native
 * scrollbar. Everything the rail draws is derived from the scroll container's
 * metrics here so the mapping stays testable.
 */

export interface RailTurn {
  /** Matches the `data-turn` attribute on the turn's .msg.user element. */
  id: string;
  /** One-line prompt excerpt for the hover chip and aria-label. */
  excerpt: string;
}

export interface RailMetrics {
  scrollTop: number;
  clientHeight: number;
  scrollHeight: number;
  /** Pixel height of the rail itself. */
  railHeight: number;
}

/** The rail shows only when the transcript actually overflows. */
export function hasOverflow(scrollHeight: number, clientHeight: number): boolean {
  return scrollHeight > clientHeight + 1;
}

/** Rail position for a scroll-content offset, clamped inside the rail. */
export function tickOffset(elementTop: number, scrollHeight: number, railHeight: number): number {
  if (scrollHeight <= 0) return 0;
  return Math.min(railHeight, Math.max(0, (elementTop / scrollHeight) * railHeight));
}

/** Where the lit portion of the journey line ends: the viewport's bottom edge — everything past it has been read. */
export function readLineY(metrics: RailMetrics): number {
  const { scrollTop, clientHeight, scrollHeight, railHeight } = metrics;
  return tickOffset(scrollTop + clientHeight, scrollHeight, railHeight);
}

/** Inverse of tickOffset with viewport centring: the scrollTop a rail click/scrub at `railY` aims at. */
export function scrollTopForRailPoint(railY: number, railHeight: number, scrollHeight: number, clientHeight: number): number {
  if (railHeight <= 0) return 0;
  const scrollable = Math.max(0, scrollHeight - clientHeight);
  const target = Math.min(1, Math.max(0, railY / railHeight)) * scrollHeight - clientHeight / 2;
  return Math.min(scrollable, Math.max(0, target));
}

/**
 * The turn the viewport is "reading": the last one whose top is above a line a
 * third of the way down the viewport, not the very top — the tick lights up as
 * soon as its prompt is in view, not once it scrolls off. `tops` is sorted
 * ascending (document order). Returns -1 when no turn is that far up yet.
 */
export function currentTurnIndex(tops: number[], scrollTop: number, clientHeight: number): number {
  const line = scrollTop + clientHeight * 0.35;
  let current = -1;
  for (let index = 0; index < tops.length; index++) {
    if (tops[index] <= line) current = index;
    else break;
  }
  return current;
}

/** ScrollTop a tick click aims at: the turn's top just below the transcript's top edge. */
export function scrollTopFor(elementTop: number, scrollHeight: number, clientHeight: number): number {
  return Math.min(Math.max(0, scrollHeight - clientHeight), Math.max(0, elementTop - 20));
}

/** Single line, single spaces, ellipsised — for the hover chip and aria-label. */
export function turnExcerpt(text: string, max = 60): string {
  const clean = text.replace(/\s+/g, " ").trim();
  return clean.length <= max ? clean : `${clean.slice(0, max - 1).trimEnd()}…`;
}
