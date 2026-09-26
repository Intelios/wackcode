import { createContext, useContext, useMemo, useState } from "react";
import { formatRunDuration, thinkingPreview } from "../chat-utils";
import { useSmoothText } from "../hooks/useSmoothText";
import { Markdown } from "./Markdown";
import { Icon } from "./Icons";

/**
 * The rows a user has expanded, by `expansionKey`. A streamed message and the saved message
 * that replaces it are separate elements, so the choice has to outlive the first.
 */
export const ThinkingExpansion = createContext<Set<string> | undefined>(undefined);

/** Settings → Appearance → Thinking preview. On unless a provider says otherwise. */
export const ThinkingPreviewEnabled = createContext(true);

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
      <Markdown streaming={live}>{shown}</Markdown>
    </div>
  );
}

export function ThinkingRow({ text, durationMs, live = false, expansionKey }: ThinkingRowProps) {
  const expanded = useContext(ThinkingExpansion);
  const [open, setOpen] = useState(() => expansionKey !== undefined && expanded?.has(expansionKey) === true);
  const previewEnabled = useContext(ThinkingPreviewEnabled);
  const showPreview = live && previewEnabled && !open;
  const preview = useMemo(() => (showPreview ? thinkingPreview(text) : undefined), [showPreview, text]);

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
        {/* Keyed so each new line fades in rather than swapping in place. */}
        {preview && <span className="thinking-preview" key={preview}><span aria-hidden="true">·</span> {preview}</span>}
        <Icon name="chevron" className="tool-chevron" />
      </button>
      {open && text && <ThinkingBody text={text} live={live} />}
    </div>
  );
}
