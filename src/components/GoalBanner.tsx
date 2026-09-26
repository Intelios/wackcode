import type { GoalState } from "../types";

interface Props {
  /** The worker's latest goal state for this chat; undefined renders nothing. */
  goal?: GoalState;
  /** Dispatches `/goal pause|resume|clear`; wiring lives in App. */
  onAction: (action: "pause" | "resume" | "clear") => void;
}

const LABEL: Record<GoalState["phase"], string> = {
  active: "Goal",
  verifying: "Verifying",
  paused: "Goal paused",
  complete: "Goal complete",
  stopped: "Goal stopped"
};

/**
 * The goal loop's status line, pinned above the composer next to the todo panel. Renders the
 * state the worker's built-in goal extension publishes over `goal_state`/snapshots: while the
 * loop runs it shows the current round and the verifier's next action; when it pauses, completes
 * or stops it shows why and offers the matching control.
 */
export function GoalBanner({ goal, onAction }: Props) {
  if (!goal) return null;
  const running = goal.phase === "active" || goal.phase === "verifying";
  const detail = running
    ? goal.lastNextAction ?? goal.objective
    : goal.note ?? goal.lastReason ?? goal.objective;
  return (
    <div className="goal-banner-wrap">
      <section className={`goal-banner ${goal.phase}`} aria-label="Goal" aria-live="polite">
        <span className="goal-dot" aria-hidden="true" />
        <span className="goal-banner-title">{LABEL[goal.phase]}</span>
        <span className="goal-banner-round">{`${goal.iteration}/${goal.maxIterations}`}</span>
        <span className="goal-banner-detail" title={detail}>{detail}</span>
        <div className="goal-banner-actions">
          {running && (
            <button type="button" className="goal-banner-button" onClick={() => onAction("pause")}>Pause</button>
          )}
          {goal.phase === "paused" && (
            <button type="button" className="goal-banner-button" onClick={() => onAction("resume")}>Resume</button>
          )}
          <button type="button" className="goal-banner-button" onClick={() => onAction("clear")}>
            {running || goal.phase === "paused" ? "Clear" : "Dismiss"}
          </button>
        </div>
      </section>
    </div>
  );
}
