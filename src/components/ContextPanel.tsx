import { useRef, useState } from "react";
import type { SessionSnapshot } from "../types";
import { formatTokens } from "../chat-utils";
import { Popover } from "./ui/Popover";

type Stats = SessionSnapshot["stats"];

const CATEGORY_META: Record<string, { label: string; color: string }> = {
  system: { label: "System prompt & tools", color: "#8a93a5" },
  user: { label: "User messages", color: "#c2ee4a" },
  assistant: { label: "Assistant replies", color: "#7fb8e0" },
  tool: { label: "Tool results", color: "#e6ba69" }
};

export function ContextPanel({ stats, label }: { stats: Stats; label: string }) {
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

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`composer-stats ${open ? "active" : ""}`}
        onMouseEnter={show}
        onMouseLeave={scheduleHide}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        {label}
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="top" align="end" className="context-pop">
        <div className="context-panel" onMouseEnter={show} onMouseLeave={scheduleHide}>
          <div className="context-head">
            <h3>Context window</h3>
            <span>{used == null ? "—" : `${formatTokens(used)} / ${formatTokens(window_)}`}{percent != null && ` (${Math.round(percent)}%)`}</span>
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
          {breakdown?.cacheHitRate != null && (
            <>
              <div className="context-divider" />
              <div className="context-row context-foot">
                <span className="context-label">Cache hit rate</span>
                <span className="context-pct">{(breakdown.cacheHitRate * 100).toFixed(1)}%</span>
              </div>
            </>
          )}
          <div className="context-hint">Estimates; exact counts come from the provider each turn.</div>
        </div>
      </Popover>
    </>
  );
}
