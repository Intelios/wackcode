import { useEffect, useState } from "react";

interface ConfirmDialogProps {
  title: string;
  body?: string;
  confirmLabel?: string;
  danger?: boolean;
  onConfirm: () => Promise<void> | void;
  onCancel: () => void;
}

export function ConfirmDialog({ title, body, confirmLabel = "Confirm", danger, onConfirm, onCancel }: ConfirmDialogProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onCancel(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onCancel]);

  async function confirm() {
    setBusy(true);
    setError(undefined);
    try {
      await onConfirm();
      onCancel();
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onCancel()}>
      <div className="confirm-dialog" role="alertdialog" aria-modal="true" aria-label={title}>
        <h2>{title}</h2>
        {body && <p>{body}</p>}
        {error && <div className="error-banner">{error}</div>}
        <div className="confirm-actions">
          <button type="button" className="secondary-button" onClick={onCancel} autoFocus>Cancel</button>
          <button type="button" className={danger ? "danger-button" : "primary-button"} disabled={busy} onClick={() => void confirm()}>
            {busy ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
