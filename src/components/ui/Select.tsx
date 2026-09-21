import { useRef, useState, type ReactNode } from "react";
import { Menu, type MenuEntry } from "./Menu";
import { Popover } from "./Popover";

export interface SelectOption {
  value: string;
  label: ReactNode;
  hint?: string;
  disabled?: boolean;
}

interface SelectProps {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  placeholder?: string;
  "aria-label"?: string;
  className?: string;
  side?: "top" | "bottom";
  matchWidth?: boolean;
}

/** Custom dropdown styled for the app — replaces native <select>. */
export function Select({ value, options, onChange, disabled, placeholder, className, side = "bottom", matchWidth, "aria-label": ariaLabel }: SelectProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const selected = options.find((option) => option.value === value);
  const entries: MenuEntry[] = options.map((option) => ({
    label: option.label,
    hint: option.hint,
    disabled: option.disabled,
    selected: option.value === value,
    onSelect: () => onChange(option.value)
  }));
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`select-trigger ${className ?? ""}`}
        disabled={disabled}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="select-value">{selected?.label ?? placeholder ?? "Select…"}</span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={triggerRef} open={open} onClose={close} side={side} matchWidth={matchWidth}>
        <Menu items={entries} onClose={close} />
      </Popover>
    </>
  );
}
