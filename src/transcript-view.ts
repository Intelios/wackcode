/** DOM-free, session-only reading state. Anchors carry viewport offsets so a
 * background completion/fold can change row heights without moving the reader. */
export interface ScrollPosition {
  following: boolean;
  top: number;
  anchors: { key: string; offset: number }[];
}

export interface TranscriptViewState {
  position?: ScrollPosition;
  work?: Set<string>;
  compactions?: Set<string>;
  thinking?: Set<string>;
  exploration?: Set<string>;
  tools?: Set<string>;
}

export function captureScroll(element: HTMLElement, following: boolean): ScrollPosition {
  const top = element.getBoundingClientRect().top;
  const anchors = following ? [] : Array.from(element.querySelectorAll<HTMLElement>("[data-transcript-anchor]"))
    .map((node) => ({ key: node.dataset.transcriptAnchor!, offset: node.getBoundingClientRect().top - top }))
    .sort((a, b) => Math.abs(a.offset) - Math.abs(b.offset));
  return { following, top: element.scrollTop, anchors };
}

export function restoreScroll(element: HTMLElement, position: ScrollPosition): HTMLElement | undefined {
  const max = Math.max(0, element.scrollHeight - element.clientHeight);
  if (position.following) { element.scrollTop = max; return; }
  const nodes = new Map(Array.from(element.querySelectorAll<HTMLElement>("[data-transcript-anchor]"))
    .map((node) => [node.dataset.transcriptAnchor!, node]));
  for (const anchor of position.anchors) {
    const node = nodes.get(anchor.key);
    if (!node) continue;
    element.scrollTop = Math.max(0, Math.min(max,
      element.scrollTop + node.getBoundingClientRect().top - element.getBoundingClientRect().top - anchor.offset));
    return node;
  }
  element.scrollTop = Math.max(0, Math.min(max, position.top));
}
