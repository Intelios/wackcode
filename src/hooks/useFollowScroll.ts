import { useLayoutEffect, useRef, useState } from "react";

const NEAR_BOTTOM = 90;

/**
 * Keeps a scroll container pinned to the bottom while content grows, unless the
 * user scrolls up — then a "Jump to latest" affordance is shown instead.
 */
export function useFollowScroll() {
  const ref = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const [detached, setDetached] = useState(false);

  const onScroll = () => {
    const el = ref.current;
    if (!el) return;
    const near = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM;
    following.current = near;
    setDetached(!near);
  };

  useLayoutEffect(() => {
    const el = ref.current;
    if (el && following.current && el.scrollHeight - el.scrollTop - el.clientHeight > 1) {
      el.scrollTop = el.scrollHeight;
    }
  });

  const jumpToLatest = () => {
    following.current = true;
    setDetached(false);
    const el = ref.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  };

  return { ref, onScroll, detached, jumpToLatest };
}
