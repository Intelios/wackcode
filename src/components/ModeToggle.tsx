import { useLayoutEffect, useRef, useState } from "react";
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
 * The Build/Plan composer toggle. Plan mode is read-only: the agent explores and asks
 * questions, then submits a plan for review instead of editing. Clicking Plan again switches
 * to Ultra Plan, where the agent interviews the user in depth before planning, and back.
 * ⇧Tab cycles all three.
 *
 * A pill behind the options slides to the active one. Its geometry is measured from the
 * button itself rather than hardcoded, because the Plan button resizes when it becomes
 * Ultra Plan (wider label, flame instead of brain).
 */
export function ModeToggle({ mode, disabled, onChange }: ModeToggleProps) {
  const planning = mode !== "build";
  const ultra = mode === "ultraplan";
  const buildRef = useRef<HTMLButtonElement>(null);
  const planRef = useRef<HTMLButtonElement>(null);
  const [pill, setPill] = useState<{ left: number; width: number } | null>(null);
  const [settled, setSettled] = useState(false);
  const tip = disabled
    ? "Wait for the current run"
    : mode === "plan"
      ? "Click Plan again for Ultra Plan (⇧Tab)"
      : ultra
        ? "Click to go back to Plan (⇧Tab)"
        : "Switch mode (⇧Tab)";

  const active = planning ? planRef.current : buildRef.current;
  useLayoutEffect(() => {
    if (!active) return;
    const measure = () => setPill({ left: active.offsetLeft, width: active.offsetWidth });
    measure();
    // The first measurement would otherwise animate in from the left edge on mount.
    const raf = requestAnimationFrame(() => setSettled(true));
    // Ultra Plan's label swap reflows the button a frame later, so track its size too.
    const observer = new ResizeObserver(measure);
    observer.observe(active);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
    };
  }, [active]);

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
          className={`mode-option ${mode === "build" ? "active" : ""}`}
          disabled={disabled}
          onClick={() => onChange("build")}
        >
          Build
        </button>
        <button
          ref={planRef}
          type="button"
          role="radio"
          aria-checked={planning}
          className={`mode-option plan ${planning ? "active" : ""} ${ultra ? "ultra" : ""}`}
          disabled={disabled}
          onClick={() => onChange(planButtonTarget(mode))}
        >
          <Icon name={ultra ? "flame" : "brain"} className="mode-icon" />
          <span className="mode-label">{ultra ? "Ultra Plan" : "Plan"}</span>
        </button>
      </div>
    </Tooltip>
  );
}
