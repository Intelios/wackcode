import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { motion } from "motion/react";
import type { NormalizedBlock } from "../types";
import {
  exploreActivity, exploreCounts, exploreLabelParts, exploreLiveItems, exploreMarks, flyId, settledGroup,
  type ExploreGroup as Group, type ExploreItem, type ExploreMark
} from "../explore-utils";
import { Icon, type IconName } from "./Icons";
import { RollingNumber } from "./RollingNumber";
import { ThinkingRow } from "./ThinkingRow";
import { FLY, ToolChips, ToolImages, useTranscriptMotion, type ChipCall } from "./ToolRow";

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

const MARK_ICONS: Record<ExploreMark, IconName> = {
  file: "file", image: "image", search: "search", list: "folder", command: "terminal", thought: "brain"
};

/**
 * One mark in the head's icon stack. It carries the shared-element id of the newest item of its
 * kind, keyed by that id, so when an item joins, a new mark mounts under the id the item's chip
 * (or reasoning row) just gave up and motion flies it from there into the stack — the chip
 * shrinks into the head instead of vanishing. `layoutDependency` is fixed so the head's own
 * re-renders (rolling counts) never move it; it differs from the chip's so the hand-off resumes.
 */
function Mark({ mark, id, animate }: { mark: ExploreMark; id: string; animate: boolean }) {
  return (
    <motion.span
      className={`explore-mark ${mark}`}
      layoutId={animate ? id : undefined}
      layoutDependency="mark"
      style={{ borderRadius: 999 }}
      transition={{ layout: FLY }}
    >
      {/* Its own layout node, so motion corrects the icon for the pill's scale mid-flight
          instead of stretching it from the chip's width. */}
      <motion.span className="explore-mark-glyph" layout={animate} transition={{ layout: FLY }}><Icon name={MARK_ICONS[mark]} /></motion.span>
    </motion.span>
  );
}

/** "3 files, 1 search", with each number rolling up as calls join. */
function Summary({ parts, animate }: { parts: ReturnType<typeof exploreLabelParts>; animate: boolean }) {
  return (
    <>
      {parts.map((part, index) => (
        <span key={part.key} className="explore-count">
          {index > 0 && ", "}
          {animate ? <RollingNumber value={part.count} /> : part.count} {part.noun}
        </span>
      ))}
    </>
  );
}

/** Settled items in order: neighbouring calls share a chip strip, reasoning sits between. */
function renderBody(items: ExploreItem[], results: Map<string, NormalizedBlock>, liveToolText: Record<string, string> | undefined, running: boolean): ReactNode[] {
  const nodes: ReactNode[] = [];
  let strip: ChipCall[] = [];
  const flush = () => {
    if (!strip.length) return;
    nodes.push(<ToolChips key={`chips:${strip[0].call.toolCallId ?? nodes.length}`} entries={strip} />);
    strip = [];
  };
  for (const item of items) {
    const { block } = item;
    if (block.type === "thinking") {
      flush();
      nodes.push(<ThinkingRow key={item.key} text={block.text ?? ""} durationMs={block.durationMs} startedAt={block.startedAt} live={running && item.streaming && block.durationMs === undefined} expansionKey={item.key} />);
      continue;
    }
    const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
    strip.push({ call: block, result, liveText: block.toolCallId ? liveToolText?.[block.toolCallId] : undefined, running: item.live && !result });
  }
  flush();
  return nodes;
}

/**
 * A run of read-only tool calls folded into one chip-shaped head (`explore-utils.ts`). The head
 * holds what has finished — an icon stack of the kinds seen and rolling counts — while calls
 * still in flight show as live chips beneath it and fly into the head as they land. Starts
 * collapsed; images its calls returned stay in view beneath it, and move under their own chips
 * once open.
 */
export function ExploreGroup({ group, results, liveToolText, running }: ExploreGroupProps) {
  const expanded = useContext(ExploreExpansion);
  const animate = useTranscriptMotion();
  const [open, setOpen] = useState(() => expanded?.has(group.key) === true);
  const activity = exploreActivity(group, results);
  const live = exploreLiveItems(group, results);
  const settled = settledGroup(group, results);
  const counts = exploreCounts(group, results);
  const marks = exploreMarks(settled, results);
  const parts = exploreLabelParts(exploreCounts(settled, results));

  // A brief glow on the head each time something joins it (never on mount, nor for history).
  const joined = settled.items.length;
  const lastJoined = useRef(joined);
  const [pulse, setPulse] = useState(0);
  useEffect(() => {
    if (joined > lastJoined.current && animate) setPulse((value) => value + 1);
    lastJoined.current = joined;
  }, [joined, animate]);

  // What the folded calls returned as pictures: shown under the fold, where their chips aren't.
  const pictured = open ? [] : settled.items.flatMap(({ block }) => {
    const result = block.type === "tool-call" && block.toolCallId ? results.get(block.toolCallId) : undefined;
    return result?.images?.length ? [{ call: block, result }] : [];
  });

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) expanded?.add(group.key);
    else expanded?.delete(group.key);
  };

  return (
    <div className={`explore-group ${open ? "open" : ""} ${activity ? "live" : ""}`}>
      <button type="button" className="explore-head" onClick={toggle} aria-expanded={open}>
        {pulse > 0 && <span key={pulse} className="explore-pulse" aria-hidden="true" />}
        <span className="explore-stack" aria-hidden="true">
          {marks.map(({ mark, id }) => <Mark key={id} mark={mark} id={id} animate={animate} />)}
        </span>
        {activity
          ? <span className="explore-verb thinking-shimmer">Exploring</span>
          : <span className="explore-verb">Explored</span>}
        {/* Spaces between the parts, so the button's name reads "Explored 2 files, 1 search". */}
        {" "}<span className="explore-summary"><span aria-hidden="true">·</span> {activity ?? <Summary parts={parts} animate={animate} />}</span>
        {!activity && counts.failed > 0 && <>{" "}<span className="tool-row-failed">{counts.failed} failed</span></>}
        {" "}<span className="tool-row-status"><Icon name="chevron" className="tool-chevron" /></span>
      </button>
      {live.length > 0 && (
        <div className="explore-live">
          <ToolChips entries={live.map((item) => {
            const result = item.block.toolCallId ? results.get(item.block.toolCallId) : undefined;
            return {
              call: item.block,
              result,
              liveText: item.block.toolCallId ? liveToolText?.[item.block.toolCallId] : undefined,
              running: true,
              flyId: flyId(item)
            };
          })} />
        </div>
      )}
      {pictured.length > 0 && (
        <div className="explore-images">
          {pictured.map(({ call, result }) => <ToolImages key={call.toolCallId} call={call} result={result} caption />)}
        </div>
      )}
      {open && <div className="explore-body">{renderBody(settled.items, results, liveToolText, running)}</div>}
    </div>
  );
}
