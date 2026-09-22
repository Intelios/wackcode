import { useEffect, useState } from "react";

interface Props {
  source: string;
  /** "install" fetches and enables; "enable" only grants trust to something already on disk. */
  mode: "install" | "enable";
  busy?: boolean;
  error?: string;
  onCancel: () => void;
  onConfirm: () => void;
}

/**
 * Shown before the first install of a package. Installing grants arbitrary local code execution
 * inside the process that can read provider credentials, so the consequences are stated plainly and
 * the confirm button stays locked until the user acknowledges them.
 */
export function TrustDialog({ source, mode, busy, error, onCancel, onConfirm }: Props) {
  const [acknowledged, setAcknowledged] = useState(false);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onCancel();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [busy, onCancel]);

  return (
    <div className="modal-backdrop" onPointerDown={() => { if (!busy) onCancel(); }}>
      <div
        className="confirm-dialog trust-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="trust-dialog-title"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <h3 id="trust-dialog-title">{mode === "install" ? "Install this package?" : "Enable this package?"}</h3>
        <p className="trust-source">{source}</p>
        <div className="trust-warning">
          Pi packages run as ordinary local code with your macOS account&rsquo;s permissions. This package
          will be able to read and write any file you can, run any command, make network requests, and
          read WackCode&rsquo;s API keys and subscription credentials. Install it only if you trust its author.
        </div>
        <label className="trust-ack">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(event) => setAcknowledged(event.target.checked)}
          />
          <span>I understand and trust this package</span>
        </label>
        {error && <div className="error-banner">{error}</div>}
        <div className="confirm-actions">
          <button type="button" className="secondary-button" onClick={onCancel} disabled={busy}>Cancel</button>
          <button type="button" className="danger-button" onClick={onConfirm} disabled={busy || !acknowledged}>
            {busy ? "Working…" : mode === "install" ? "Trust and install" : "Trust and enable"}
          </button>
        </div>
      </div>
    </div>
  );
}
