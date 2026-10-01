import { createContext, useContext, useState } from "react";
import type { NormalizedBlock } from "../types";
import { exploreActivity, exploreCounts, exploreLabel, type ExploreGroup as Group } from "../explore-utils";
import { Icon } from "./Icons";
import { ThinkingRow } from "./ThinkingRow";
import { ToolRow } from "./ToolRow";

/** Settings → Appearance → Group exploration. When off, every call keeps its own row. */
export const ExploreGroupingEnabled = createContext(true);

/**
 * The groups a user has expanded, by group key. A group opened while its run streams can move
 * to the saved message's element, so the choice has to outlive the first.
 */
export const ExploreExpansion = createContext<Set<string> | undefined>(undefined);

interface ExploreGroupProps {
  group: Group;
  results: Map<string, NormalizedBlock>;
  liveToolText?: Record<string, string>;
  running: boolean;
}

/** A run of read-only tool calls folded into one row (`explore-utils.ts`). Starts collapsed. */
export function ExploreGroup({ group, results, liveToolText, running }: ExploreGroupProps) {
  const expanded = useContext(ExploreExpansion);
  const [open, setOpen] = useState(() => expanded?.has(group.key) === true);
  const activity = exploreActivity(group, results);
  const counts = exploreCounts(group, results);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) expanded?.add(group.key);
    else expanded?.delete(group.key);
  };

  return (
    <div className={`explore-group ${open ? "open" : ""}`}>
      <button type="button" className="explore-head" onClick={toggle} aria-expanded={open}>
        <Icon name="search" className="tool-row-icon" />
        {activity
          ? <span className="explore-verb thinking-shimmer">Exploring</span>
          : <span className="explore-verb">Explored</span>}
        {/* Spaces between the parts, so the button's name reads "Explored 2 files, 1 search". */}
        {" "}<span className="explore-summary"><span aria-hidden="true">·</span> {activity ?? exploreLabel(counts)}</span>
        {!activity && counts.failed > 0 && <>{" "}<span className="tool-row-failed">{counts.failed} failed</span></>}
        {" "}<span className="tool-row-status">
          {activity && <span className="tool-spinner" aria-label="Running" />}
          <Icon name="chevron" className="tool-chevron" />
        </span>
      </button>
      {open && (
        <div className="explore-body">
          {group.items.map((item) => {
            const { block } = item;
            if (block.type === "thinking") {
              return <ThinkingRow key={item.key} text={block.text ?? ""} durationMs={block.durationMs} startedAt={block.startedAt} live={running && item.streaming && block.durationMs === undefined} expansionKey={item.key} />;
            }
            const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
            const liveText = block.toolCallId ? liveToolText?.[block.toolCallId] : undefined;
            return <ToolRow key={block.toolCallId ?? item.key} call={block} result={result} liveText={liveText} running={item.live && !result} />;
          })}
        </div>
      )}
    </div>
  );
}
