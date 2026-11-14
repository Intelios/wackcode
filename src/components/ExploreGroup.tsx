import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { animate as play, motion } from "motion/react";
import type { NormalizedBlock } from "../types";
import {
  coalesceWorkItems,
  exploreActivity,
  exploreMarks,
  flyId,
  liveTailWindow,
  MAX_LIVE_VISIBLE_STEPS,
  settledGroup,
  summarizeWorkGroup,
  type CoalescedStep,
  type ExploreGroup as ExploreGroupData,
  type ExploreMark,
  type WorkSummary
} from "../explore-utils";
import { toolPresentation } from "../tool-utils";
import { Icon, type IconName } from "./Icons";
import { RollingNumber } from "./RollingNumber";
import { ThinkingRow } from "./ThinkingRow";
import { contentBox, takeDuckBox } from "./thoughtFlight";
import { buildCoalescedProps, CoalescedRow, FLY, ToolChips, ToolImages, useTranscriptMotion, type ChipCall } from "./ToolRow";

/**
 * Whether adjacent work calls collapse into one timeline group (Settings › Appearance).
 * Sub-components read this rather than threading a prop down every turn wrapper.
 */
export const ExploreGroupingEnabled = createContext(true);

/**
 * The group a running transcript ends with while it waits on the model (`tailGroupKey`), and
 * the activity caption for that wait. That group's `.explore-live` lane holds the waiting dots.
 */
export const ExploreLaneWait = createContext<{ key: string; label: string } | undefined>(undefined);

const LANE_CLOSE = { duration: 0.24, ease: [0.33, 1, 0.68, 1] } as const;
const LANE_OPEN = { duration: 0 } as const;

/**
 * Open group keys across transcript view swaps (`TranscriptViewState.exploration`), in the
 * host's `Set` shape (`scope` not needed because keys are call ids / timestamps).
 */
export const ExploreExpansion = createContext<Set<string> | undefined>(undefined);

const MARK_ICONS: Record<ExploreMark, IconName> = {
  write: "filePlus",
  edit: "pencil",
  file: "file",
  image: "image",
  search: "search",
  list: "folder",
  command: "terminal",
  web: "globe",
  browser: "browser",
  computer: "cursor",
  memory: "memory",
  mcp: "plug",
  other: "wrench",
  note: "comment",
  thought: "brain"
};

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
      <motion.span className="explore-mark-glyph" layout={animate && !thought} transition={{ layout: FLY }}>
        <Icon name={MARK_ICONS[mark]} />
      </motion.span>
    </motion.span>
  );
}

/** Pure exploration summary: "1 file, 2 searches", with rolling counts. */
function ExploreParts({ parts, animate }: { parts: WorkSummary["exploreParts"]; animate: boolean }) {
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

/** Tiered work summary: "Edited 2 files, ran 2 commands · 1 note", with rolling counts. */
function WorkParts({ summary, animate }: { summary: WorkSummary; animate: boolean }) {
  const { parts, overflowCount, notes } = summary;
  return (
    <>
      {parts.map((part, index) => (
        <span key={part.key} className="explore-count">
          {index > 0 && ", "}
          {index === 0 && part.leadPhrase ? (
            part.leadPhrase
          ) : index > 0 && part.trailPhrase ? (
            part.trailPhrase
          ) : (
            <>
              {(index === 0 ? part.leadVerb : part.trailVerb) && `${index === 0 ? part.leadVerb : part.trailVerb} `}
              {part.count !== undefined && (animate ? <RollingNumber value={part.count} /> : part.count)}
              {part.noun ? ` ${part.noun}` : ""}
            </>
          )}
        </span>
      ))}
      {overflowCount > 0 && (
        <span className="explore-count">
          {parts.length > 0 ? ", and " : ""}
          {animate ? <RollingNumber value={overflowCount} /> : overflowCount}
          {overflowCount === 1 ? " more step" : " more steps"}
        </span>
      )}
      {notes > 0 && (
        <>
          {" "}
          <span className="explore-notes">
            · {notes} {notes === 1 ? "note" : "notes"}
          </span>
        </>
      )}
    </>
  );
}

/** Shared step renderer for both `.explore-body` (all settled steps) and `.explore-live` (bounded tail). */
function renderSteps(
  steps: CoalescedStep[],
  results: Map<string, NormalizedBlock>,
  liveToolText: Record<string, string> | undefined,
  running: boolean,
  options: { liveLane?: boolean } = {}
): ReactNode[] {
  const out: ReactNode[] = [];
  let strip: ChipCall[] = [];
  let stripKey = "";

  const flush = () => {
    if (!strip.length) return;
    out.push(<ToolChips key={`chips:${stripKey}`} entries={strip} />);
    strip = [];
    stripKey = "";
  };

  for (const step of steps) {
    if (step.type === "thinking") {
      flush();
      out.push(
        <ThinkingRow
          key={step.key}
          text={step.item.block.text ?? ""}
          durationMs={step.item.block.durationMs}
          startedAt={step.item.block.startedAt}
          live={running && step.live}
          expansionKey={step.key}
          flyId={step.flyId}
        />
      );
      continue;
    }
    if (step.type === "note") {
      flush();
      out.push(
        <div key={`note:${step.key}`} className="work-note">
          <Icon name="comment" className="work-note-icon" />
          <span>{step.text}</span>
        </div>
      );
      continue;
    }

    const props = buildCoalescedProps(step, results, liveToolText, options.liveLane ? true : running);
    // Settled failed steps pinned inside the live lane already exist on the head stack: suppress
    // `flyId` on pinned settled items so Framer Motion never mounts duplicate layoutIds.
    const fly = options.liveLane && !step.live ? undefined : props.flyId;
    if (toolPresentation(props.call) === "chip") {
      if (!strip.length) stripKey = step.key;
      strip.push({ ...props, flyId: fly });
    } else {
      flush();
      out.push(
        <CoalescedRow
          key={step.key}
          step={step}
          results={results}
          liveToolText={liveToolText}
          running={options.liveLane ? true : running}
          compact
        />
      );
    }
  }

  flush();
  return out;
}

interface ExploreGroupProps {
  group: ExploreGroupData;
  results: Map<string, NormalizedBlock>;
  liveToolText?: Record<string, string>;
  running: boolean;
}

/**
 * A contiguous run of work steps collapsed onto one compact head row (`ExploreGroup` + unified
 * multi-step timeline). In-flight calls stream inside a bounded live lane beneath the head and
 * roll into the head counts as they settle.
 */
export function ExploreGroup({ group, results, liveToolText, running }: ExploreGroupProps) {
  const expanded = useContext(ExploreExpansion);
  const animate = useTranscriptMotion();
  const [open, setOpen] = useState(() => expanded?.has(group.key) === true);
  const activity = exploreActivity(group, results);
  const tail = liveTailWindow(group, results, MAX_LIVE_VISIBLE_STEPS, { includeFailed: !open });
  const trailing = group.trailing ?? [];
  const lane = useContext(ExploreLaneWait);
  const wait = lane?.key === group.key ? lane.label : undefined;
  const livePendingCount = tail.steps.filter((s) => s.live).length + tail.hiddenCount;
  const exploring = Boolean(activity) || trailing.length > 0 || wait !== undefined;
  const busy = tail.steps.length > 0 || trailing.length > 0 || wait !== undefined;

  const settled = settledGroup(group, results);
  const fullSummary = summarizeWorkGroup(group, results);
  const settledSummary = settled === group ? fullSummary : summarizeWorkGroup(settled, results);
  const marks = exploreMarks(settled, results).slice(0, 5);
  const settledSteps = coalesceWorkItems(settled.items, results);

  // A brief glow on the head each time something joins it (never on mount, nor for history).
  const joined = settled.items.length;
  const lastJoined = useRef(joined);
  const [pulse, setPulse] = useState(0);
  useEffect(() => {
    if (joined > lastJoined.current && animate) setPulse((value) => value + 1);
    lastJoined.current = joined;
  }, [joined, animate]);

  // Pictures returned by folded calls stay visible below the head until expanded.
  const pictured = open ? [] : settled.items.flatMap(({ block }) => {
    const result = block.type === "tool-call" && block.toolCallId ? results.get(block.toolCallId) : undefined;
    return result?.images?.length ? [{ call: block, result }] : [];
  });

  const laneContent = (
    <>
      {tail.steps.length > 0 && renderSteps(tail.steps, results, liveToolText, running, { liveLane: true })}
      {tail.hiddenCount > 0 && (
        <span className="work-live-more">+{tail.hiddenCount} running</span>
      )}
      {trailing.map((item) => (
        <ThinkingRow
          key={item.key}
          text={item.block.text ?? ""}
          durationMs={item.block.durationMs}
          startedAt={item.block.startedAt}
          live={running && item.streaming && item.block.durationMs === undefined}
          expansionKey={item.key}
          flyId={flyId(item)}
        />
      ))}
      {wait !== undefined && livePendingCount === 0 && !trailing.length && (
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

  const renderHeadText = () => {
    if (fullSummary.isPureExploration) {
      return (
        <>
          {exploring
            ? <span className="explore-verb thinking-shimmer">Exploring</span>
            : <span className="explore-verb">Explored</span>}
          {/* Spaces between the parts, so the button's name reads "Explored 2 files, 1 search". */}
          {" "}
          <span className="explore-summary">
            <span aria-hidden="true">·</span>{" "}
            {activity ?? <ExploreParts parts={settledSummary.exploreParts} animate={animate} />}
          </span>
        </>
      );
    }

    if (exploring && activity) {
      return (
        <>
          <span className="explore-verb thinking-shimmer">Working</span>
          {" "}
          <span className="explore-summary"><span aria-hidden="true">·</span> {activity}</span>
        </>
      );
    }

    const displaySummary = settled === group ? fullSummary : settledSummary;
    const hasLead = displaySummary.parts.length > 0;
    return (
      <>
        {exploring ? (
          <span className="explore-verb thinking-shimmer">Working</span>
        ) : !hasLead ? (
          <span className="explore-verb">Worked</span>
        ) : null}
        {(exploring || !hasLead) && hasLead ? " " : ""}
        <span className={exploring || !hasLead ? "explore-summary" : "explore-summary work-headline"}>
          {exploring && hasLead && <><span aria-hidden="true">·</span>{" "}</>}
          <WorkParts summary={displaySummary} animate={animate} />
        </span>
      </>
    );
  };

  return (
    <div className={`explore-group ${open ? "open" : ""} ${exploring ? "live" : ""}`}>
      <button type="button" className="explore-head" onClick={toggle} aria-expanded={open}>
        {pulse > 0 && <span key={pulse} className="explore-pulse" aria-hidden="true" />}
        <span className="explore-stack" aria-hidden="true">
          {marks.map(({ mark, id }) => <Mark key={id} mark={mark} id={id} animate={animate} />)}
        </span>
        {renderHeadText()}
        {!activity && (settledSummary.additions > 0 || settledSummary.deletions > 0) && (
          <>
            {" "}
            <span className="tool-row-stats">
              {settledSummary.additions ? <em className="add"><RollingNumber prefix="+" value={settledSummary.additions} /></em> : null}
              {settledSummary.deletions ? <em className="del"><RollingNumber prefix="−" value={settledSummary.deletions} /></em> : null}
            </span>
          </>
        )}
        {!activity && fullSummary.failed > 0 && <>{" "}<span className="tool-row-failed">{fullSummary.failed} failed</span></>}
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
      {open && <div className="explore-body">{renderSteps(settledSteps, results, liveToolText, running)}</div>}
    </div>
  );
}
