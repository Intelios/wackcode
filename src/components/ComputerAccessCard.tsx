import type { ComputerAccessDecision, ComputerAccessRequest } from "../types";
import { Icon } from "./Icons";

interface Props {
  request: ComputerAccessRequest;
  agentName: string;
  busy?: boolean;
  onDecide: (decision: ComputerAccessDecision) => void;
}

/**
 * Computer use's per-app access card: shown the first time a chat's agent wants to see and
 * operate an app. The host raised it and holds the grant; the answer goes straight back to the
 * host. Deliberately no default focus, so a stray Enter can't allow anything; Escape denies
 * (InlineDialog).
 */
export function ComputerAccessCard({ request, agentName, busy = false, onDecide }: Props) {
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
        {agentName} will be able to see {app.name}'s windows and press, type and choose menus in it.
        Press <kbd>⌃⌥⌘.</kbd> to stop at any time.
      </p>
      <p className="computer-access-note">
        Allow for this chat lasts until WackCode quits.
        {request.canAlwaysAllow && " Always allow remembers this app at this location for all coding chats, even after restarting. Revoke it in Settings › Computer use."}
      </p>
      <div className="confirm-actions computer-access-actions" aria-busy={busy}>
        {app.bundleId && (
          <button type="button" className="secondary-button computer-access-never" disabled={busy} onClick={() => onDecide("never")}>Never allow</button>
        )}
        <button type="button" className="secondary-button" disabled={busy} onClick={() => onDecide("deny")}>Deny</button>
        {request.canAlwaysAllow && (
          <button type="button" className="secondary-button" disabled={busy} onClick={() => onDecide("always")}>Always allow</button>
        )}
        <button type="button" className="primary-button" disabled={busy} onClick={() => onDecide("allow")}>Allow for this chat</button>
      </div>
    </div>
  );
}
