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
  /** The plan came out of Ultra Plan: the card wears its warm palette and flame. */
  ultra?: boolean;
}

/**
 * A proposed plan, rendered where plan_mode_complete's tool result lands in the transcript.
 * Only the plan that is *currently* awaiting review gets the action row — earlier or revised
 * proposals stay readable but inert, so there is always at most one decision on screen.
 *
 * It is the transcript's one hero card: a lit head with a large mark, the plan's numbered steps
 * drawn as marks of their own (styles.css), and, while a decision is owed, a halo, a breathing
 * ring on the mark and a status pill. An earlier proposal keeps the shape and drops the light.
 */
export function PlanCard({ plan, current, busy, onAction, ultra = false }: PlanCardProps) {
  /** The action handler, only while this card is the one a decision is owed on. */
  const decide = current ? onAction : undefined;
  return (
    <div className={`plan-card${current ? " current" : ""}${ultra ? " ultra" : ""}`}>
      <div className="plan-card-head">
        <span className="plan-card-mark" aria-hidden="true"><Icon name={ultra ? "flame" : "brain"} /></span>
        <div className="plan-card-title">
          <strong>Proposed plan</strong>
          {decide && <span>Approve it, or type feedback below to revise it.</span>}
        </div>
        {decide && <span className="plan-card-status"><i aria-hidden="true" />Awaiting you</span>}
      </div>
      <div className="plan-card-body">
        <Markdown>{plan}</Markdown>
      </div>
      {decide && (
        <div className="plan-card-actions">
          <button type="button" className="primary-button" disabled={busy} onClick={() => decide("implement")}>
            Approve & implement
          </button>
          <button type="button" className="secondary-button" disabled={busy} onClick={() => decide("save")}>
            Save PLAN.md
          </button>
          <button type="button" className="secondary-button" disabled={busy} onClick={() => decide("copy")}>
            Copy
          </button>
          <button type="button" className="danger-button" disabled={busy} onClick={() => decide("discard")}>
            Discard
          </button>
        </div>
      )}
    </div>
  );
}
