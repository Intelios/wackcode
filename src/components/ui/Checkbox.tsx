import { useEffect, useRef } from "react";

interface CheckboxProps {
  checked: boolean;
  /** The "some but not all" state of a select-all box; announced as mixed. */
  indeterminate?: boolean;
  disabled?: boolean;
  /** Accessible name; the box has no visible label of its own. */
  label: string;
  onChange: (checked: boolean) => void;
  tabIndex?: number;
}

/**
 * A native checkbox (so it keeps its role, keyboard and mixed state) drawn as an accent box
 * whose tick strokes itself in. Clicks don't bubble, so it can sit inside a clickable row.
 */
export function Checkbox({ checked, indeterminate = false, disabled, label, onChange, tabIndex }: CheckboxProps) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  const state = indeterminate ? "mixed" : checked ? "checked" : "unchecked";
  return (
    <label className={`wc-checkbox ${state}${disabled ? " disabled" : ""}`} onClick={(event) => event.stopPropagation()}>
      <input
        ref={ref}
        type="checkbox"
        aria-label={label}
        checked={checked}
        disabled={disabled}
        tabIndex={tabIndex}
        onChange={(event) => onChange(event.target.checked)}
      />
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <rect className="wc-checkbox-box" x="1.25" y="1.25" width="13.5" height="13.5" rx="3.75" />
        <path className="wc-checkbox-tick" d="m4.6 8.4 2.3 2.3 4.5-5" pathLength={1} />
        <path className="wc-checkbox-bar" d="M4.75 8h6.5" />
      </svg>
    </label>
  );
}
