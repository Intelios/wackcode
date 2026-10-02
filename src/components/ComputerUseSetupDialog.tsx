import { useEffect, useState } from "react";
import { ComputerUsePermissionSteps, useComputerUseStatus, type ComputerUseActions } from "./ComputerUseSection";

interface Props {
  actions: ComputerUseActions;
  agentName: string;
  /** Saves the setting on; the dialog closes once it resolves. */
  onEnable: () => Promise<void>;
  onCancel: () => void;
}

/**
 * Switching computer use on: what it does, the two macOS permissions with their live state,
 * and only then the switch. Either approval can need a fresh process, so reopening saves the
 * setting first.
 */
export function ComputerUseSetupDialog({ actions, agentName, onEnable, onCancel }: Props) {
  const { status } = useComputerUseStatus(actions.onStatus, 1000);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const ready = status?.accessibility && status.screenRecording;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onCancel(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);

  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError(undefined);
    try { await work(); }
    catch (reason) { setError(String(reason)); setBusy(false); }
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.button === 0 && event.target === event.currentTarget && onCancel()}>
      <div className="confirm-dialog computer-setup" role="dialog" aria-modal="true" aria-label="Set up computer use">
        <span className="eyebrow">Computer use</span>
        <h2>Let {agentName} use the apps you build</h2>
        <p>
          {agentName} can open, look at and operate native app windows to check its work — pressing buttons and typing in the background
          where it can, and briefly taking the pointer only when it must. The first time it wants an app, you're asked in the chat.
        </p>
        <ComputerUsePermissionSteps
          status={status}
          actions={actions}
          onError={setError}
          onRelaunch={() => void run(async () => { await onEnable(); await actions.onRelaunch(); })}
        />
        {status?.devBuild && (
          <p className="computer-note">Development build: macOS credits these permissions to whatever launched it, not to WackCode.</p>
        )}
        {status && !status.supported && <div className="error-banner" role="alert">Computer use needs macOS 14 or later.</div>}
        {error && <div className="error-banner" role="alert">{error}</div>}
        <div className="confirm-actions">
          <button type="button" className="secondary-button" onClick={onCancel}>Cancel</button>
          <button
            type="button"
            className="primary-button"
            disabled={busy || !status?.supported}
            onClick={() => void run(onEnable)}
          >
            {ready ? "Turn on" : "Turn on anyway"}
          </button>
        </div>
      </div>
    </div>
  );
}
