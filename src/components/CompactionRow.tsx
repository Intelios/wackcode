import { useId } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { NormalizedMessage } from "../types";
import { motionAllowed } from "../hooks/useSmoothText";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";

/** A durable boundary in display history, even when Pi has summarized the preceding work. */
export function CompactionRow({ message, open, onToggle }: {
  message: NormalizedMessage;
  open: boolean;
  onToggle: () => void;
}) {
  const regionId = useId();
  const reduce = useReducedMotion();
  const animate = reduce !== true && motionAllowed();
  const result = message.compaction!;
  const count = (tokens: number) => Math.round(tokens).toLocaleString();
  const body = <div className="compaction-body" role="region" aria-label="Compaction summary" id={regionId} tabIndex={0}>
    <dl className="compaction-counts">
      <div><dt>Context before</dt><dd>{count(result.tokensBefore)} tokens</dd></div>
      {result.estimatedTokensAfter !== undefined && <div><dt>Estimated context after</dt><dd>≈{count(result.estimatedTokensAfter)} tokens</dd></div>}
    </dl>
    <p className="compaction-hint">Summary carried into the agent’s context. Earlier messages remain in this chat.</p>
    <div className="assistant-text"><Markdown>{result.summary}</Markdown></div>
  </div>;
  return <div className={`compaction-row${open ? " open" : ""}`} data-transcript-anchor={`compaction:${message.id}`}>
    <button type="button" className="compaction-head" aria-expanded={open} aria-controls={regionId}
      aria-label={`Context compacted. ${open ? "Hide" : "Show"} summary`} onClick={onToggle}>
      <Icon name="collapse" />
      <span>Context compacted</span>
      <small>{count(result.tokensBefore)}{result.estimatedTokensAfter !== undefined ? ` → ≈${count(result.estimatedTokensAfter)}` : " before"} tokens</small>
      <Icon name="chevron" className="compaction-chevron" />
    </button>
    {!animate ? open && body : <AnimatePresence initial={false}>
      {open && <motion.div className="compaction-reveal" initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }}
        exit={{ height: 0, opacity: 0 }} transition={{ duration: 0.2, ease: [0.33, 1, 0.68, 1] }}>{body}</motion.div>}
    </AnimatePresence>}
  </div>;
}
