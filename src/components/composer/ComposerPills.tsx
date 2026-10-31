import { useEffect, useMemo, useState } from "react";
import type { GitChanges, NormalizedBlock, NormalizedMessage, SessionSnapshot, TodoTask } from "../../types";
import { changeEntries } from "../../changes-utils";
import { formatRunDuration } from "../../chat-utils";
import { summarizeTool } from "../../tool-utils";
import { Icon } from "../Icons";
import { TodoPill } from "../TodoPanel";
import { Tooltip } from "../ui/Tooltip";

export interface PillDetail {
  /** Verb shown plain ("Reading"); empty when the subject carries the line ("pnpm test"). */
  verb?: string;
  subject: string;
  /** Subjects that are paths or commands keep the transcript's mono treatment. */
  subjectMono?: boolean;
}

interface WorkingPillProps {
  /** The live run's start; the timer ticks from it. Absent detail-only states omit it. */
  run?: { startedAt: number };
  detail?: PillDetail;
}

interface ComposerPillsProps {
  run?: WorkingPillProps["run"];
  /** In-flight tool calls keyed by call id; the most recently started one is shown. */
  liveToolCalls?: Record<string, NormalizedBlock>;
  /** The streaming message: an open thinking block reads as "Thinking…". */
  partial?: NormalizedMessage;
  compaction?: SessionSnapshot["compaction"];
  activity?: string;
  todos?: TodoTask[];
  todoBusy?: boolean;
  /** The chat's working-tree changes; the files pill shows once any exist. */
  changes?: GitChanges;
  /** The side panel is already on its Changes view (the pill reads as pressed). */
  changesOpen?: boolean;
  /** Opens (or closes) the Changes view — the header's ⌘⇧C toggle. */
  onToggleChanges?: () => void;
}

/** The live clock shared by the transcript's chip and the classic composer's floating pill:
 *  accent ring, spinning hand, tabular seconds. */
export function WorkingPill({ run, detail }: WorkingPillProps) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (run === undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [run]);
  return (
    <div className={`composer-pill composer-pill-working${run ? " live" : ""}`}>
      <Icon name="clock" />
      {run && <>Working for <strong>{formatRunDuration(Math.max(0, now - run.startedAt))}</strong></>}
      {detail && (
        <span className="composer-pill-detail" title={detail.verb ? `${detail.verb} ${detail.subject}`.trim() : detail.subject}>
          {detail.verb && <>{detail.verb} </>}
          <span className={detail.subjectMono ? "mono" : undefined}>{detail.subject}</span>
        </span>
      )}
    </div>
  );
}

/**
 * What follows the timer in the working pill: the live tool's summary ("pnpm test providers"),
 * or the run's state ("Thinking…", "Compacting context…"). Plain streaming and idle gaps show
 * nothing after the time. One detail at a time, in tool → thinking → compaction → activity order.
 */
export function runDetail({ liveToolCalls, partial, compaction, activity }: Pick<ComposerPillsProps, "liveToolCalls" | "partial" | "compaction" | "activity">): PillDetail | undefined {
  const calls = Object.values(liveToolCalls ?? {});
  // Generating a canvas's arguments can take minutes before tool_execution_start. Name that
  // streamed call immediately, rather than leaving the composer blank after thinking ends.
  const last = partial?.blocks[partial.blocks.length - 1];
  const latest = calls[calls.length - 1] ?? (last?.type === "tool-call" ? last : undefined);
  if (latest) {
    const summary = summarizeTool(latest);
    // A command or path is the detail on its own; other tools read as verb + subject, and a
    // subject-less call (plan, preview) reads as its verb alone.
    if (summary.kind === "bash") return { subject: summary.subject || summary.activeVerb, subjectMono: Boolean(summary.subject) };
    if (!summary.subject) return { subject: summary.activeVerb };
    return { verb: summary.activeVerb, subject: summary.subject, subjectMono: summary.kind === "read" || summary.kind === "write" || summary.kind === "edit" || summary.kind === "search" };
  }
  if (last?.type === "thinking" && last.durationMs === undefined) return { subject: "Thinking…" };
  if (compaction) return { subject: "Compacting context…" };
  if (activity === "mcp_connect_start") return { subject: "Starting MCP servers…" };
  if (activity?.startsWith("auto_retry") || activity?.startsWith("summarization_retry")) return { subject: "Retrying…" };
  return undefined;
}

/**
 * The floating status row above the Classic composers (classic and classicGlass only — App
 * decides): the live "Working for" timer, the todo pill and the changed-files pill, centred
 * pills hovering just over the card. Rendered inside `.composer-wrap` so its height rides
 * --composer-h and the glass variant's backdrop styling.
 */
export function ComposerPills({ run, liveToolCalls, partial, compaction, activity, todos, todoBusy, changes, changesOpen, onToggleChanges }: ComposerPillsProps) {
  const detail = useMemo(() => runDetail({ liveToolCalls, partial, compaction, activity }), [liveToolCalls, partial, compaction, activity]);
  const working = run !== undefined || detail !== undefined;
  const hasTodos = (todos ?? []).some((task) => task.status !== "deleted");
  const hasChanges = changes?.isGit === true && changes.files.length > 0;
  if (!working && !hasTodos && !hasChanges) return null;
  return (
    <div className="composer-pills">
      {working && <WorkingPill run={run} detail={detail} />}
      {hasTodos && <TodoPill tasks={todos} busy={todoBusy} />}
      {hasChanges && <ChangesPill changes={changes} open={changesOpen === true} onToggle={onToggleChanges} />}
    </div>
  );
}

/**
 * The Classic composer's changes pill: the same "N files +A −D" count the Changes view's head
 * carries, so the two never disagree (a file contributes every section it has — staged plus
 * working — like `changeEntries` lists them). Clicking toggles that view, like the header's
 * ⌘⇧C button; it hides entirely while the tree is clean or the workspace isn't a checkout.
 */
export function ChangesPill({ changes, open, onToggle }: { changes: GitChanges; open?: boolean; onToggle?: () => void }) {
  const files = changes.files.length;
  const totals = changeEntries(changes.files).reduce((sum, entry) => ({ add: sum.add + entry.section.additions, del: sum.del + entry.section.deletions }), { add: 0, del: 0 });
  const button = (
    <button
      type="button"
      className={`composer-pill composer-pill-files${open ? " open" : ""}`}
      aria-pressed={open === true}
      onClick={onToggle}
    >
      <Icon name="git" />
      <span className="changes-meta"><strong>{files}</strong> {files === 1 ? "file" : "files"} <em className="stat-add">+{totals.add}</em> <em className="stat-del">−{totals.del}</em></span>
    </button>
  );
  return onToggle ? <Tooltip label={<>Changes <kbd>⌘⇧C</kbd></>}>{button}</Tooltip> : button;
}
