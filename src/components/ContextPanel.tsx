import { useRef, useState } from "react";
import type { SessionSnapshot } from "../types";
import { formatTokens } from "../chat-utils";
import { Popover } from "./ui/Popover";

type Stats = SessionSnapshot["stats"];

const CATEGORY_META: Record<string, { label: string; color: string }> = {
  system: { label: "System prompt & tools", color: "#8a93a5" },
  // The accent token, so the chart follows Settings › Appearance.
  user: { label: "User messages", color: "var(--wc-accent)" },
  assistant: { label: "Assistant replies", color: "#7fb8e0" },
  tool: { label: "Tool results", color: "#e6ba69" }
};

/** From here the ring swaps the accent for the fixed warning colour: a nearly full window is
 *  meaning, not interaction, so it doesn't follow the user's accent. */
const NEARLY_FULL = 90;

/**
 * A ring that fills clockwise with the share of the context window in use. `pathLength` makes
 * the arc's dash lengths percentages, so the fill is a single dashoffset that CSS transitions.
 * An unknown percent draws the bare track.
 */
export function ContextRing({ percent, size = 16, children }: { percent: number | null; size?: number; children?: React.ReactNode }) {
  const clamped = percent == null ? 0 : Math.max(0, Math.min(percent, 100));
  return (
    <span className={`context-ring${percent != null && percent >= NEARLY_FULL ? " full" : ""}`} style={{ width: size, height: size }} aria-hidden="true">
      <svg viewBox="0 0 20 20" width={size} height={size}>
        <circle className="context-ring-track" cx="10" cy="10" r="8" />
        {percent != null && <circle className="context-ring-fill" cx="10" cy="10" r="8" pathLength={100} strokeDasharray="100" strokeDashoffset={100 - clamped} />}
      </svg>
      {children}
    </span>
  );
}

export function ContextPanel({ stats }: { stats: Stats }) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const show = () => {
    clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const scheduleHide = () => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };
  const context = stats.contextUsage;
  const breakdown = stats.contextBreakdown;
  const used = context?.tokens ?? null;
  const window_ = context?.contextWindow ?? 0;
  const percent = context?.percent ?? null;
  const entries = breakdown?.entries.filter((entry) => entry.tokens > 0) ?? [];
  const left = used != null && window_ > 0 ? Math.max(window_ - used, 0) : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`composer-stats ${open ? "active" : ""}`}
        onMouseEnter={show}
        onMouseLeave={scheduleHide}
        onFocus={show}
        onBlur={scheduleHide}
        onClick={() => (open ? setOpen(false) : show())}
        aria-label={percent == null ? "Context window usage unknown" : `Context window ${Math.round(percent)}% full`}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        <ContextRing percent={percent} />
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="top" align="end" className="context-pop">
        <div className="context-panel" role="dialog" aria-label="Context window" onMouseEnter={show} onMouseLeave={scheduleHide}>
          <div className="context-hero">
            <ContextRing percent={percent} size={42}>
              <span className="context-ring-value">{percent == null ? "—" : `${Math.round(percent)}%`}</span>
            </ContextRing>
            <div className="context-hero-text">
              <h3>Context window</h3>
              <span>{used == null ? "Usage arrives after the next reply" : `${formatTokens(used)} of ${formatTokens(window_)} tokens`}</span>
              {left != null && <span className="context-left">{formatTokens(left)} left</span>}
            </div>
          </div>
          {window_ > 0 && (
            <div className="context-bar">
              {entries.length > 0 ? (
                entries.map((entry) => (
                  <span key={entry.id} style={{ width: `${Math.min((entry.tokens / window_) * 100, 100)}%`, background: CATEGORY_META[entry.id]?.color }} />
                ))
              ) : percent != null ? (
                <span style={{ width: `${Math.min(percent, 100)}%`, background: "var(--wc-accent)" }} />
              ) : null}
            </div>
          )}
          {entries.length > 0 && used != null && used > 0 && (
            <div className="context-legend">
              {entries.map((entry) => (
                <div className="context-row" key={entry.id}>
                  <span className="context-dot" style={{ background: CATEGORY_META[entry.id]?.color }} />
                  <span className="context-label">{CATEGORY_META[entry.id]?.label ?? entry.id}{entry.id === "system" ? " (est.)" : ""}</span>
                  <span className="context-value">{formatTokens(entry.tokens)}</span>
                  <span className="context-pct">{((entry.tokens / used) * 100).toFixed(1)}%</span>
                </div>
              ))}
            </div>
          )}
          <div className="context-divider" />
          <dl className="context-stats">
            <div>
              <dt>Session tokens</dt>
              <dd>{formatTokens(stats.tokens.total)}</dd>
            </div>
            <div>
              <dt>Cache hits</dt>
              <dd>{breakdown?.cacheHitRate == null ? "—" : `${(breakdown.cacheHitRate * 100).toFixed(1)}%`}</dd>
            </div>
          </dl>
          <div className="context-hint">Estimates; exact counts come from the provider each turn.</div>
        </div>
      </Popover>
    </>
  );
}
