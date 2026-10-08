import { useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { useNavigationDismiss } from "./NavigationScope";

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
  useNavigationDismiss(() => { if (open) onClose(); });
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
      // Full content height in layout px: ignores an earlier maxHeight clamp and the pop-in scale.
      const height = panel.scrollHeight + panel.offsetHeight - panel.clientHeight;
      const margin = 8;
      const spaceBelow = window.innerHeight - (rect.bottom + offset) - margin;
      const spaceAbove = rect.top - offset - margin;

      // Open on the preferred side when the panel fits there, else on whichever side has more room.
      const fitsBelow = height <= spaceBelow;
      const fitsAbove = height <= spaceAbove;
      const below = side === "bottom"
        ? fitsBelow || (!fitsAbove && spaceBelow >= spaceAbove)
        : !fitsAbove && (fitsBelow || spaceBelow > spaceAbove);
      const maxHeight = Math.max(100, below ? spaceBelow : spaceAbove);
      const top = below ? rect.bottom + offset : Math.max(margin, rect.top - Math.min(height, maxHeight) - offset);

      let left = align === "end" ? rect.right - panelRect.width : align === "center" ? rect.left + rect.width / 2 - panelRect.width / 2 : rect.left;
      left = Math.max(margin, Math.min(left, window.innerWidth - panelRect.width - margin));
      setStyle({ position: "fixed", top, left, maxHeight, minWidth: matchWidth ? rect.width : undefined, zIndex: 150 });
    };
    reposition();
    // Sliding pickers can grow when a provider list changes to the taller favourites rows.
    // Keep the preferred side anchored as content changes, including after an async save.
    const resizeObserver = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(reposition);
    if (panelRef.current) resizeObserver?.observe(panelRef.current);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      resizeObserver?.disconnect();
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
        anchor.current?.focus();
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
