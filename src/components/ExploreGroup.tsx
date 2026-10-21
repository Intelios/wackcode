import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { animate as play, motion } from "motion/react";
import type { NormalizedBlock } from "../types";
import {
  exploreActivity, exploreCounts, exploreLabelParts, exploreLiveItems, exploreMarks, flyId, settledGroup,
  type ExploreGroup as Group, type ExploreItem, type ExploreMark
} from "../explore-utils";
import { Icon, type IconName } from "./Icons";
import { RollingNumber } from "./RollingNumber";
import { ThinkingRow } from "./ThinkingRow";
import { contentBox, takeDuckBox } from "./thoughtFlight";
import { FLY, ToolChips, ToolImages, useTranscriptMotion, type ChipCall } from "./ToolRow";

/** Settings → Appearance → Group exploration. When off, every call keeps its own row. */
export const ExploreGroupingEnabled = createContext(true);

/**
 * The group a running transcript ends with while it waits on the model (`tailGroupKey`), and
 * what the wait is called. That group's lane holds the wait in place of the transcript's own
 * "Working…" row, so the space under the head stays put between one call and the next.
 */
export const ExploreLaneWait = createContext<{ key: string; label: string } | undefined>(undefined);

/** The lane closing once the run moves on: the space under the head eases shut. */
const LANE_CLOSE = { duration: 0.24, ease: [0.33, 1, 0.68, 1] } as const;
/** It opens at once: what arrives pops in itself, and a growing gap would dip the view. */
const LANE_OPEN = { duration: 0 } as const;

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
 * just gave up and motion flies it from there into the stack — the chip shrinks into the head
 * instead of vanishing. `layoutDependency` is fixed so the head's own re-renders (rolling
 * counts) never move it; it differs from the chip's so the hand-off resumes. Reasoning hands
 * off by hand instead (`thoughtFlight.ts`): the thought mark flies from the row's duck.
 */
function Mark({ mark, id, animate }: { mark: ExploreMark; id: string; animate: boolean }) {
  const ref = useRef<HTMLSpanElement>(null);
  const thought = mark === "thought";
  useLayoutEffect(() => {
    const from = animate && thought ? takeDuckBox(id) : undefined;
    if (!from || !ref.current) return;
    const to = contentBox(ref.current);
    const controls = play(ref.current, { x: [from.x - to.x, 0], y: [from.y - to.y, 0] }, FLY);
    return () => controls.stop();
  }, []);
  return (
    <motion.span
      ref={ref}
      className={`explore-mark ${mark}`}
      layoutId={animate && !thought ? id : undefined}
      layoutDependency="mark"
      style={{ borderRadius: 999 }}
      transition={{ layout: FLY }}
    >
      {/* Its own layout node, so motion corrects the icon for the pill's scale mid-flight
          instead of stretching it from the chip's width. */}
      <motion.span className="explore-mark-glyph" layout={animate && !thought} transition={{ layout: FLY }}><Icon name={MARK_ICONS[mark]} /></motion.span>
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
 * holds what has finished — an icon stack of the kinds seen and rolling counts. Beneath it, the
 * lane holds whatever the run is doing now: calls in flight as live chips, the reasoning after
 * the last call, or the wait for the model. Each flies into the head as the next arrives, in
 * the same lane, so the transcript below doesn't hop as the model alternates; the head says
 * "Exploring" until the run is over. The lane eases shut when the run moves on. Starts
 * collapsed; images its calls returned stay in view beneath it, and move under their own chips
 * once open.
 */
export function ExploreGroup({ group, results, liveToolText, running }: ExploreGroupProps) {
  const expanded = useContext(ExploreExpansion);
  const animate = useTranscriptMotion();
  const [open, setOpen] = useState(() => expanded?.has(group.key) === true);
  const activity = exploreActivity(group, results);
  const live = exploreLiveItems(group, results);
  const trailing = group.trailing ?? [];
  const lane = useContext(ExploreLaneWait);
  const wait = lane?.key === group.key ? lane.label : undefined;
  const exploring = Boolean(activity) || trailing.length > 0 || wait !== undefined;
  const busy = live.length > 0 || trailing.length > 0 || wait !== undefined;
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

  const laneContent = (
    <>
      {live.length > 0 && (
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
      )}
      {trailing.map((item) => (
        <ThinkingRow key={item.key} text={item.block.text ?? ""} durationMs={item.block.durationMs} startedAt={item.block.startedAt}
          live={running && item.streaming && item.block.durationMs === undefined} expansionKey={item.key} flyId={flyId(item)} />
      ))}
      {wait !== undefined && !live.length && !trailing.length && (
        <div className="explore-wait" role="status" aria-label={wait || "Working…"}>
          <span className="work-dots" aria-hidden="true"><i /><i /><i /></span>
        </div>
      )}
    </>
  );

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) expanded?.add(group.key);
    else expanded?.delete(group.key);
  };

  return (
    <div className={`explore-group ${open ? "open" : ""} ${exploring ? "live" : ""}`}>
      <button type="button" className="explore-head" onClick={toggle} aria-expanded={open}>
        {pulse > 0 && <span key={pulse} className="explore-pulse" aria-hidden="true" />}
        <span className="explore-stack" aria-hidden="true">
          {marks.map(({ mark, id }) => <Mark key={id} mark={mark} id={id} animate={animate} />)}
        </span>
        {exploring
          ? <span className="explore-verb thinking-shimmer">Exploring</span>
          : <span className="explore-verb">Explored</span>}
        {/* Spaces between the parts, so the button's name reads "Explored 2 files, 1 search". */}
        {" "}<span className="explore-summary"><span aria-hidden="true">·</span> {activity ?? <Summary parts={parts} animate={animate} />}</span>
        {!activity && counts.failed > 0 && <>{" "}<span className="tool-row-failed">{counts.failed} failed</span></>}
        {" "}<span className="tool-row-status"><Icon name="chevron" className="tool-chevron" /></span>
      </button>
      {animate ? (running || busy) && (
        <motion.div
          className="explore-live"
          initial={false}
          animate={{ height: busy ? "auto" : 0 }}
          transition={busy ? LANE_OPEN : LANE_CLOSE}
          style={{ overflow: busy ? "visible" : "hidden" }}
        >
          {laneContent}
        </motion.div>
      ) : busy && <div className="explore-live">{laneContent}</div>}
      {pictured.length > 0 && (
        <div className="explore-images">
          {pictured.map(({ call, result }) => <ToolImages key={call.toolCallId} call={call} result={result} caption />)}
        </div>
      )}
      {open && <div className="explore-body">{renderBody(settled.items, results, liveToolText, running)}</div>}
    </div>
  );
}
