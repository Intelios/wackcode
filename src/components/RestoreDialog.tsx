import { useEffect, useRef, useState } from "react";
import type { CheckpointChange } from "../types";
import { changeLabel } from "../tree-utils";

export interface RestoreChoice {
  id: string;
  label: string;
  /** Restores the selected files, so it needs at least one selected. */
  files?: boolean;
  danger?: boolean;
}

interface Props {
  title: string;
  body?: string;
  changes: CheckpointChange[];
  initialSelection: string[];
  /** Other chats working in the same folder, whose files a restore affects too. */
  sharedWith?: string;
  choices: RestoreChoice[];
  onChoose: (choice: string, paths: string[]) => Promise<void>;
  onCancel: () => void;
}

/**
 * Asks what to do with files that changed since a point in the conversation, listing them so
 * the user can keep some. Used before retry and edit, for rewinds, and after switching versions.
 */
export function RestoreDialog({ title, body, changes, initialSelection, sharedWith, choices, onChoose, onCancel }: Props) {
  const [selected, setSelected] = useState(() => new Set(initialSelection));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const dialogRef = useRef<HTMLDivElement>(null);
  const allSelected = changes.length > 0 && changes.every((change) => selected.has(change.path));

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape" && !busy) onCancel(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [busy, onCancel]);

  useEffect(() => {
    dialogRef.current?.querySelector<HTMLButtonElement>(".confirm-actions button")?.focus();
  }, []);

  function toggle(path: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  }

  async function choose(choice: RestoreChoice) {
    setBusy(true);
    setError(undefined);
    try {
      await onChoose(choice.id, changes.filter((change) => selected.has(change.path)).map((change) => change.path));
      onCancel();
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(event) => event.button === 0 && event.target === event.currentTarget && !busy && onCancel()}>
      <div ref={dialogRef} className="confirm-dialog restore-dialog" role="dialog" aria-modal="true" aria-label={title}>
        <h2>{title}</h2>
        {body && <p>{body}</p>}
        {sharedWith && <p className="restore-shared">{sharedWith} also works in this folder. Restoring changes its files too.</p>}
        {changes.length > 0 && (
          <div className="restore-files">
            <label className="restore-file all">
              <input
                type="checkbox"
                checked={allSelected}
                onChange={() => setSelected(allSelected ? new Set() : new Set(changes.map((change) => change.path)))}
              />
              <span>{changes.length === 1 ? "1 changed file" : `${changes.length} changed files`}</span>
            </label>
            <ul aria-label="Changed files">
              {changes.map((change) => (
                <li key={change.path}>
                  <label className="restore-file">
                    <input type="checkbox" checked={selected.has(change.path)} onChange={() => toggle(change.path)} />
                    <span className="restore-path" title={change.path}>{change.path}</span>
                    <em className={`restore-status ${change.status}`}>{changeLabel(change.status)}</em>
                  </label>
                </li>
              ))}
            </ul>
          </div>
        )}
        {error && <div className="error-banner">{error}</div>}
        <div className="confirm-actions">
          <button type="button" className="secondary-button" onClick={onCancel} disabled={busy}>Cancel</button>
          {choices.map((choice) => (
            <button
              key={choice.id}
              type="button"
              className={choice.danger ? "danger-button" : choice.files ? "primary-button" : "secondary-button"}
              disabled={busy || (choice.files === true && selected.size === 0)}
              onClick={() => void choose(choice)}
            >
              {choice.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
