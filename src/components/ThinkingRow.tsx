import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Icon } from "./Icons";

interface ThinkingRowProps {
  text: string;
  /** True while the model is still producing this block. */
  streaming?: boolean;
}

function formatDuration(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function ThinkingRow({ text, streaming }: ThinkingRowProps) {
  const [open, setOpen] = useState(false);
  const startedAt = useRef(Date.now());
  const [duration, setDuration] = useState<string>();

  useEffect(() => {
    if (!streaming) setDuration((current) => current ?? formatDuration(Date.now() - startedAt.current));
  }, [streaming]);

  if (streaming) {
    return (
      <div className="thinking-row live">
        <Icon name="spark" className="thinking-icon" />
        <span className="thinking-shimmer">Thinking…</span>
      </div>
    );
  }

  return (
    <div className={`thinking-row ${open ? "open" : ""}`}>
      <button type="button" className="thinking-head" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <Icon name="spark" className="thinking-icon" />
        <span>{duration ? `Thought for ${duration}` : "Reasoning"}</span>
        <Icon name="chevron" className="tool-chevron" />
      </button>
      {open && (
        <div className="thinking-body">
          <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>{text}</ReactMarkdown>
        </div>
      )}
    </div>
  );
}
