import { createContext, useContext, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatRunDuration } from "../chat-utils";
import { useSmoothText } from "../hooks/useSmoothText";
import { Icon } from "./Icons";

/**
 * The rows a user has expanded, by `expansionKey`. A streamed message and the saved message
 * that replaces it are separate elements, so the choice has to outlive the first.
 */
export const ThinkingExpansion = createContext<Set<string> | undefined>(undefined);

interface ThinkingRowProps {
  text: string;
  /** How long the model reasoned, measured by the worker. Absent for blocks that were never clocked. */
  durationMs?: number;
  /** True while the model is still producing this block. */
  live?: boolean;
  /** Identifies the block in both the streamed and the saved message. */
  expansionKey?: string;
}

function formatDuration(ms: number): string {
  return ms < 1000 ? "<1s" : formatRunDuration(ms);
}

function ThinkingBody({ text, live }: { text: string; live: boolean }) {
  const shown = useSmoothText(text, live);
  return (
    <div className="thinking-body">
      <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>{shown}</ReactMarkdown>
    </div>
  );
}

export function ThinkingRow({ text, durationMs, live = false, expansionKey }: ThinkingRowProps) {
  const expanded = useContext(ThinkingExpansion);
  const [open, setOpen] = useState(() => expansionKey !== undefined && expanded?.has(expansionKey) === true);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (expansionKey === undefined) return;
    if (next) expanded?.add(expansionKey);
    else expanded?.delete(expansionKey);
  };

  return (
    <div className={`thinking-row ${open ? "open" : ""}`}>
      <button type="button" className="thinking-head" onClick={toggle} aria-expanded={open}>
        <Icon name="spark" className="thinking-icon" />
        {live
          ? <span className="thinking-shimmer">Thinking…</span>
          : <span>{durationMs !== undefined ? `Thought for ${formatDuration(durationMs)}` : "Reasoning"}</span>}
        <Icon name="chevron" className="tool-chevron" />
      </button>
      {open && text && <ThinkingBody text={text} live={live} />}
    </div>
  );
}
