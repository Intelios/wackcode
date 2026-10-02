import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { formatRunDuration, thinkingPreview } from "../chat-utils";
import { useFollowScroll } from "../hooks/useFollowScroll";
import { useSmoothText } from "../hooks/useSmoothText";
import { Markdown } from "./Markdown";
import { Icon } from "./Icons";

/**
 * The rows a user has expanded, by `expansionKey`. A streamed message and the saved message
 * that replaces it are separate elements, so the choice has to outlive the first.
 */
export const ThinkingExpansion = createContext<Set<string> | undefined>(undefined);

/** Settings → Appearance → Thinking preview. On unless a provider says otherwise. */
export const ThinkingPreviewEnabled = createContext(true);

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

function formatDuration(ms: number): string {
  return ms < 1000 ? "<1s" : formatRunDuration(ms);
}

/** Only the elapsed label ticks; the reasoning body and preview need not re-render each second. */
function ThinkingElapsed({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [startedAt]);
  return <span className="thinking-elapsed">{formatDuration(Math.max(0, now - startedAt))}</span>;
}

function ThinkingBody({ text, live }: { text: string; live: boolean }) {
  const shown = useSmoothText(text, live);
  // Opens at the newest reasoning and, while live, stays pinned there so the
  // stream can be watched as it arrives. Scrolling up reads earlier text
  // without being yanked; only returning to the bottom resumes following.
  const { ref, onScroll, onWheel } = useFollowScroll();
  return (
    <div ref={ref} onScroll={onScroll} onWheel={onWheel} className="thinking-body" role="region" aria-label="Reasoning" tabIndex={0}>
      <Markdown streaming={live}>{shown}</Markdown>
    </div>
  );
}

export function ThinkingRow({ text, durationMs, startedAt, live = false, expansionKey }: ThinkingRowProps) {
  const expanded = useContext(ThinkingExpansion);
  const [open, setOpen] = useState(() => expansionKey !== undefined && expanded?.has(expansionKey) === true);
  const previewEnabled = useContext(ThinkingPreviewEnabled);
  const showPreview = live && previewEnabled && !open;
  const preview = useMemo(() => (showPreview ? thinkingPreview(text) : undefined), [showPreview, text]);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (expansionKey === undefined) return;
    if (next) expanded?.add(expansionKey);
    else expanded?.delete(expansionKey);
  };

  return (
    <div className={`thinking-row ${open ? "open" : ""}`}>
      <button type="button" className="thinking-head" onClick={toggle} aria-expanded={open}>
        <Icon name="spark" className="thinking-icon" />
        {live
          ? <span className="thinking-shimmer">Thinking…</span>
          : <span>{durationMs !== undefined ? `Thought for ${formatDuration(durationMs)}` : "Reasoning"}</span>}
        {live && startedAt !== undefined && <>{" "}<ThinkingElapsed startedAt={startedAt} /></>}
        {/* Keyed so each new line fades in rather than swapping in place. */}
        {preview && <span className="thinking-preview" key={preview}><span aria-hidden="true">·</span> {preview}</span>}
        <Icon name="chevron" className="tool-chevron" />
      </button>
      {open && text && <ThinkingBody text={text} live={live} />}
    </div>
  );
}
