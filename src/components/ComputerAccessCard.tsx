import type { ComputerAccessDecision, ComputerAccessRequest } from "../types";
import { Icon } from "./Icons";

interface Props {
  request: ComputerAccessRequest;
  agentName: string;
  onDecide: (decision: ComputerAccessDecision) => void;
}

/**
 * Computer use's per-app access card: shown the first time a chat's agent wants to see and
 * operate an app. The host raised it and holds the grant; the answer goes straight back to the
 * host. Deliberately no default focus, so a stray Enter can't allow anything; Escape denies
 * (InlineDialog).
 */
export function ComputerAccessCard({ request, agentName, onDecide }: Props) {
  const { app } = request;
  return (
    <div className="inline-dialog-card computer-access-card" role="region" aria-label={`Allow ${agentName} to use ${app.name}?`}>
      <div className="computer-access-head">
        <span className="computer-access-icon" aria-hidden="true">
          {app.icon ? <img src={app.icon} alt="" /> : <Icon name="cursor" />}
        </span>
        <div className="computer-access-title">
          <span className="eyebrow">Computer use</span>
          <h3>Let {agentName} use {app.name}?</h3>
          {(app.bundleId || app.path) && (
            <code className="computer-access-id" title={app.path ?? undefined}>{app.bundleId ?? app.path}</code>
          )}
        </div>
      </div>
      <p className="computer-access-body">
        {request.launch ? `${app.name} isn't open yet; allowing also launches it in the background. ` : ""}
        {agentName} will be able to see {app.name}'s windows and press, type and choose menus in it for the rest of this chat, until WackCode quits.
        Press <kbd>⌃⌥⌘.</kbd> to stop at any time.
      </p>
      <div className="confirm-actions computer-access-actions">
        {app.bundleId && (
          <button type="button" className="secondary-button computer-access-never" onClick={() => onDecide("never")}>Never allow</button>
        )}
        <button type="button" className="secondary-button" onClick={() => onDecide("deny")}>Deny</button>
        <button type="button" className="primary-button" onClick={() => onDecide("allow")}>Allow for this chat</button>
      </div>
    </div>
  );
}
