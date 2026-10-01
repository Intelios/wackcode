import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  currentTurnIndex, hasOverflow, readLineY, scrollTopFor, scrollTopForRailPoint, tickOffset,
  type RailTurn,
} from "../scroll-rail";
import { Icon } from "./Icons";

const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface Metrics {
  scrollTop: number;
  clientHeight: number;
  scrollHeight: number;
  railHeight: number;
  /** Scroll-content offset (offsetTop) of each turn's message, in document order. NaN while unmeasured. */
  tops: number[];
}

interface Props {
  /** The .conversation-scroll element this rail mirrors. */
  target: RefObject<HTMLDivElement | null>;
  /** One per user turn, in document order. */
  turns: RailTurn[];
  /** The turn with a live run: its tick glows in the accent. */
  liveTurnId?: string;
  /** Whether the viewport is off the bottom — the end cap lights up as a call to action. */
  detached?: boolean;
  /** Releases bottom-pinning before a timeline jump or scrub starts. */
  onNavigate?: () => void;
  /** Scrolls to the latest message and re-pins follow mode. */
  onJumpToLatest?: () => void;
}

function scrollElementTo(el: HTMLElement, top: number, smooth: boolean) {
  if (smooth && typeof el.scrollTo === "function") el.scrollTo({ top, behavior: "smooth" });
  else el.scrollTop = top;
}

function sameMetrics(a: Metrics, b: Metrics): boolean {
  return a.scrollTop === b.scrollTop && a.clientHeight === b.clientHeight && a.scrollHeight === b.scrollHeight
    && a.railHeight === b.railHeight && a.tops.length === b.tops.length && a.tops.every((top, i) => top === b.tops[i]);
}

/**
 * WackCode's scrollbar replacement: a timeline down the transcript's right edge.
 * A hairline whose lit end follows the reading position, with a tick per turn —
 * hover a tick for the prompt it started, click one to jump back, or grab the
 * line anywhere to scrub. It reads the scroll container's geometry on
 * scroll/resize (rAF-coalesced) and writes scrollTop on interaction; useFollowScroll
 * stays the single owner of bottom-pinning. The rail is always mounted (even
 * empty) so its height can be measured before the transcript overflows.
 */
export function ScrollRail({ target, turns, liveTurnId, detached, onNavigate, onJumpToLatest }: Props) {
  const reduced = useReducedMotion();
  const railRef = useRef<HTMLDivElement>(null);
  const lineRef = useRef<HTMLDivElement | null>(null);
  const frame = useRef(0);
  const idleTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const scrollIdle = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [metrics, setMetrics] = useState<Metrics | null>(null);
  const metricsRef = useRef<Metrics | null>(null);
  const [awake, setAwake] = useState(false);
  const [hovered, setHovered] = useState<number | null>(null);
  const [hoveredEnd, setHoveredEnd] = useState(false);
  const [scrubbing, setScrubbing] = useState(false);
  // A press that never moves is a click: it deserves a smooth jump, not a scrub.
  const scrubOrigin = useRef<{ y: number; moved: boolean } | null>(null);
  // Read through a ref so `measure` stays stable while the message list changes every stream chunk.
  const turnsRef = useRef(turns);
  turnsRef.current = turns;
  // Whether the next coalesced measure must re-read turn offsets; pure scrolls can't
  // move them, so we only pay that layout cost when content (or the turn list) changed.
  const pendingTops = useRef(true);

  const measure = useCallback((measureTops = true) => {
    const el = target.current;
    if (!el) return;
    const prev = metricsRef.current;
    let tops = prev?.tops ?? [];
    if (measureTops || !prev || prev.scrollHeight !== el.scrollHeight || tops.length !== turnsRef.current.length) {
      const offsets = new Map<string, number>();
      for (const node of el.querySelectorAll<HTMLElement>("[data-turn]")) offsets.set(node.dataset.turn!, node.offsetTop);
      tops = turnsRef.current.map((turn) => offsets.get(turn.id) ?? Number.NaN);
    }
    const next: Metrics = {
      scrollTop: el.scrollTop,
      clientHeight: el.clientHeight,
      scrollHeight: el.scrollHeight,
      railHeight: railRef.current?.clientHeight ?? 0,
      tops,
    };
    metricsRef.current = next;
    setMetrics((old) => (old && sameMetrics(old, next) ? old : next));
  }, [target]);

  const schedule = useCallback((measureTops = true) => {
    pendingTops.current ||= measureTops;
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      measure(pendingTops.current);
      pendingTops.current = false;
    });
  }, [measure]);

  // "Awake" is the bright state; it comes from the user wheeling (and any rail
  // interaction), deliberately not from plain scroll events — auto-following a
  // stream would otherwise hold it lit for the whole run.
  const wake = useCallback(() => {
    setAwake(true);
    if (idleTimer.current) clearTimeout(idleTimer.current);
    idleTimer.current = setTimeout(() => setAwake(false), 1100);
  }, []);

  useEffect(() => {
    const el = target.current;
    if (!el) return;
    measure();
    // Instant feedback: while scroll events stream in, move the lit line in this same
    // frame with its height transition off — easing is only for content-driven changes
    // (streamed growth while detached), never for the user's gesture. The transition
    // is set imperatively because React's style diff won't touch a prop it never set.
    const onScroll = () => {
      const line = lineRef.current;
      const m = metricsRef.current;
      if (line && m) {
        if (!scrollIdle.current) line.style.transition = "none";
        line.style.height = `${readLineY({ ...m, scrollTop: el.scrollTop })}px`;
      }
      if (scrollIdle.current) clearTimeout(scrollIdle.current);
      scrollIdle.current = setTimeout(() => {
        scrollIdle.current = undefined;
        if (lineRef.current) lineRef.current.style.transition = "";
      }, 140);
      schedule(false);
    };
    const onWheel = () => { schedule(); wake(); };
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("wheel", onWheel, { passive: true });
    // The content grows while streaming and on resize; element turns appear with new messages.
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => schedule(true));
    observer?.observe(el);
    if (el.firstElementChild) observer?.observe(el.firstElementChild);
    return () => {
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("wheel", onWheel);
      observer?.disconnect();
      if (frame.current) { cancelAnimationFrame(frame.current); frame.current = 0; }
      if (idleTimer.current) clearTimeout(idleTimer.current);
      if (scrollIdle.current) clearTimeout(scrollIdle.current);
    };
  }, [target, measure, schedule, wake]);

  // Remeasure when turns appear or move (the elements may mount a frame later).
  useEffect(() => schedule(true), [turns, schedule]);

  const railY = (event: ReactPointerEvent) => {
    const rect = railRef.current?.getBoundingClientRect();
    return rect ? event.clientY - rect.top : 0;
  };

  const scrubTo = (event: ReactPointerEvent, smooth: boolean) => {
    const el = target.current;
    if (!el || !metrics) return;
    const top = scrollTopForRailPoint(railY(event), metrics.railHeight, metrics.scrollHeight, metrics.clientHeight);
    onNavigate?.();
    scrollElementTo(el, top, smooth);
  };

  const onRailDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!metrics) return;
    event.preventDefault();
    onNavigate?.();
    wake();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setScrubbing(true);
    scrubOrigin.current = { y: railY(event), moved: false };
  };

  const onRailMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const origin = scrubOrigin.current;
    if (!scrubbing || !origin) return;
    if (!origin.moved && Math.abs(railY(event) - origin.y) < 3) return;
    origin.moved = true;
    scrubTo(event, false);
  };

  const endScrub = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!scrubbing) return;
    const origin = scrubOrigin.current;
    scrubOrigin.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    setScrubbing(false);
    if (origin && !origin.moved) scrubTo(event, !reduced);
  };

  const goToTurn = (index: number) => {
    const el = target.current;
    if (!el || !metrics) return;
    const top = metrics.tops[index];
    if (Number.isNaN(top)) return;
    onNavigate?.();
    scrollElementTo(el, scrollTopFor(top, metrics.scrollHeight, metrics.clientHeight), !reduced);
  };

  const overflow = metrics !== null && hasOverflow(metrics.scrollHeight, metrics.clientHeight);
  const current = metrics ? currentTurnIndex(metrics.tops, metrics.scrollTop, metrics.clientHeight) : -1;
  const readY = metrics ? readLineY(metrics) : 0;

  return (
    <div
      ref={railRef}
      className={`scroll-rail${awake ? " awake" : ""}${scrubbing ? " scrubbing" : ""}`}
      role="navigation"
      aria-label="Conversation timeline"
      onPointerDown={overflow ? onRailDown : undefined}
      onPointerMove={overflow ? onRailMove : undefined}
      onPointerUp={endScrub}
      onPointerCancel={endScrub}
      onPointerEnter={wake}
    >
      {overflow && metrics && (
        <>
          <div className="scroll-rail-track" />
          <div ref={lineRef} className="scroll-rail-line" style={{ height: readY }} />
          {turns.map((turn, index) => {
            const top = metrics.tops[index];
            if (Number.isNaN(top)) return null;
            const classes = ["scroll-rail-tick"];
            if (index === current) classes.push("current");
            if (turn.id === liveTurnId) classes.push("live");
            return (
              <button
                key={turn.id}
                type="button"
                tabIndex={-1}
                className={classes.join(" ")}
                style={{ transform: `translateY(${tickOffset(top, metrics.scrollHeight, metrics.railHeight)}px)` }}
                aria-label={`Turn ${index + 1}${turn.excerpt ? `: ${turn.excerpt}` : ""}`}
                onPointerEnter={() => setHovered(index)}
                onPointerLeave={() => setHovered((value) => (value === index ? null : value))}
                onFocus={() => setHovered(index)}
                onBlur={() => setHovered((value) => (value === index ? null : value))}
                onPointerDown={(event) => event.stopPropagation()}
                onClick={() => goToTurn(index)}
              />
            );
          })}
          {onJumpToLatest && (
            <button
              type="button"
              className={`scroll-rail-end${detached ? " detached" : ""}`}
              aria-label="Jump to latest"
              onPointerDown={(event) => event.stopPropagation()}
              onClick={() => onJumpToLatest()}
              onPointerEnter={() => setHoveredEnd(true)}
              onPointerLeave={() => setHoveredEnd(false)}
              onFocus={() => setHoveredEnd(true)}
              onBlur={() => setHoveredEnd(false)}
            >
              <Icon name="chevron" style={{ transform: "rotate(90deg)" }} />
            </button>
          )}
          <AnimatePresence>
            {hovered !== null && turns[hovered] && !Number.isNaN(metrics.tops[hovered]) && (
              <motion.div
                key={hovered}
                className="scroll-rail-label"
                style={{ top: tickOffset(metrics.tops[hovered], metrics.scrollHeight, metrics.railHeight) }}
                initial={reduced ? { opacity: 0 } : { opacity: 0, x: 8 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: reduced ? 0 : 6 }}
                transition={{ duration: reduced ? 0.08 : 0.16, ease: EASE }}
              >
                <strong>Turn {hovered + 1}</strong>
                {turns[hovered].excerpt && <span>{turns[hovered].excerpt}</span>}
              </motion.div>
            )}
            {hovered === null && hoveredEnd && (
              <motion.div
                key="latest"
                className="scroll-rail-label"
                style={{ top: metrics.railHeight - 3 }}
                initial={reduced ? { opacity: 0 } : { opacity: 0, x: 8 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: reduced ? 0 : 6 }}
                transition={{ duration: reduced ? 0.08 : 0.16, ease: EASE }}
              >
                <strong>Latest</strong>
                <span>Jump to the end</span>
              </motion.div>
            )}
          </AnimatePresence>
        </>
      )}
    </div>
  );
}
