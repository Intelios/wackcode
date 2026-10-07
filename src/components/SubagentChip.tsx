import { createContext, useContext } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { formatRunDuration } from "../chat-utils";
import { displayAgentName } from "../tool-utils";
import type { SubagentDetails, SubagentResult, SubagentTarget } from "../types";
import { Icon } from "./Icons";
import { RobotMark, type RobotStatus } from "./RobotMark";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

export interface SubagentPanelLinkValue {
  /** The child the side panel is showing, if any. */
  open?: SubagentTarget;
  /** Show this child in the side panel, or close the panel when it is the one showing. */
  onOpen?: (toolCallId: string, index: number) => void;
}

/**
 * Links the transcript's chips to the side panel. Provided by App as a context, so selecting a
 * sub-agent re-renders only the chips, not the memoized messages around them.
 */
export const SubagentPanelLink = createContext<SubagentPanelLinkValue>({});

/**
 * A child's status as its chip and panel show it. Background cards keep `live` independent
 * of their launch tool result; foreground cards still settle with the blocking call.
 */
export function childStatus(result: SubagentResult, live: boolean): RobotStatus {
  if (live && (result.status === "running" || result.status === "queued")) return result.status;
  if (result.status === "done" || result.status === "failed") return result.status;
  return "stopped";
}

export const STATUS_WORDS: Record<RobotStatus, string> = {
  running: "Working",
  queued: "Queued",
  done: "Done",
  failed: "Failed",
  stopped: "Stopped"
};

const STEP_VERBS: Record<string, string> = {
  read: "Reading",
  grep: "Searching",
  find: "Finding",
  ls: "Listing",
  bash: "Running",
  edit: "Editing",
  write: "Writing",
  web_fetch: "Fetching"
};

/** A child's latest tool call as its chip's ticker reads it: "Reading session.ts". */
export function stepLabel(step: { tool: string; subject: string }): { verb: string; subject: string } {
  const file = step.tool === "read" || step.tool === "edit" || step.tool === "write";
  return {
    verb: STEP_VERBS[step.tool] ?? step.tool,
    subject: file ? step.subject.split("/").filter(Boolean).pop() ?? step.subject : step.subject
  };
}

/** "2 running · 1 done": a parallel call's children at a glance. */
function groupSummary(results: SubagentResult[], live: boolean): string {
  const counts = new Map<RobotStatus, number>();
  for (const result of results) {
    const status = childStatus(result, live);
    counts.set(status, (counts.get(status) ?? 0) + 1);
  }
  return (["running", "queued", "done", "failed", "stopped"] as const)
    .filter((status) => counts.has(status))
    .map((status) => `${counts.get(status)} ${status === "running" ? "working" : STATUS_WORDS[status].toLowerCase()}`)
    .join(" · ");
}

interface ChipProps {
  toolCallId: string;
  index: number;
  result: SubagentResult;
  live: boolean;
}

function SubagentChip({ toolCallId, index, result, live }: ChipProps) {
  const reduce = useReducedMotion();
  const { open, onOpen } = useContext(SubagentPanelLink);
  const selected = open?.toolCallId === toolCallId && open.index === index;
  const status = childStatus(result, live);
  const role = displayAgentName(result.agent);
  const latest = result.activity[result.activity.length - 1];
  const step = status === "running" && latest ? stepLabel(latest) : undefined;
  const duration = result.startedAt !== undefined && result.endedAt !== undefined ? result.endedAt - result.startedAt : undefined;
  const state = status === "done" && duration !== undefined ? formatRunDuration(duration) : STATUS_WORDS[status];

  return (
    <motion.button
      type="button"
      className={`subagent-chip ${status} ${selected ? "selected" : ""}`}
      aria-label={`SubAgent ${role}, ${STATUS_WORDS[status].toLowerCase()}: ${result.task}. Access: ${result.readOnly ? "read-only" : "can edit files"}.`}
      aria-expanded={selected}
      aria-controls="side-panel"
      disabled={!onOpen}
      onClick={() => onOpen?.(toolCallId, index)}
      // Only a chip appearing during a run rises in; history renders in place.
      initial={reduce || !live ? false : { opacity: 0, y: 8, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={{ duration: reduce ? 0 : 0.34, ease: EASE, delay: reduce ? 0 : index * 0.06 }}
      whileTap={reduce ? undefined : { scale: 0.985 }}
    >
      <RobotMark status={status} />
      <span className="subagent-chip-kind">SubAgent</span>
      <span className="subagent-chip-role">{role}</span>
      {!result.readOnly && <span className="subagent-tag">edits</span>}
      <span className="subagent-chip-task">{result.task}</span>
      <span className="subagent-chip-status">
        {step ? (
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={`${result.activity.length}:${latest.tool}:${latest.subject}`}
              className="subagent-chip-step"
              initial={reduce ? false : { opacity: 0, y: 7 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -7 }}
              transition={{ duration: reduce ? 0 : 0.22, ease: EASE }}
            >
              <span className="subagent-chip-verb">{step.verb}</span>
              {step.subject && <code>{step.subject}</code>}
            </motion.span>
          </AnimatePresence>
        ) : (
          <span className="subagent-chip-state">{state}</span>
        )}
      </span>
      <Icon name="panel" className="subagent-chip-open" />
    </motion.button>
  );
}

interface GroupProps {
  toolCallId: string;
  details: SubagentDetails;
  /** The child jobs are live, even if a background launch has already returned. */
  live: boolean;
}

/**
 * One `subagent` call in the transcript: a SubAgent chip per child, each opening that child's
 * live transcript in the side panel. A parallel call's chips share a rail under a one-line
 * summary. Rendered from the tool result's details, so it reads the same after a reload.
 */
export function SubagentGroup({ toolCallId, details, live }: GroupProps) {
  const parallel = details.results.length > 1;
  return (
    <div className={`subagent-group ${parallel ? "parallel" : ""}`}>
      {parallel && (
        <div className="subagent-group-head">
          <span>{details.results.length} SubAgents in parallel</span>
          <span className="subagent-group-summary">{groupSummary(details.results, live)}</span>
        </div>
      )}
      {details.results.map((result, index) => (
        <SubagentChip key={index} toolCallId={toolCallId} index={index} result={result} live={live} />
      ))}
    </div>
  );
}
