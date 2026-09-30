import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { commitLabel } from "../git-mode";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

interface GitCommitFormProps {
  branch: string | null;
  summary: string;
  description: string;
  /** Set while the text came from Generate and the changes have moved since. */
  stale: boolean;
  checkedCount: number;
  /** Why committing is blocked right now (a chat is running, a conflict), if it is. */
  blockedReason?: string;
  generate: { available: boolean; reason?: string; running: boolean };
  agentName: string;
  committing: boolean;
  /** Bumps after each commit: the button briefly reads "Committed". */
  commitPulse: number;
  onSummary: (value: string) => void;
  onDescription: (value: string) => void;
  onGenerate: () => void;
  onCommit: () => void;
}

/** A summary this long reads fine in `git log --oneline`; past the hard limit it gets cut. */
const SOFT_LIMIT = 50;
const HARD_LIMIT = 72;

/** Git mode's commit box: Summary + Description + "Commit 3 files to main", as in GitHub Desktop. */
export function GitCommitForm(props: GitCommitFormProps) {
  const reduce = useReducedMotion();
  const summaryRef = useRef<HTMLTextAreaElement>(null);
  const descriptionRef = useRef<HTMLTextAreaElement>(null);
  const [done, setDone] = useState(false);
  const length = props.summary.length;
  const canCommit = !props.blockedReason && !props.committing && props.checkedCount > 0 && props.summary.trim() !== "";
  const label = props.committing ? "Committing…" : done ? "Committed ✓" : commitLabel(props.checkedCount, props.branch);

  useEffect(() => {
    if (props.commitPulse === 0) return;
    setDone(true);
    const timer = setTimeout(() => setDone(false), 1600);
    return () => clearTimeout(timer);
  }, [props.commitPulse]);

  // Both fields grow with their text up to a cap, then scroll. The sidebar is narrow, so the
  // summary wraps to stay readable; it is still one line of the commit message.
  useLayoutEffect(() => {
    for (const [element, cap] of [[summaryRef.current, 84], [descriptionRef.current, 180]] as const) {
      if (!element) continue;
      element.style.height = "auto";
      element.style.height = `${Math.min(element.scrollHeight + 2, cap)}px`;
    }
  }, [props.summary, props.description]);

  function submitOnShortcut(event: React.KeyboardEvent) {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      if (canCommit) props.onCommit();
    }
  }

  const button = (
    <button type="button" className={`primary-button git-commit-button${done ? " done" : ""}`} disabled={!canCommit} onClick={props.onCommit}>
      <Icon name="commit" />
      <span className="git-commit-label">
        <AnimatePresence initial={false} mode="popLayout">
          <motion.span
            key={label}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: -12 }}
            transition={{ type: "spring", stiffness: 520, damping: 34 }}
          >{label}</motion.span>
        </AnimatePresence>
      </span>
    </button>
  );

  return (
    <div className="git-commit-form" onKeyDown={submitOnShortcut}>
      <div className="git-summary-row">
        <textarea
          ref={summaryRef}
          className="git-summary"
          aria-label="Commit summary"
          placeholder="Summary (required)"
          value={props.summary}
          rows={1}
          maxLength={200}
          onChange={(event) => props.onSummary(event.target.value.replace(/\s*\n\s*/g, " "))}
          onKeyDown={(event) => {
            // Return finishes the summary line; ⌘↩ (handled on the form) commits.
            if (event.key === "Enter" && !event.metaKey && !event.ctrlKey) {
              event.preventDefault();
              descriptionRef.current?.focus();
            }
          }}
        />
        {length > SOFT_LIMIT && (
          <span className={`git-summary-count ${length > HARD_LIMIT ? "over" : ""}`} title={length > HARD_LIMIT ? "Long summaries get cut off in most Git tools" : "Aim for 50 characters or fewer"}>
            {HARD_LIMIT - length}
          </span>
        )}
      </div>
      <textarea
        ref={descriptionRef}
        aria-label="Commit description"
        placeholder="Description"
        value={props.description}
        rows={2}
        onChange={(event) => props.onDescription(event.target.value)}
      />
      {props.stale && <small className="git-form-note">Your changes moved since this message was generated.</small>}
      <div className="git-form-row">
        <Tooltip label={props.generate.reason ?? `Write the message with ${props.agentName}`}>
          <span className="change-action-wrap">
            <button
              type="button"
              className="ghost-button git-generate"
              disabled={!props.generate.available || props.generate.running || props.checkedCount === 0}
              aria-label={`Generate commit message with ${props.agentName}`}
              onClick={props.onGenerate}
            >
              <Icon name="spark" className={props.generate.running ? "spinning" : ""} />
              <span>{props.generate.running ? "Writing…" : "Generate"}</span>
            </button>
          </span>
        </Tooltip>
        <span className="git-form-hint"><kbd>⌘↩</kbd></span>
      </div>
      {props.blockedReason ? <Tooltip label={props.blockedReason}><span className="git-commit-wrap">{button}</span></Tooltip> : button}
    </div>
  );
}
