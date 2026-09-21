import { useEffect, useRef, useState, type ReactNode } from "react";

export interface MenuItem {
  label: ReactNode;
  icon?: ReactNode;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  selected?: boolean;
  onSelect?: () => void;
}

export type MenuEntry = MenuItem | "separator";

export function Menu({ items, onClose }: { items: MenuEntry[]; onClose: () => void }) {
  const [active, setActive] = useState(() => items.findIndex((item) => item !== "separator" && !item.disabled));
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listRef.current?.focus();
  }, []);

  const enabledIndexes = items.map((item, index) => item !== "separator" && !item.disabled ? index : -1).filter((index) => index >= 0);

  function move(direction: 1 | -1) {
    if (enabledIndexes.length === 0) return;
    const position = enabledIndexes.indexOf(active);
    const next = position === -1
      ? (direction === 1 ? enabledIndexes[0] : enabledIndexes[enabledIndexes.length - 1])
      : enabledIndexes[(position + direction + enabledIndexes.length) % enabledIndexes.length];
    setActive(next);
  }

  function choose(index: number) {
    const item = items[index];
    if (item === "separator" || item.disabled) return;
    onClose();
    item.onSelect?.();
  }

  return (
    <div
      ref={listRef}
      className="menu"
      role="menu"
      tabIndex={-1}
      onKeyDown={(event) => {
        if (event.key === "ArrowDown") { event.preventDefault(); move(1); }
        else if (event.key === "ArrowUp") { event.preventDefault(); move(-1); }
        else if (event.key === "Home") { event.preventDefault(); setActive(enabledIndexes[0] ?? -1); }
        else if (event.key === "End") { event.preventDefault(); setActive(enabledIndexes[enabledIndexes.length - 1] ?? -1); }
        else if (event.key === "Enter" || event.key === " ") { event.preventDefault(); if (active >= 0) choose(active); }
      }}
    >
      {items.map((item, index) => item === "separator" ? (
        <div className="menu-separator" key={index} role="separator" />
      ) : (
        <button
          key={index}
          type="button"
          role="menuitem"
          className={`menu-item ${index === active ? "active" : ""} ${item.danger ? "danger" : ""}`}
          disabled={item.disabled}
          onMouseEnter={() => setActive(index)}
          onClick={() => choose(index)}
        >
          {item.icon && <span className="menu-icon">{item.icon}</span>}
          <span className="menu-label">{item.label}</span>
          {item.selected && <span className="menu-check">✓</span>}
          {item.hint && <span className="menu-hint">{item.hint}</span>}
        </button>
      ))}
    </div>
  );
}
