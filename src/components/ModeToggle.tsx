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
 * questions, then submits a plan for review instead of editing. ⇧Tab cycles modes.
 */
export function ModeToggle({ mode, disabled, onChange }: ModeToggleProps) {
  return (
    <Tooltip label={disabled ? "Wait for the current run" : "Switch mode (⇧Tab)"}>
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
          aria-checked={mode === "plan"}
          className={`mode-option plan ${mode === "plan" ? "active" : ""}`}
          disabled={disabled}
          onClick={() => onChange("plan")}
        >
          <Icon name="brain" className="mode-icon" /> Plan
        </button>
      </div>
    </Tooltip>
  );
}
