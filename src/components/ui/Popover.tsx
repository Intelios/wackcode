import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";

interface PopoverProps {
  anchor: RefObject<HTMLElement | null>;
  open: boolean;
  onClose: () => void;
  side?: "top" | "bottom";
  align?: "start" | "center" | "end";
  offset?: number;
  matchWidth?: boolean;
  className?: string;
  children: ReactNode;
}

export function Popover({ anchor, open, onClose, side = "bottom", align = "start", offset = 6, matchWidth, className, children }: PopoverProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<React.CSSProperties>({ position: "fixed", top: 0, left: 0, visibility: "hidden" });

  useLayoutEffect(() => {
    if (!open) return;
    const reposition = () => {
      const anchorEl = anchor.current;
      const panel = panelRef.current;
      if (!anchorEl || !panel) return;
      const rect = anchorEl.getBoundingClientRect();
      const panelRect = panel.getBoundingClientRect();
      const margin = 8;
      const spaceBelow = window.innerHeight - (rect.bottom + offset) - margin;
      const spaceAbove = rect.top - offset - margin;

      let top: number;
      let maxHeight: number | undefined;

      if (side === "bottom") {
        if (spaceBelow < 120 && spaceAbove > spaceBelow) {
          top = Math.max(margin, rect.top - panelRect.height - offset);
          maxHeight = Math.max(100, spaceAbove);
        } else {
          top = rect.bottom + offset;
          maxHeight = Math.max(100, spaceBelow);
        }
      } else {
        if (spaceAbove < 120 && spaceBelow > spaceAbove) {
          top = rect.bottom + offset;
          maxHeight = Math.max(100, spaceBelow);
        } else {
          top = Math.max(margin, rect.top - panelRect.height - offset);
          maxHeight = Math.max(100, spaceAbove);
        }
      }

      let left = align === "end" ? rect.right - panelRect.width : align === "center" ? rect.left + rect.width / 2 - panelRect.width / 2 : rect.left;
      left = Math.max(margin, Math.min(left, window.innerWidth - panelRect.width - margin));
      setStyle({ position: "fixed", top, left, maxHeight, minWidth: matchWidth ? rect.width : undefined, zIndex: 150 });
    };
    reposition();
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [open, anchor, side, align, offset, matchWidth]);

  useLayoutEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (panelRef.current?.contains(target) || anchor.current?.contains(target)) return;
      onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose, anchor]);

  if (!open) return null;
  return createPortal(
    <div ref={panelRef} className={`popover ${className ?? ""}`} style={style} role="presentation">
      {children}
    </div>,
    document.body
  );
}
