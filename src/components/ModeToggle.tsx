import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { planButtonTarget } from "../chat-utils";
import type { TaskMode } from "../types";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

interface ModeToggleProps {
  mode: TaskMode;
  disabled?: boolean;
  onChange: (mode: TaskMode) => void;
}

/**
 * The Build/Plan composer toggle. Plan mode is read-only by default: the agent explores and
 * asks questions, then submits a plan for review. Clicking Plan again switches
 * to Ultra Plan, where the agent interviews the user in depth before planning, and back.
 * ⇧Tab cycles all three.
 *
 * A pill behind the options slides to the active one. Its geometry is measured from the
 * button itself rather than hardcoded, because the Plan button resizes when it becomes
 * Ultra Plan (wider label, flame instead of brain).
 *
 * Switching into Build or Plan plays a one-shot nudge on the icon that just became active
 * (the hammer taps, the brain swells) plus a soft wash behind the option — subtle, but the
 * mode change lands. Ultra Plan is deliberately exempt: swapping in its fire is its own,
 * much louder, moment. The nudge is keyed by a nonce so each switch remounts the icon and
 * replays the animation; see the activation block in styles.css.
 */
export function ModeToggle({ mode, disabled, onChange }: ModeToggleProps) {
  const planning = mode !== "build";
  const ultra = mode === "ultraplan";
  const buildRef = useRef<HTMLButtonElement>(null);
  const planRef = useRef<HTMLButtonElement>(null);
  const [pill, setPill] = useState<{ left: number; width: number } | null>(null);
  const [settled, setSettled] = useState(false);
  const [pulse, setPulse] = useState<{ side: "build" | "plan"; nonce: number } | null>(null);
  const lastMode = useRef(mode);
  const tip = disabled
    ? "Wait for the current run"
    : mode === "plan"
      ? "Click Plan again for Ultra Plan (⇧Tab)"
      : ultra
        ? "Click to go back to Plan (⇧Tab)"
        : "Switch mode (⇧Tab)";

  // A run that owns the mode gets no motion (like the held-still flame): only an actual
  // switch the user made replays the nudge, never the mount of a fresh composer.
  useEffect(() => {
    if (lastMode.current === mode) return;
    lastMode.current = mode;
    if (disabled || mode === "ultraplan") setPulse(null);
    else setPulse((prev) => ({ side: mode, nonce: (prev?.nonce ?? 0) + 1 }));
  }, [mode, disabled]);

  const active = planning ? planRef.current : buildRef.current;
  useLayoutEffect(() => {
    if (!active) return;
    const measure = () => setPill({ left: active.offsetLeft, width: active.offsetWidth });
    measure();
    // The first measurement would otherwise animate in from the left edge on mount.
    const raf = requestAnimationFrame(() => setSettled(true));
    // Ultra Plan's label swap reflows the button a frame later, so track its size too.
    // Absent in jsdom, where the one measurement above is all there is to do.
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(active);
    return () => {
      cancelAnimationFrame(raf);
      observer?.disconnect();
    };
  }, [active]);

  // The nudge belongs to the option that is actually active, so a stale pulse left over
  // from the other side (or from before an Ultra Plan swap) never touches the wrong button.
  const pulsing = pulse && pulse.side === mode ? pulse : null;
  const pulseKey = (side: "build" | "plan") => (pulsing?.side === side ? `pulse-${pulsing.nonce}` : "idle");
  const pulseClass = (side: "build" | "plan") => (pulsing?.side === side ? "mode-in" : "");

  return (
    <Tooltip label={tip}>
      <div className="mode-toggle sliding" role="radiogroup" aria-label="Mode">
        {pill && (
          <span
            className={`mode-pill ${planning ? "plan" : ""} ${ultra ? "ultra" : ""} ${settled ? "settled" : ""} ${disabled ? "disabled" : ""}`}
            style={{ left: pill.left, width: pill.width }}
            aria-hidden="true"
          />
        )}
        <button
          ref={buildRef}
          type="button"
          role="radio"
          aria-checked={mode === "build"}
          className={`mode-option build ${mode === "build" ? "active" : ""} ${pulseClass("build")}`}
          disabled={disabled}
          onClick={() => onChange("build")}
        >
          {pulseClass("build") && <span key={pulseKey("build")} className="mode-glow build" aria-hidden="true" />}
          <Icon key={pulseKey("build")} name="hammer" className="mode-icon" />
          <span className="mode-label">Build</span>
        </button>
        <button
          ref={planRef}
          type="button"
          role="radio"
          aria-checked={planning}
          className={`mode-option plan ${planning ? "active" : ""} ${ultra ? "ultra" : ""} ${pulseClass("plan")}`}
          disabled={disabled}
          onClick={() => onChange(planButtonTarget(mode))}
        >
          {ultra ? (
            <span className="mode-flame">
              <Icon name="flame" className="mode-icon" />
              {/* Embers drift off the flame; the wrapper carries the bloom and they ride above it.
                  Three, on staggered delays, so the fire keeps a pulse instead of pulsing twice. */}
              <span className="flame-ember" aria-hidden="true" />
              <span className="flame-ember" aria-hidden="true" />
              <span className="flame-ember" aria-hidden="true" />
            </span>
          ) : (
            <>
              {pulseClass("plan") && <span key={pulseKey("plan")} className="mode-glow plan" aria-hidden="true" />}
              <Icon key={pulseKey("plan")} name="brain" className="mode-icon" />
            </>
          )}
          <span className="mode-label">{ultra ? "Ultra Plan" : "Plan"}</span>
        </button>
      </div>
    </Tooltip>
  );
}
