import { createContext, useContext, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import type { NormalizedBlock } from "../types";
import {
  coalesceWorkItems,
  exploreActivity,
  liveTailWindow,
  MAX_LIVE_VISIBLE_STEPS,
  settledGroup,
  summarizeWorkGroup,
  type CoalescedStep,
  type ExploreGroup as ExploreGroupData,
  type WorkBucketKey,
  type WorkSummary
} from "../explore-utils";
import { Icon, type IconName } from "./Icons";
import { OrbitSpinner } from "./OrbitSpinner";
import { RollingNumber } from "./RollingNumber";
import { ThinkingRow } from "./ThinkingRow";
import { CoalescedRow, ToolImages, useTranscriptMotion } from "./ToolRow";

/**
 * Whether adjacent work calls collapse into one timeline group (Settings › Appearance).
 * Sub-components read this rather than threading a prop down every turn wrapper.
 */
export const ExploreGroupingEnabled = createContext(true);

/**
 * The group a running transcript ends with while it waits on the model (`tailGroupKey`), and
 * the activity caption for that wait. That group's head reflects the waiting state via its
 * `OrbitSpinner` and inline status caption.
 */
export const ExploreLaneWait = createContext<{ key: string; label: string } | undefined>(undefined);

const LANE_CLOSE = { duration: 0.22, ease: [0.33, 1, 0.68, 1] } as const;
const BODY_REVEAL = { duration: 0.24, ease: [0.33, 1, 0.68, 1] } as const;

/**
 * Open group keys across transcript view swaps (`TranscriptViewState.exploration`), in the
 * host's `Set` shape (`scope` not needed because keys are call ids / timestamps).
 */
export const ExploreExpansion = createContext<Set<string> | undefined>(undefined);

const BUCKET_ICONS: Record<WorkBucketKey, IconName> = {
  create: "filePlus",
  edit: "pencil",
  write: "filePlus",
  command: "terminal",
  browser: "browser",
  computer: "cursor",
  mcp: "plug",
  canvas: "canvas",
  todo: "checklist",
  ask: "question",
  read: "file",
  image: "image",
  search: "search",
  list: "folder",
  web_search: "globe",
  web_fetch: "globe",
  memory: "memory",
  other: "wrench"
};

/** Natural-English summary clauses with rolling counts for every populated bucket. */
function WorkParts({ summary, animate }: { summary: WorkSummary; animate: boolean }) {
  const { parts, notes } = summary;
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

/** Shared uniform hairline step renderer for `.explore-body` and bounded `.explore-live`. */
function renderSteps(
  steps: CoalescedStep[],
  results: Map<string, NormalizedBlock>,
  liveToolText: Record<string, string> | undefined,
  running: boolean,
  options: { liveLane?: boolean } = {}
): ReactNode[] {
  return steps.map((step) => {
    if (step.type === "thinking") {
      return (
        <ThinkingRow
          key={step.key}
          text={step.item.block.text ?? ""}
          durationMs={step.item.block.durationMs}
          startedAt={step.item.block.startedAt}
          live={running && step.live}
          expansionKey={step.key}
        />
      );
    }
    if (step.type === "note") {
      return (
        <div key={`note:${step.key}`} className="work-note">
          <Icon name="comment" className="work-note-icon" />
          <span>{step.text}</span>
        </div>
      );
    }
    return (
      <CoalescedRow
        key={step.key}
        step={step}
        results={results}
        liveToolText={liveToolText}
        running={options.liveLane ? true : running}
        variant="step"
      />
    );
  });
}

interface ExploreGroupProps {
  group: ExploreGroupData;
  results: Map<string, NormalizedBlock>;
  liveToolText?: Record<string, string>;
  running: boolean;
}

/**
 * A contiguous run of work steps folded into a quiet collapsed summary line that expands
 * into WackCode's open-surface card with uniform hairline-divided step rows.
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
  const hasLiveRows = !open && (tail.steps.length > 0 || trailing.length > 0);
  const isWaitingOnly = exploring && livePendingCount === 0 && !trailing.length && wait !== undefined;

  const settled = settledGroup(group, results);
  const fullSummary = summarizeWorkGroup(group, results);
  const settledSummary = settled === group ? fullSummary : summarizeWorkGroup(settled, results);
  const settledSteps = coalesceWorkItems(settled.items, results);
  const allSteps = settled === group ? settledSteps : coalesceWorkItems(group.items, results);
  const leadIcon = BUCKET_ICONS[(settledSummary.parts[0] ?? fullSummary.parts[0])?.key ?? "other"] ?? "wrench";

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
        />
      ))}
    </>
  );

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) expanded?.add(group.key);
    else expanded?.delete(group.key);
  };

  const hasSettled = settledSummary.parts.length > 0 || settledSummary.notes > 0;
  const bodyContent = (
    <div className="explore-body">
      {renderSteps(allSteps, results, liveToolText, running)}
      {trailing.map((item) => (
        <ThinkingRow
          key={item.key}
          text={item.block.text ?? ""}
          durationMs={item.block.durationMs}
          startedAt={item.block.startedAt}
          live={running && item.streaming && item.block.durationMs === undefined}
          expansionKey={item.key}
        />
      ))}
    </div>
  );

  return (
    <div className={`explore-group ${open ? "open" : ""} ${exploring ? "live" : ""}`}>
      <button type="button" className="explore-head" onClick={toggle} aria-expanded={open} aria-busy={exploring || undefined}>
        <span className="explore-head-mark" aria-hidden="true">
          {open && !exploring
            ? <span className="explore-head-glyph"><Icon name={leadIcon} /></span>
            : <OrbitSpinner active={exploring} failed={fullSummary.failed > 0} />}
        </span>
        {hasSettled ? (
          <span className="explore-summary work-headline">
            <WorkParts summary={settledSummary} animate={animate} />
            {isWaitingOnly && (
              <>
                {" "}
                <span className="explore-wait-text">· {wait || "Thinking…"}</span>
              </>
            )}
          </span>
        ) : (
          <span className={`explore-summary ${exploring ? "thinking-shimmer" : ""}`}>
            {activity ?? (wait || (exploring ? "Working…" : "Worked"))}
          </span>
        )}
        {(settledSummary.additions > 0 || settledSummary.deletions > 0) && (
          <>
            {" "}
            <span className="tool-row-stats">
              {settledSummary.additions ? <em className="add"><RollingNumber prefix="+" value={settledSummary.additions} /></em> : null}
              {settledSummary.deletions ? <em className="del"><RollingNumber prefix="−" value={settledSummary.deletions} /></em> : null}
            </span>
          </>
        )}
        {fullSummary.failed > 0 && <>{" "}<span className="tool-row-failed">{fullSummary.failed} failed</span></>}
        {" "}<span className="tool-row-status"><Icon name="chevron" className="tool-chevron" /></span>
      </button>
      {animate ? (
        <AnimatePresence initial={false}>
          {hasLiveRows && (
            <motion.div
              key="live"
              className="explore-live"
              initial={false}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={LANE_CLOSE}
              style={{ overflow: "hidden" }}
            >
              {laneContent}
            </motion.div>
          )}
        </AnimatePresence>
      ) : hasLiveRows && <div className="explore-live">{laneContent}</div>}
      {pictured.length > 0 && (
        <div className="explore-images">
          {pictured.map(({ call, result }) => <ToolImages key={call.toolCallId} call={call} result={result} caption />)}
        </div>
      )}
      {animate ? (
        <AnimatePresence initial={false}>
          {open && (
            <motion.div
              key="body"
              className="explore-body-reveal"
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={BODY_REVEAL}
            >
              {bodyContent}
            </motion.div>
          )}
        </AnimatePresence>
      ) : open && bodyContent}
    </div>
  );
}
