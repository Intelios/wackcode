import { useRef, useState, type ReactNode } from "react";
import { Icon } from "../Icons";
import { Menu, type MenuEntry } from "./Menu";
import { Popover } from "./Popover";

interface MenuButtonProps {
  items: MenuEntry[] | (() => MenuEntry[]);
  label: string;
  icon?: ReactNode;
  className?: string;
  align?: "start" | "end";
  side?: "top" | "bottom";
}

/** Icon button that opens a dropdown menu. */
export function MenuButton({ items, label, icon, className, align = "end", side = "bottom" }: MenuButtonProps) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className={`icon-button ${className ?? ""}`}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {icon ?? <Icon name="more" />}
      </button>
      <Popover anchor={buttonRef} open={open} onClose={close} align={align} side={side}>
        <Menu items={typeof items === "function" ? items() : items} onClose={close} />
      </Popover>
    </>
  );
}
