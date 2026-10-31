import { useRef, useState } from "react";
import type { SessionSnapshot } from "../types";
import { formatTokens } from "../chat-utils";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

type Stats = SessionSnapshot["stats"];
type Detail = NonNullable<NonNullable<Stats["contextBreakdown"]>["detail"]>;
type Group = Detail["groups"][number];

const CATEGORY_META: Record<string, { label: string; color: string }> = {
  system: { label: "System prompt & tools", color: "#8a93a5" },
  // The accent token, so the chart follows Settings › Appearance.
  user: { label: "User messages", color: "var(--wc-accent)" },
  assistant: { label: "Assistant replies", color: "#7fb8e0" },
  tool: { label: "Tool results", color: "#e6ba69" }
};

/**
 * Categorical colours for the itemised rows. Fixed hues, like the coarse split above: the accent
 * stays on the conversation itself, and the prompt material recedes into greys while capability
 * groups (tools, skills, memory) keep a readable colour each.
 */
const GROUP_META: Record<string, { color: string }> = {
  messages: { color: "var(--wc-accent)" },
  summary: { color: "#7d86a0" },
  contract: { color: "#8f7fb8" },
  "app-context": { color: "#9aa3b5" },
  "tools:builtin": { color: "#7fb8e0" },
  "tools:app": { color: "#a58ce0" },
  "tools:package": { color: "#e6ba69" },
  system: { color: "#8a93a5" },
  project: { color: "#d78f6b" },
  skills: { color: "#6fc7a4" },
  memory: { color: "#c0a2d8" }
};

const groupColor = (id: string) => (id.startsWith("mcp:") ? "#4fc2a8" : GROUP_META[id]?.color ?? "#8a93a5");

/** A share of the window to one decimal; a real but tiny share reads `<0.1%`, never a flat 0.0%. */
function shareOf(tokens: number, window_: number): string {
  if (window_ <= 0) return "—";
  const percent = (tokens / window_) * 100;
  return percent > 0 && percent < 0.05 ? "<0.1%" : `${percent.toFixed(1)}%`;
}

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

function GroupRow({ group, window_, open, onToggle }: {
  group: Group;
  window_: number;
  open: boolean;
  onToggle: () => void;
}) {
  const expandable = (group.children?.length ?? 0) > 0;
  const share = shareOf(group.tokens, window_);
  const main = (
    <>
      <span className="context-dot" style={{ background: groupColor(group.id) }} />
      <span className="context-label">{group.label}</span>
      <span className="context-value">{formatTokens(group.tokens)}</span>
      <span className="context-pct">{share}</span>
      {expandable ? <Icon name="chevron" className="context-chevron" /> : <span className="context-chevron" />}
    </>
  );
  return (
    <div className={`context-row${open ? " open" : ""}`}>
      {expandable ? (
        <button type="button" className="context-row-main" aria-expanded={open} onClick={onToggle}>
          {main}
        </button>
      ) : (
        <div className="context-row-main">{main}</div>
      )}
      {expandable && open && (
        <div className="context-children">
          {group.children!.map((child, index) => (
            <div className="context-child" key={`${child.label}:${index}`}>
              <span className="context-label">{child.label}</span>
              <span className="context-value">{formatTokens(child.tokens)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function ContextPanel({ stats, popoverSide = "top", glass = false, onCompact, compact }: {
  stats: Stats;
  popoverSide?: "top" | "bottom";
  glass?: boolean;
  /** Runs a manual compaction; omitted where there is nothing to act on. */
  onCompact?: () => void;
  /** Why Compact cannot run right now: the panel shows this instead of its estimating hint. */
  compact?: { disabledReason?: string };
}) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<string[]>([]);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const show = () => {
    clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const scheduleHide = () => {
    clearTimeout(closeTimer.current);
    closeTimer.current = setTimeout(() => setOpen(false), 120);
  };
  // Moving focus into the panel (Tab, or a row pressed from the keyboard) must not read as
  // leaving the trigger: the rows and Compact are reachable without a pointer.
  const releaseFocus = (event: React.FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget as Node | null;
    if (next && panelRef.current?.contains(next)) return;
    scheduleHide();
  };
  const toggle = (id: string) => setExpanded((current) =>
    current.includes(id) ? current.filter((entry) => entry !== id) : [...current, id]);

  const context = stats.contextUsage;
  const breakdown = stats.contextBreakdown;
  const detail = breakdown?.detail;
  const used = context?.tokens ?? null;
  const window_ = context?.contextWindow ?? 0;
  const percent = context?.percent ?? null;
  const entries = breakdown?.entries.filter((entry) => entry.tokens > 0) ?? [];
  const left = used != null && window_ > 0 ? Math.max(window_ - used, 0) : null;
  // Pi leaves its compaction reserve clear of the conversation; the gap to it is what the user
  // can still spend. Without a live measurement (a saved view) there is no threshold to show.
  const untilCompact = used != null && window_ > 0 && detail
    ? Math.max(window_ - detail.reserveTokens - used, 0)
    : null;
  const freeShare = left != null ? shareOf(left, window_) : "—";

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`composer-stats ${open ? "active" : ""}`}
        onMouseEnter={show}
        onMouseLeave={scheduleHide}
        onFocus={show}
        onBlur={releaseFocus}
        onClick={() => (open ? setOpen(false) : show())}
        aria-label={percent == null ? "Context window usage unknown" : `Context window ${Math.round(percent)}% full`}
        aria-expanded={open}
        aria-haspopup="dialog"
      >
        <ContextRing percent={percent} />
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side={popoverSide} align="center" offset={12} glass={glass} className="context-pop">
        <div ref={panelRef} className="context-panel" role="dialog" aria-label="Context window" onMouseEnter={show} onMouseLeave={scheduleHide} onFocus={show} onBlur={releaseFocus}>
          <div className="context-hero">
            <ContextRing percent={percent} size={42}>
              <span className="context-ring-value">{percent == null ? "—" : `${Math.round(percent)}%`}</span>
            </ContextRing>
            <div className="context-hero-text">
              <h3>Context window</h3>
              <span>{used == null ? "Usage arrives after the next reply" : `${formatTokens(used)} of ${formatTokens(window_)} tokens`}</span>
              {left != null && (
                <span className="context-left">
                  {untilCompact == null ? `${formatTokens(left)} left` : untilCompact > 0 ? `${formatTokens(untilCompact)} until auto-compact` : "Auto-compaction due"}
                </span>
              )}
            </div>
          </div>
          {window_ > 0 && (
            <div className="context-bar" aria-hidden="true">
              {detail ? (
                detail.groups.map((group) => (
                  <span key={group.id} title={group.label} style={{ width: `${Math.min((group.tokens / window_) * 100, 100)}%`, background: groupColor(group.id) }} />
                ))
              ) : entries.length > 0 ? (
                entries.map((entry) => (
                  <span key={entry.id} style={{ width: `${Math.min((entry.tokens / window_) * 100, 100)}%`, background: CATEGORY_META[entry.id]?.color }} />
                ))
              ) : percent != null ? (
                <span style={{ width: `${Math.min(percent, 100)}%`, background: "var(--wc-accent-fill)" }} />
              ) : null}
            </div>
          )}
          {detail ? (
            <div className="context-legend">
              {detail.groups.map((group) => (
                <GroupRow
                  key={group.id}
                  group={group}
                  window_={window_}
                  open={expanded.includes(group.id)}
                  onToggle={() => toggle(group.id)}
                />
              ))}
              {left != null && <div className="context-row context-free">
                <span className="context-dot" />
                <span className="context-label">Free space</span>
                <span className="context-value">{formatTokens(left)}</span>
                <span className="context-pct">{freeShare}</span>
                <span className="context-chevron" />
              </div>}
            </div>
          ) : entries.length > 0 && used != null && used > 0 ? (
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
          ) : null}
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
          {onCompact && (
            <button
              type="button"
              className="secondary-button context-compact"
              onClick={onCompact}
              disabled={Boolean(compact?.disabledReason)}
              title={compact?.disabledReason}
            >
              Compact
            </button>
          )}
          <div className="context-hint">{compact?.disabledReason ?? "Estimates scaled to the provider's count for this turn."}</div>
        </div>
      </Popover>
    </>
  );
}
