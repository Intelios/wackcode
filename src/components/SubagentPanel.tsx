import { useContext, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { AssistantNameContext } from "../agentName";
import { formatCost, formatRunDuration, formatTokens } from "../chat-utils";
import { displayAgentName } from "../tool-utils";
import type { SubagentDetails, SubagentResult, SubagentView } from "../types";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";
import { RobotMark, type RobotStatus } from "./RobotMark";
import { STATUS_WORDS, childStatus, stepLabel } from "./SubagentChip";
import { Transcript } from "./Transcript";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

/** Siblings slide in from the side of the tab they were picked from; the old page clears out quickly. */
function pageVariants(reduce: boolean) {
  return {
    enter: (direction: number) => ({ opacity: 0, x: 22 * direction }),
    center: { opacity: 1, x: 0 },
    exit: (direction: number) => ({ opacity: 0, x: -16 * direction, transition: { duration: reduce ? 0 : 0.16, ease: EASE } })
  };
}

interface SubagentPanelProps {
  toolCallId: string;
  index: number;
  /** The call's details as its chips show them. Undefined for a moment when a stopped call's
   *  result hasn't landed yet: the panel keeps showing the last it had. */
  details?: SubagentDetails;
  /** The call is still running. */
  live: boolean;
  /** The watched transcript (`TaskRuntime.subagentView`), when it is this child's. */
  stream?: SubagentView;
  onSelect: (index: number) => void;
  onClose: () => void;
  /** Watch again after the watch itself failed. */
  onRetry: () => void;
}

/** Tab and title labels: a role that appears more than once in the call is numbered. */
export function siblingLabels(results: SubagentResult[]): string[] {
  const totals = new Map<string, number>();
  for (const result of results) totals.set(result.agent, (totals.get(result.agent) ?? 0) + 1);
  const seen = new Map<string, number>();
  return results.map((result) => {
    const role = displayAgentName(result.agent);
    if ((totals.get(result.agent) ?? 0) < 2) return role;
    const nth = (seen.get(result.agent) ?? 0) + 1;
    seen.set(result.agent, nth);
    return `${role} ${nth}`;
  });
}

/** The current time, ticking once a second while `active`. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

function StatusPill({ status, startedAt, endedAt }: { status: RobotStatus; startedAt?: number; endedAt?: number }) {
  const now = useNow(status === "running");
  const elapsed = startedAt === undefined || status === "queued" ? undefined : (status === "running" ? now : endedAt ?? now) - startedAt;
  return (
    <span className={`subagent-pill ${status}`} role="status">
      <span className="subagent-pill-dot" aria-hidden="true" />
      {STATUS_WORDS[status]}
      {elapsed !== undefined && <span className="subagent-pill-time">{formatRunDuration(elapsed)}</span>}
    </span>
  );
}

function SiblingTabs({ toolCallId, results, index, live, labels, onSelect }: {
  toolCallId: string;
  results: SubagentResult[];
  index: number;
  live: boolean;
  labels: string[];
  onSelect: (index: number) => void;
}) {
  const reduce = useReducedMotion();
  return (
    <div className="subagent-tabs" role="tablist" aria-label="SubAgents in this call">
      {results.map((result, position) => {
        const active = position === index;
        return (
          <button
            key={position}
            type="button"
            role="tab"
            aria-selected={active}
            className={`subagent-tab ${active ? "active" : ""}`}
            onClick={() => onSelect(position)}
          >
            {active && (
              <motion.span
                layoutId={`subagent-tab-pill:${toolCallId}`}
                className="subagent-tab-pill"
                transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 520, damping: 38 }}
              />
            )}
            <span className={`subagent-tab-dot ${childStatus(result, live)}`} aria-hidden="true" />
            <span className="subagent-tab-label">{labels[position]}</span>
          </button>
        );
      })}
    </div>
  );
}

function Brief({ task }: { task: string }) {
  const [open, setOpen] = useState(false);
  const agent = useContext(AssistantNameContext);
  const long = task.length > 220 || task.split("\n").length > 3;
  return (
    <section className={`subagent-brief ${open ? "open" : ""}`} aria-label="Brief">
      <div className="subagent-brief-head">
        <Icon name="comment" />
        <span>Brief from {agent}</span>
        {long && (
          <button type="button" className="subagent-brief-toggle" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
            {open ? "Show less" : "Show all"}
          </button>
        )}
      </div>
      <p className="subagent-brief-text">{task}</p>
    </section>
  );
}

/**
 * Everything known without a transcript: none was saved (`legacy`: a call from before
 * transcripts were kept, or a child a worker restart cut off), or it ended before it said anything.
 */
function Summary({ result, legacy }: { result: SubagentResult; legacy: boolean }) {
  return (
    <div className="subagent-summary">
      {legacy && <p className="subagent-note">No transcript was saved for this SubAgent, so here’s what it reported.</p>}
      {!legacy && !result.output && result.activity.length === 0 && <p className="subagent-note">It stopped before it said anything.</p>}
      {result.activity.length > 0 && (
        <ol className="subagent-activity" aria-label="Recent tool calls">
          {result.activity.map((call, position) => {
            const step = stepLabel(call);
            return (
              <li key={position}>
                <span className="subagent-live-tool">{step.verb}</span>
                {call.subject && <code>{call.subject}</code>}
              </li>
            );
          })}
        </ol>
      )}
      {result.output && (
        <div className="subagent-output">
          <Markdown>{result.output}</Markdown>
          {result.outputTruncated && <p className="subagent-note">Shortened here. The agent received the full answer.</p>}
        </div>
      )}
    </div>
  );
}

/** Before the first message: dozing in the queue, or warming up. */
function Waiting({ status }: { status: RobotStatus }) {
  return (
    <div className="subagent-waiting">
      <RobotMark status={status} className="subagent-waiting-robot" />
      <strong>{status === "queued" ? "Waiting for a free slot" : "Warming up…"}</strong>
      <span>{status === "queued" ? "It starts as soon as another SubAgent in this call finishes." : "Its first steps will appear here as it takes them."}</span>
    </div>
  );
}

function Usage({ result, status }: { result: SubagentResult; status: RobotStatus }) {
  const agent = useContext(AssistantNameContext);
  const { usage } = result;
  const duration = result.startedAt !== undefined && result.endedAt !== undefined ? result.endedAt - result.startedAt : undefined;
  // Nothing to report yet (a queued child): no empty bar.
  if (status !== "done" && !result.model && usage.turns === 0 && usage.input + usage.output === 0 && duration === undefined) return null;
  return (
    <footer className="subagent-usage-bar">
      {status === "done" && <span className="subagent-returned"><Icon name="check" /> Returned to {agent}</span>}
      <span className="subagent-usage-items">
        {result.model && <span>{result.model}</span>}
        {usage.turns > 0 && <span>{usage.turns} {usage.turns === 1 ? "turn" : "turns"}</span>}
        {usage.input + usage.output > 0 && <span>↑{formatTokens(usage.input)} ↓{formatTokens(usage.output)}</span>}
        {usage.cacheRead > 0 && <span>cache {formatTokens(usage.cacheRead)}</span>}
        {usage.cost > 0 && <span>{formatCost(usage.cost)}</span>}
        {duration !== undefined && <span>{formatRunDuration(duration)}</span>}
      </span>
    </footer>
  );
}

function Body({ result, status, stream, onRetry }: { result: SubagentResult; status: RobotStatus; stream?: SubagentView; onRetry: () => void }) {
  if (stream?.error) {
    return (
      <div className="subagent-waiting">
        <strong>Couldn’t follow this SubAgent</strong>
        <span>{stream.error}</span>
        <button type="button" className="secondary-button compact" onClick={onRetry}><Icon name="refresh" /> Try again</button>
      </div>
    );
  }
  if (!stream || stream.loading) {
    return (
      <div className="subagent-skeleton" aria-label="Loading transcript" role="status">
        <span /><span /><span />
      </div>
    );
  }
  const empty = stream.messages.length === 0 && !stream.partial;
  if (empty && (status === "queued" || status === "running")) return <Waiting status={status} />;
  if (stream.missing || empty) return <Summary result={result} legacy={stream.missing} />;
  return <Transcript messages={stream.messages} partial={stream.partial} running={stream.live} collapseCompletedWork={false} />;
}

/**
 * One sub-agent in the side panel: who it is and how it is doing, what it was asked, and its
 * transcript, live while it works. A parallel call's children are tabs; moving between them
 * slides the page and leaves the tabs in place. Presentational: App watches the child and
 * passes its transcript in as `stream`.
 */
export function SubagentPanel({ toolCallId, index, details, live, stream, onSelect, onClose, onRetry }: SubagentPanelProps) {
  const reduce = useReducedMotion();
  const known = useRef(details);
  if (details) known.current = details;
  const shown = details ?? known.current;
  const previousIndex = useRef(index);
  const direction = index < previousIndex.current ? -1 : 1;
  useEffect(() => { previousIndex.current = index; }, [index]);

  const results = shown?.results ?? [];
  const result = results[index];
  const labels = siblingLabels(results);
  const status = result ? childStatus(result, live) : "stopped";
  const watched = stream && stream.toolCallId === toolCallId && stream.index === index ? stream : undefined;

  return (
    <div className="subagent-panel">
      <header className="panel-header subagent-panel-header">
        <RobotMark status={status} boot className="subagent-panel-robot" />
        <div className="panel-title">
          <span className="panel-kicker">SubAgent</span>
          <h3>{labels[index] ?? "SubAgent"}</h3>
        </div>
        {result && <StatusPill status={status} startedAt={result.startedAt} endedAt={result.endedAt} />}
        <div className="panel-header-actions">
          <button type="button" className="icon-button" onClick={onClose} aria-label="Close SubAgent panel"><Icon name="close" /></button>
        </div>
        {status === "running" && <span className="panel-loading" aria-hidden="true" />}
      </header>
      {results.length > 1 && (
        <SiblingTabs toolCallId={toolCallId} results={results} index={index} live={live} labels={labels} onSelect={onSelect} />
      )}
      {result && (
        <div className="subagent-pages">
          <AnimatePresence mode="popLayout" initial={false} custom={direction}>
            <motion.div
              key={index}
              className="subagent-page"
              custom={direction}
              variants={pageVariants(reduce === true)}
              initial={reduce ? false : "enter"}
              animate="center"
              exit="exit"
              transition={{ duration: reduce ? 0 : 0.26, ease: EASE }}
            >
              <Brief task={result.task} />
              {watched?.truncated && <p className="subagent-note subagent-trimmed">Older tool output was trimmed to keep this chat’s history small.</p>}
              <div className="subagent-page-body">
                <Body result={result} status={status} stream={watched} onRetry={onRetry} />
              </div>
              {result.error && <div className="subagent-error">{result.error}</div>}
              <Usage result={result} status={status} />
            </motion.div>
          </AnimatePresence>
        </div>
      )}
    </div>
  );
}
