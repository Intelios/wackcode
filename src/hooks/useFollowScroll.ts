import { useCallback, useLayoutEffect, useRef, useState, type WheelEvent } from "react";

const BOTTOM_EPSILON = 1;

/**
 * Keeps a scroll container pinned to the bottom while content grows, unless the
 * user scrolls up — then a "Jump to latest" affordance is shown instead.
 * Upward intent releases the pin before the browser scrolls. Only returning to
 * the bottom reattaches — a proximity threshold would fight small gestures.
 */
export function useFollowScroll() {
  const ref = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const lastTop = useRef(0);
  const [detached, setDetached] = useState(false);

  const pauseFollowing = useCallback(() => {
    following.current = false;
    setDetached(true);
  }, []);

  const onWheel = (event: WheelEvent<HTMLDivElement>) => {
    const el = ref.current;
    if (!el || event.deltaY >= 0 || event.ctrlKey || el.scrollHeight <= el.clientHeight + BOTTOM_EPSILON) return;
    if (!(event.target instanceof Element) || !el.contains(event.target)) return;
    // Tool output/previews can scroll independently. Release the transcript only
    // when the upward gesture can reach it (the nested scroller is at its top).
    for (let node: Element | null = event.target; node && node !== el; node = node.parentElement) {
      if (node.scrollTop > 0 && /^(auto|scroll)$/.test(getComputedStyle(node).overflowY)) return;
    }
    pauseFollowing();
  };

  const onScroll = () => {
    const el = ref.current;
    if (!el) return;
    const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
    // At the bottom, a smaller scrollTop can also mean content was collapsed.
    if (el.scrollTop < lastTop.current && gap > BOTTOM_EPSILON) pauseFollowing();
    else if ((el.scrollTop > lastTop.current && gap <= BOTTOM_EPSILON) || el.scrollHeight <= el.clientHeight) {
      following.current = true;
      setDetached(false);
    }
    lastTop.current = el.scrollTop;
  };

  useLayoutEffect(() => {
    const el = ref.current;
    if (el && following.current) {
      if (el.scrollHeight - el.scrollTop - el.clientHeight > BOTTOM_EPSILON) el.scrollTop = el.scrollHeight;
      // Record our own writes so their delayed scroll events aren't user intent.
      lastTop.current = el.scrollTop;
    }
  });

  const jumpToLatest = () => {
    following.current = true;
    setDetached(false);
    const el = ref.current;
    if (el) {
      lastTop.current = el.scrollTop;
      el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    }
  };

  return { ref, onScroll, onWheel, detached, pauseFollowing, jumpToLatest };
}
