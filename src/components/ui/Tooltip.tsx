import { useEffect, useRef, useState, type ReactElement, type ReactNode } from "react";
import { createPortal } from "react-dom";

interface TooltipProps {
  label: ReactNode;
  side?: "top" | "bottom";
  disabled?: boolean;
  children: ReactElement;
}

export function Tooltip({ label, side = "top", disabled = false, children }: TooltipProps) {
  const wrapRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [style, setStyle] = useState<React.CSSProperties>();

  const show = () => {
    if (disabled || !label) return;
    timer.current = setTimeout(() => {
      const rect = wrapRef.current?.getBoundingClientRect();
      if (!rect) return;
      setStyle({
        position: "fixed",
        top: side === "top" ? rect.top - 6 : rect.bottom + 6,
        left: rect.left + rect.width / 2,
        transform: side === "top" ? "translate(-50%, -100%)" : "translate(-50%, 0)",
        zIndex: 200
      });
    }, 400);
  };
  const hide = () => {
    if (timer.current) clearTimeout(timer.current);
    setStyle(undefined);
  };

  useEffect(() => {
    if (disabled) {
      if (timer.current) clearTimeout(timer.current);
      setStyle(undefined);
    }
  }, [disabled]);

  return (
    <span ref={wrapRef} className="tooltip-wrap" onMouseEnter={show} onMouseLeave={hide} onMouseDown={hide}>
      {children}
      {style && !disabled && createPortal(<span className="tooltip" style={style} role="tooltip">{label}</span>, document.body)}
    </span>
  );
}
