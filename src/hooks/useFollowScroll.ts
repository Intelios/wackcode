import { useCallback, useLayoutEffect, useRef, useState, type WheelEvent } from "react";
import { captureScroll, restoreScroll, type TranscriptViewState } from "../transcript-view";

const BOTTOM_EPSILON = 1;

/**
 * Keeps a scroll container pinned to the bottom while content grows, unless the
 * user scrolls up — then a "Jump to latest" affordance is shown instead.
 * Upward intent releases the pin before the browser scrolls. Only returning to
 * the bottom reattaches — a proximity threshold would fight small gestures.
 */
export function useFollowScroll(memory?: TranscriptViewState, historyReady = true) {
  const ref = useRef<HTMLDivElement>(null);
  const ready = useRef(historyReady);
  ready.current = historyReady;
  const following = useRef(memory?.position?.following ?? true);
  const restoring = useRef(memory?.position);
  const lastTop = useRef(0);
  const heldAnchor = useRef<{ node: HTMLElement; offset: number } | null>(null);
  const observed = useRef<{ element: HTMLElement; content: Element | null; observer: ResizeObserver } | null>(null);
  const [detached, setDetached] = useState(!following.current);

  const savePosition = useCallback(() => {
    const el = ref.current;
    if (memory && el && !restoring.current) memory.position = captureScroll(el, following.current);
  }, [memory]);

  const pauseFollowing = useCallback(() => {
    heldAnchor.current = null;
    following.current = false;
    setDetached(true);
    savePosition();
  }, [savePosition]);

  const onWheel = (event: WheelEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el || event.deltaY === 0 || event.ctrlKey) return;
    if (!(event.target instanceof Element) || !el.contains(event.target)) return;
    // Tool output/previews can scroll independently. A consumed wheel must not release
    // the reading anchor while a disclosure is animating around that nested scroller.
    for (let node: Element | null = event.target; node && node !== el; node = node.parentElement) {
      if (!/^(auto|scroll)$/.test(getComputedStyle(node).overflowY)) continue;
      if (event.deltaY < 0 ? node.scrollTop > 0 : node.scrollTop + node.clientHeight < node.scrollHeight) return;
    }
    heldAnchor.current = null;
    if (event.deltaY < 0 && el.scrollHeight > el.clientHeight + BOTTOM_EPSILON) pauseFollowing();
  };

  const onScroll = () => {
    const el = ref.current;
    if (!el) return;
    if (el.scrollTop !== lastTop.current) heldAnchor.current = null;
    const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
    // At the bottom, a smaller scrollTop can also mean content was collapsed.
    if (el.scrollTop < lastTop.current && gap > BOTTOM_EPSILON) pauseFollowing();
    else if ((el.scrollTop > lastTop.current && gap <= BOTTOM_EPSILON) || el.scrollHeight <= el.clientHeight) {
      following.current = true;
      setDetached(false);
    }
    lastTop.current = el.scrollTop;
    savePosition();
  };

  const reconcile = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    // A cold history may arrive after the first paint. Keep the saved state intact until
    // there is a real transcript to anchor, rather than saving the empty placeholder.
    const position = restoring.current;
    if (position) {
      if (!ready.current) return;
      const node = restoreScroll(el, position);
      following.current = position.following;
      if (node) heldAnchor.current = { node, offset: node.getBoundingClientRect().top - el.getBoundingClientRect().top };
      restoring.current = undefined;
    }
    const anchor = heldAnchor.current;
    if (following.current) {
      if (Math.abs(el.scrollHeight - el.scrollTop - el.clientHeight) > BOTTOM_EPSILON) el.scrollTop = el.scrollHeight;
    } else if (anchor?.node.isConnected && el.contains(anchor.node)) {
      // Anchors are viewport-relative: use rects, never a nested wrapper's offsetTop.
      const delta = anchor.node.getBoundingClientRect().top - el.getBoundingClientRect().top - anchor.offset;
      if (Math.abs(delta) > BOTTOM_EPSILON) el.scrollTop += delta;
    } else if (memory?.position && ready.current) {
      // A branch change can retire the held row. Resolve the nearest surviving
      // saved location before replacing that memory with the new layout.
      const node = restoreScroll(el, memory.position);
      if (node) heldAnchor.current = { node, offset: node.getBoundingClientRect().top - el.getBoundingClientRect().top };
    }
    // Record our own writes so their delayed scroll events aren't user intent.
    lastTop.current = el.scrollTop;
    savePosition();
  }, [savePosition, memory]);

  const keepAnchor = useCallback((node: HTMLElement, offset: number) => {
    heldAnchor.current = { node, offset };
    reconcile();
  }, [reconcile]);
  const isFollowing = useCallback(() => following.current, []);

  useLayoutEffect(() => {
    reconcile();
    const el = ref.current;
    const content = el?.firstElementChild ?? null;
    if (observed.current?.element === el && observed.current?.content === content) return;
    observed.current?.observer.disconnect();
    observed.current = null;
    if (!el || typeof ResizeObserver === "undefined") return;
    // Motion changes height between React commits. Follow or hold the disclosure/reading
    // anchor through those frames, and reconnect when an empty transcript gains content.
    const observer = new ResizeObserver(reconcile);
    observer.observe(el);
    if (content) observer.observe(content);
    observed.current = { element: el, content, observer };
  });
  useLayoutEffect(() => () => { savePosition(); observed.current?.observer.disconnect(); }, [savePosition]);

  const jumpToLatest = () => {
    heldAnchor.current = null;
    following.current = true;
    setDetached(false);
    const el = ref.current;
    if (el) {
      lastTop.current = el.scrollTop;
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    }
  };

  return { ref, onScroll, onWheel, detached, pauseFollowing, jumpToLatest, keepAnchor, isFollowing, reconcile };
}
