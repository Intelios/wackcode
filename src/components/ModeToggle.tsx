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
 */
export function ModeToggle({ mode, disabled, onChange }: ModeToggleProps) {
  const planning = mode !== "build";
  const ultra = mode === "ultraplan";
  const tip = disabled
    ? "Wait for the current run"
    : mode === "plan"
      ? "Click Plan again for Ultra Plan (⇧Tab)"
      : ultra
        ? "Click to go back to Plan (⇧Tab)"
        : "Switch mode (⇧Tab)";
  return (
    <Tooltip label={tip}>
      <div className="mode-toggle" role="radiogroup" aria-label="Mode">
        <button
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
