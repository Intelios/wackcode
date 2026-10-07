import { useEffect, useState } from "react";
import type { SkillDraftStatus, SkillPreviewDetails } from "../types";
import { Icon } from "./Icons";
import { Markdown } from "./Markdown";

/** What the review card asks the app to do. Saving is a native action, never an agent tool. */
export type SkillDraftAction =
  | { type: "save"; draftId: string; revision: string; name: string }
  | { type: "reveal"; draftRoot: string };

interface SkillDraftCardProps {
  details: SkillPreviewDetails;
  /** The host's publication state for this draft, once hydrated. */
  status?: SkillDraftStatus;
  /** This card is the newest preview on the branch; only it can be actionable. */
  current: boolean;
  /** This chat owns the draft — a forked chat can read the card but not save from it. */
  owned: boolean;
  /** A run is in progress — actions are disabled. */
  busy?: boolean;
  /** A save for this draft is in flight. */
  saving?: boolean;
  onAction?: (action: SkillDraftAction) => void;
  /** The draft's full SKILL.md body, on demand. */
  loadDocument?: (draftId: string) => Promise<string | undefined>;
}

function targetLabel(details: SkillPreviewDetails): string {
  switch (details.target) {
    case "library-update":
      return details.originLabel ? `Updates the ${details.originLabel} copy` : "Updates Your skills";
    case "library-copy":
      return details.originLabel ? `Copies from ${details.originLabel} into Your skills` : "Copies into Your skills";
    default:
      return "New skill in Your skills";
  }
}

function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * A skill draft awaiting review, rendered where the `skill_creator` preview's tool result
 * lands. Only the newest preview on the branch is actionable, and only in the chat that owns
 * the draft; publishing is the user's Save, never the agent's to do.
 */
export function SkillDraftCard({ details, status, current, owned, busy, saving, onAction, loadDocument }: SkillDraftCardProps) {
  const [expanded, setExpanded] = useState(false);
  const [fullBody, setFullBody] = useState<string>();
  const [loadingBody, setLoadingBody] = useState(false);

  // A new revision is a new document; never show one preview's body under another's card.
  useEffect(() => {
    setFullBody(undefined);
    setExpanded(false);
  }, [details.draftId, details.revision]);

  useEffect(() => {
    if (!expanded || fullBody !== undefined || !loadDocument) return;
    let active = true;
    setLoadingBody(true);
    void loadDocument(details.draftId)
      .then((body) => { if (active) setFullBody(body ?? ""); })
      .catch(() => { if (active) setFullBody(""); })
      .finally(() => { if (active) setLoadingBody(false); });
    return () => { active = false; };
  }, [expanded, fullBody, details.draftId, loadDocument]);

  const saved = current && status?.state === "saved";
  const actionable = current && owned && !busy && !saved;
  const revisionCurrent = status === undefined || status.revision === undefined || status.revision === details.revision;
  const canSave = actionable && status?.state === "ready" && revisionCurrent;
  const stale = actionable && ((status?.state === "stale") || (status?.state === "ready" && !revisionCurrent));
  const saveLabel = details.target === "library-update" ? "Update skill" : "Save skill";

  return (
    <div className={`skill-draft-card ${current ? "current" : ""} ${saved ? "saved" : ""}`}>
      <div className="skill-draft-head">
        <Icon name="book" />
        <span>Skill draft</span>
        <strong className="skill-draft-name">{details.name}</strong>
        {details.manual && <span className="skill-draft-flag" title="Only loads when you type /skill:name">manual</span>}
      </div>
      <p className="skill-draft-description">{details.description}</p>
      <div className="skill-draft-meta">
        <span>{targetLabel(details)}</span>
        <span aria-hidden="true">·</span>
        <span>{details.fileCount} {details.fileCount === 1 ? "file" : "files"}</span>
        <span aria-hidden="true">·</span>
        <span>{sizeLabel(details.totalBytes)}</span>
      </div>
      {details.warnings.length > 0 && (
        <ul className="skill-draft-warnings">
          {details.warnings.map((warning, index) => <li key={index}>{warning}</li>)}
        </ul>
      )}
      <div className="skill-draft-body">
        <Markdown>{details.bodyPreview}</Markdown>
        {(details.bodyTruncated || loadDocument) && (
          <button type="button" className="text-button" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}>
            {expanded ? "Hide full instructions" : "View full instructions"}
          </button>
        )}
        {expanded && (
          <div className="skill-draft-full" role="region" aria-label="Full skill instructions">
            {loadingBody ? <p className="skill-draft-note">Loading…</p> : <Markdown>{fullBody ?? details.bodyPreview}</Markdown>}
          </div>
        )}
        {details.files.length > 0 && (
          <details className="skill-draft-files">
            <summary>{details.files.length} supporting {details.files.length === 1 ? "file" : "files"}</summary>
            <ul>{details.files.map((file) => <li key={file}>{file}</li>)}</ul>
          </details>
        )}
      </div>
      {current && (
        <>
          {saved ? (
            <div className="skill-draft-saved">
              <span className="skill-draft-saved-pill"><Icon name="check" /> Saved</span>
              {status?.path && <code>{status.path}</code>}
              <p className="skill-draft-note">Available from your next message. Feedback below still revises the draft.</p>
            </div>
          ) : !owned ? (
            <p className="skill-draft-note">This draft belongs to the chat that created it. Run /skill-creator here to work on a copy.</p>
          ) : stale ? (
            <p className="skill-draft-note">The draft changed since this review. Send feedback to get a new preview, then save.</p>
          ) : status?.state === "unknown" ? (
            <p className="skill-draft-note">The last save was interrupted. Ask for a new preview, then save again.</p>
          ) : (
            <div className="skill-draft-actions">
              <button
                type="button"
                className="primary-button"
                disabled={!canSave || saving}
                onClick={() => onAction?.({ type: "save", draftId: details.draftId, revision: details.revision, name: details.name })}
              >
                {saving ? "Saving…" : saveLabel}
              </button>
              {status?.draftRoot && (
                <button type="button" className="secondary-button" disabled={busy} onClick={() => onAction?.({ type: "reveal", draftRoot: status.draftRoot! })}>
                  Open draft folder
                </button>
              )}
            </div>
          )}
          {!saved && owned && status?.state !== "unknown" && !stale && (
            <p className="skill-draft-note">Nothing is installed until you save. Feedback below revises the draft.</p>
          )}
        </>
      )}
      {!current && <p className="skill-draft-note superseded">Replaced by a newer preview.</p>}
    </div>
  );
}
