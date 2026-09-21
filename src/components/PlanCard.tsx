import { Icon } from "./Icons";
import { Markdown } from "./Markdown";

export type PlanAction = "implement" | "copy" | "save" | "discard";

interface PlanCardProps {
  plan: string;
  /** This card's plan is the one currently awaiting a decision. */
  current: boolean;
  /** A run is in progress — actions are disabled. */
  busy?: boolean;
  onAction?: (action: PlanAction) => void;
}

/**
 * A proposed plan, rendered where plan_mode_complete's tool result lands in the transcript.
 * Only the plan that is *currently* awaiting review gets the action row — earlier or revised
 * proposals stay readable but inert, so there is always at most one decision on screen.
 */
export function PlanCard({ plan, current, busy, onAction }: PlanCardProps) {
  return (
    <div className={`plan-card ${current ? "current" : ""}`}>
      <div className="plan-card-head">
        <Icon name="brain" />
        <span>Proposed plan</span>
      </div>
      <div className="plan-card-body">
        <Markdown>{plan}</Markdown>
      </div>
      {current && onAction && (
        <>
          <div className="plan-card-actions">
            <button type="button" className="primary-button" disabled={busy} onClick={() => onAction("implement")}>
              Approve & implement
            </button>
            <button type="button" className="secondary-button" disabled={busy} onClick={() => onAction("save")}>
              Save PLAN.md
            </button>
            <button type="button" className="secondary-button" disabled={busy} onClick={() => onAction("copy")}>
              Copy
            </button>
            <button type="button" className="danger-button" disabled={busy} onClick={() => onAction("discard")}>
              Discard
            </button>
          </div>
          <p className="plan-card-hint">Type feedback below to revise the plan.</p>
        </>
      )}
    </div>
  );
}
