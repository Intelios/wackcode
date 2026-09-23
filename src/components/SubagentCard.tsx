import { useState } from "react";
import { formatRunDuration, formatTokens } from "../chat-utils";
import type { SubagentDetails, SubagentResult } from "../types";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";

interface Props {
  details: SubagentDetails;
  /** The call is still in flight: children marked running are live, not interrupted. */
  running: boolean;
}

const STATUS_LABELS: Record<SubagentResult["status"], string> = {
  queued: "Queued",
  running: "Running",
  done: "Done",
  failed: "Failed",
  aborted: "Stopped"
};

function usageTokens(result: SubagentResult): number {
  const { input, output, cacheRead, cacheWrite } = result.usage;
  return input + output + cacheRead + cacheWrite;
}

function formatCost(cost: number): string {
  return cost >= 0.01 ? `$${cost.toFixed(2)}` : `$${cost.toFixed(4)}`;
}

function summary(results: SubagentResult[], running: boolean): string {
  const count = (status: SubagentResult["status"]) => results.filter((result) => result.status === status).length;
  const parts: string[] = [];
  if (running && count("running")) parts.push(`${count("running")} running`);
  if (running && count("queued")) parts.push(`${count("queued")} queued`);
  if (count("done")) parts.push(`${count("done")} done`);
  if (count("failed")) parts.push(`${count("failed")} failed`);
  if (count("aborted") || (!running && count("running") + count("queued"))) {
    parts.push(`${count("aborted") + (running ? 0 : count("running") + count("queued"))} stopped`);
  }
  return parts.join(" · ");
}

function StatusMark({ status }: { status: SubagentResult["status"] }) {
  const mark = status === "running" ? <span className="tool-spinner" />
    : status === "done" ? <Icon name="check" className="subagent-mark done" />
    : status === "failed" ? <Icon name="close" className="subagent-mark failed" />
    : status === "queued" ? <span className="subagent-mark queued" />
    : <Icon name="stop" className="subagent-mark stopped" />;
  return <span className="subagent-status" role="img" aria-label={STATUS_LABELS[status]}>{mark}</span>;
}

function SubagentRow({ result, live }: { result: SubagentResult; live: boolean }) {
  const [open, setOpen] = useState(false);
  const active = live && (result.status === "running" || result.status === "queued");
  const latest = result.activity[result.activity.length - 1];
  const tokens = usageTokens(result);
  const status = active || result.status === "done" || result.status === "failed" ? result.status : "aborted";
  const duration = result.startedAt !== undefined && result.endedAt !== undefined ? result.endedAt - result.startedAt : undefined;

  return (
    <li className={`subagent-row ${status} ${open ? "open" : ""}`}>
      <button type="button" className="subagent-row-head" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        <StatusMark status={status} />
        <span className="subagent-name">{result.agent}</span>
        {!result.readOnly && <span className="subagent-tag">edits</span>}
        <span className="subagent-task" title={result.task}>{result.task}</span>
        <span className="subagent-meta">
          {tokens > 0 && <span>{formatTokens(tokens)} tokens</span>}
          <Icon name="chevron" className="tool-chevron" />
        </span>
      </button>
      {active && result.status === "running" && latest && !open && (
        <div className="subagent-live">
          <span className="subagent-live-tool">{latest.tool}</span>
          {latest.subject && <code>{latest.subject}</code>}
        </div>
      )}
      {!open && result.status === "failed" && result.error && <div className="subagent-error">{result.error}</div>}
      {open && (
        <div className="subagent-body">
          <p className="subagent-full-task">{result.task}</p>
          {result.activity.length > 0 && (
            <ol className="subagent-activity" aria-label={`${result.agent} tool calls`}>
              {result.activity.map((call, index) => (
                <li key={index}>
                  <span className="subagent-live-tool">{call.tool}</span>
                  {call.subject && <code>{call.subject}</code>}
                </li>
              ))}
            </ol>
          )}
          {result.error && <div className="subagent-error">{result.error}</div>}
          {result.output && (
            <div className="subagent-output">
              <Markdown>{result.output}</Markdown>
              {result.outputTruncated && <p className="subagent-note">Shortened here. The agent received the full answer.</p>}
            </div>
          )}
          <div className="subagent-usage">
            {result.model && <span>{result.model}</span>}
            {result.usage.turns > 0 && <span>{result.usage.turns} {result.usage.turns === 1 ? "turn" : "turns"}</span>}
            <span>↑{formatTokens(result.usage.input)} ↓{formatTokens(result.usage.output)}</span>
            {result.usage.cacheRead > 0 && <span>cache {formatTokens(result.usage.cacheRead)}</span>}
            {result.usage.cost > 0 && <span>{formatCost(result.usage.cost)}</span>}
            {duration !== undefined && <span>{formatRunDuration(duration)}</span>}
          </div>
        </div>
      )}
    </li>
  );
}

/**
 * One `subagent` call in the transcript: a row per child with its status, latest tool call
 * while it works, and its answer and usage once expanded. Rendered from the tool result's
 * details, so it reads the same after a reload as it did live.
 */
export function SubagentCard({ details, running }: Props) {
  const label = details.results.length === 1 ? "Sub-agent" : `${details.results.length} sub-agents`;
  const total = details.results.reduce((sum, result) => sum + usageTokens(result), 0);
  const cost = details.results.reduce((sum, result) => sum + result.usage.cost, 0);
  return (
    <div className={`subagent-card ${running ? "running" : ""}`}>
      <div className="subagent-card-head">
        <Icon name="agents" />
        <span className="subagent-card-title">{label}</span>
        <span className="subagent-card-summary">{summary(details.results, running)}</span>
        <span className="subagent-card-total">
          {total > 0 && `${formatTokens(total)} tokens`}
          {cost > 0 && ` · ${formatCost(cost)}`}
        </span>
      </div>
      <ul className="subagent-list">
        {details.results.map((result, index) => (
          <SubagentRow key={index} result={result} live={running} />
        ))}
      </ul>
    </div>
  );
}
