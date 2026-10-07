import { AnimatePresence, motion, type Transition } from "motion/react";
import { createContext, useContext, useEffect, useMemo, useRef, useState } from "react";
import { formatThinkingDuration, thinkingStream } from "../chat-utils";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { motionAllowed, useSmoothText } from "../hooks/useSmoothText";
import type { ThinkingTimerPrecision as Precision } from "../types";
import { Markdown } from "./Markdown";
import { PonderingDuck } from "./PonderingDuck";
import { Icon } from "./Icons";

/**
 * The rows a user has expanded, by `expansionKey`. A streamed message and the saved message
 * that replaces it are separate elements, so the choice has to outlive the first.
 */
export const ThinkingExpansion = createContext<Set<string> | undefined>(undefined);

/** Settings → Appearance → Thinking preview. On unless a provider says otherwise. */
export const ThinkingPreviewEnabled = createContext(true);

/** Settings → Appearance → Thinking timer: whole seconds unless the user picks tenths. */
export const ThinkingTimerPrecision = createContext<Precision>("second");

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

/** The body's unfurl timings: height and drift run together; the fade leads in… */
const REVEAL_OPEN: Transition = {
  height: { duration: 0.26, ease: EASE },
  y: { duration: 0.26, ease: EASE },
  opacity: { duration: 0.18, ease: EASE }
};
/** …and leads out, so collapsing text reads as leaving rather than being squashed. */
const REVEAL_EXIT: Transition = {
  height: { duration: 0.22, ease: EASE },
  y: { duration: 0.22, ease: EASE },
  opacity: { duration: 0.14, ease: EASE }
};

interface ThinkingRowProps {
  text: string;
  /** How long the model reasoned, measured by the worker. Absent for blocks that were never clocked. */
  durationMs?: number;
  /** This block's start timestamp from the worker, so a remount never resets its live timer. */
  startedAt?: number;
  /** True while the model is still producing this block. */
  live?: boolean;
  /** Identifies the block in both the streamed and the saved message. */
  expansionKey?: string;
}

/** Only the elapsed label ticks; the reasoning body and preview need not re-render each tick. */
function ThinkingElapsed({ startedAt }: { startedAt: number }) {
  const precision = useContext(ThinkingTimerPrecision);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    setNow(Date.now());
    // Tenths count in 100 ms steps; whole seconds settle for one render a second.
    const timer = window.setInterval(() => setNow(Date.now()), precision === "tenth" ? 100 : 1000);
    return () => window.clearInterval(timer);
  }, [startedAt, precision]);
  return <span className={precision === "tenth" ? "thinking-elapsed precise" : "thinking-elapsed"}>{formatThinkingDuration(Math.max(0, now - startedAt), precision)}</span>;
}

/** The settling beat after a live block finishes (styles.css walks the rail bead off its end). */
const SETTLE_MS = 900;

function ThinkingBody({ text, live, settling, animate }: { text: string; live: boolean; settling: boolean; animate: boolean }) {
  const shown = useSmoothText(text, live);
  // Opens at the newest reasoning and, while live, stays pinned there so the
  // stream can be watched as it arrives. Scrolling up reads earlier text
  // without being yanked; only returning to the bottom resumes following.
  const { ref, onScroll, onWheel } = useFollowScroll();
  // The frame owns the rail and its bead, outside the scrolling body, so the bead rides a
  // fixed track while the text scrolls beneath it. Fresh ink exists only where motion is
  // positively allowed: reduced motion and tests (no matchMedia) render plain text nodes.
  return (
    <div className={`thinking-frame ${live ? "live" : ""} ${settling ? "settling" : ""}`}>
      <div ref={ref} onScroll={onScroll} onWheel={onWheel} className="thinking-body" role="region" aria-label="Reasoning" tabIndex={0}>
        <Markdown streaming={live} freshInk={live && animate}>{shown}</Markdown>
      </div>
    </div>
  );
}

/** The flattened reasoning tail plus the accent caret that marks the write head. */
function StreamTail({ tail }: { tail: string }) {
  return (
    <>
      <span className="thinking-stream-text">{tail}</span>
      <span className="thinking-stream-caret" />
    </>
  );
}

export function ThinkingRow({ text, durationMs, startedAt, live = false, expansionKey }: ThinkingRowProps) {
  const expanded = useContext(ThinkingExpansion);
  const [open, setOpen] = useState(() => expansionKey !== undefined && expanded?.has(expansionKey) === true);
  const previewEnabled = useContext(ThinkingPreviewEnabled);
  const precision = useContext(ThinkingTimerPrecision);
  // Same rule useSmoothText uses: only animate where the motion preference can be queried and
  // isn't reduced — elsewhere (tests, no matchMedia) the stream appears and leaves instantly.
  const animate = motionAllowed();
  const showStream = live && previewEnabled && !open;
  // The same per-frame reveal the open body uses, so the tail types in rather than popping.
  // Inactive while open or finished, where it returns the full text at once.
  const smooth = useSmoothText(text, showStream);
  const tail = useMemo(() => (showStream ? thinkingStream(smooth) : undefined), [showStream, smooth]);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (expansionKey === undefined) return;
    if (next) expanded?.add(expansionKey);
    else expanded?.delete(expansionKey);
  };

  // The settling beat: when a live block finishes in place, hold `settling` briefly so the
  // rail eases back and its bead departs instead of everything snapping to rest. Only a body
  // that is open with text when the block finishes can show the beat; a row that mounts
  // already finished (history, or the saved message replacing the streamed one) never settles
  // — `wasLive` starts false for it. The timer runs its course unwatched rather than being
  // cancelled by a collapse, so `settling` always clears.
  const wasLive = useRef(live);
  const [settling, setSettling] = useState(false);
  useEffect(() => {
    const finished = wasLive.current && !live;
    wasLive.current = live;
    if (!finished || !open || !text) return;
    setSettling(true);
    const timer = window.setTimeout(() => setSettling(false), SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [live]);

  return (
    <div className={`thinking-row ${open ? "open" : ""} ${showStream ? "streaming" : ""}`}>
      <button type="button" className="thinking-head" onClick={toggle} aria-expanded={open}>
        <PonderingDuck live={live} className="thinking-icon" />
        {/* `key` remounts the label when the block finishes, so it eases in (styles.css)
            rather than snapping from "Thinking… 12s" to "Thought for 12s". */}
        {live
          ? <span key="live" className="thinking-shimmer">Thinking…</span>
          : <span key="done" className="thinking-label">{durationMs !== undefined ? `Thought for ${formatThinkingDuration(durationMs, precision)}` : "Reasoning"}</span>}
        {live && startedAt !== undefined && <>{" "}<ThinkingElapsed startedAt={startedAt} /></>}
        {/* Hidden from AT: a per-frame-changing name is noise; the full reasoning is one click away. */}
        {!animate
          ? tail && <span className="thinking-stream" aria-hidden="true"><StreamTail tail={tail} /></span>
          : (
            <AnimatePresence initial={false}>
              {tail && (
                <motion.span
                  className="thinking-stream"
                  aria-hidden="true"
                  initial={{ opacity: 0, x: 6 }}
                  animate={{ opacity: 1, x: 0 }}
                  exit={{ opacity: 0, transition: { duration: 0.16 } }}
                >
                  <StreamTail tail={tail} />
                </motion.span>
              )}
            </AnimatePresence>
          )}
        <Icon name="chevron" className="tool-chevron" />
      </button>
      {/* The unfurl: the wrapper clips from zero height while the body drifts down into place
          (`.thinking-reveal` in styles.css). Where motion isn't allowed, the body simply appears. */}
      {!animate
        ? open && text && <ThinkingBody text={text} live={live} settling={settling} animate={animate} />
        : (
          <AnimatePresence initial={false}>
            {open && text && (
              <motion.div
                className="thinking-reveal"
                initial={{ height: 0, opacity: 0, y: -6 }}
                animate={{ height: "auto", opacity: 1, y: 0 }}
                exit={{ height: 0, opacity: 0, y: -6, transition: REVEAL_EXIT }}
                transition={REVEAL_OPEN}
              >
                <ThinkingBody text={text} live={live} settling={settling} animate={animate} />
              </motion.div>
            )}
          </AnimatePresence>
        )}
    </div>
  );
}
