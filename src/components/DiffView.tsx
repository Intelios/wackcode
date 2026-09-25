import { Fragment } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ChangeEntry } from "../changes-utils";
import { hunkLabel, lineAnchor, sectionComments, splitPath } from "../changes-utils";
import type { DiffComment, GitDiffHunk, GitDiffLine } from "../types";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

export type CommentAnchor = Omit<DiffComment, "id" | "text">;

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface DiffViewProps {
  entry: ChangeEntry;
  comments: DiffComment[];
  /** Anchor of the line whose composer is open, if it belongs to this section. */
  commentAt?: CommentAnchor;
  commentText: string;
  editing: Record<string, string>;
  disabled: boolean;
  onAddComment: (line: GitDiffLine) => void;
  onCommentText: (text: string) => void;
  onCloseComposer: () => void;
  onSaveComment: () => void;
  /** `text` updates the draft; `null` drops it (Cancel). */
  onEditComment: (id: string, text: string | null) => void;
  onUpdateComment: (comment: DiffComment, text: string) => void;
  onRemoveComment: (id: string) => void;
  onAction: (entry: ChangeEntry, hunkId?: number) => void;
}

function ActionButtons({ entry, hunkId, disabled, onAction }: { entry: ChangeEntry; hunkId?: number; disabled: boolean; onAction: DiffViewProps["onAction"] }) {
  const { file, section } = entry;
  const partial = hunkId !== undefined;
  const allowed = !partial || (file.hunkable && !section.truncated);
  const locked = disabled || file.status === "conflict" || !allowed;
  const reason = file.status === "conflict" ? "Resolve this conflict first" : !allowed ? "Whole-file actions only" : undefined;
  const button = <button type="button" className="compact danger" disabled={locked} onClick={() => onAction(entry, hunkId)}>Discard{partial ? " hunk" : ""}</button>;
  return reason ? <Tooltip label={reason}><span className="change-action-wrap">{button}</span></Tooltip> : button;
}

function CommentCard({ comment, stale, draft, disabled, onEditComment, onUpdateComment, onRemoveComment }: {
  comment: DiffComment; stale: boolean; draft: string | undefined; disabled: boolean;
  onEditComment: (id: string, text: string | null) => void;
  onUpdateComment: (comment: DiffComment, text: string) => void;
  onRemoveComment: (id: string) => void;
}) {
  return (
    <div className={`diff-comment ${stale ? "stale" : ""}`}>
      <Icon name="comment" />
      <div className="diff-comment-body">
        <div className="diff-comment-meta">
          <span>{comment.path}:{comment.line} · {comment.side === "old" ? "removed line" : comment.layer}</span>
          {stale && <em>line moved</em>}
        </div>
        {draft !== undefined ? (
          <>
            <textarea
              aria-label={`Edit comment ${comment.path}:${comment.line}`}
              value={draft}
              onChange={(event) => onEditComment(comment.id, event.target.value)}
              onKeyDown={(event) => { if (event.key === "Escape") onEditComment(comment.id, null); }}
            />
            <div className="diff-comment-actions">
              <button type="button" className="compact" onClick={() => onEditComment(comment.id, null)}>Cancel</button>
              <button type="button" className="compact" disabled={disabled || !draft.trim()} onClick={() => onUpdateComment(comment, draft)}>Save</button>
            </div>
          </>
        ) : (
          <>
            <p>{comment.text}</p>
            <div className="diff-comment-actions">
              <button type="button" className="compact" onClick={() => onEditComment(comment.id, comment.text)}>Edit</button>
              <button type="button" className="compact danger" disabled={disabled} onClick={() => onRemoveComment(comment.id)}>Remove</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function CommentComposer({ anchor, text, disabled, onText, onClose, onSave }: {
  anchor: CommentAnchor; text: string; disabled: boolean;
  onText: (text: string) => void; onClose: () => void; onSave: () => void;
}) {
  return (
    <div className="diff-comment-composer">
      <textarea
        autoFocus
        aria-label="Diff comment"
        placeholder={`Comment on ${anchor.path}:${anchor.line}`}
        value={text}
        onChange={(event) => onText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) onSave();
        }}
      />
      <div className="diff-comment-actions">
        <span className="diff-hint">⌘↩ to comment</span>
        <button type="button" className="compact" onClick={onClose}>Cancel</button>
        <button type="button" className="compact primary" disabled={!text.trim() || disabled} onClick={onSave}>Comment</button>
      </div>
    </div>
  );
}

export function DiffView(props: DiffViewProps) {
  const { entry, disabled } = props;
  const { file, section } = entry;
  const reduce = useReducedMotion();
  const transition = reduce ? { duration: 0 } : { duration: 0.16, ease: EASE };
  const { dir, base } = splitPath(file.path);
  const scoped = sectionComments(file.path, section, props.comments);

  function commentsAfter(line: GitDiffLine) {
    const anchor = lineAnchor(line);
    if (!anchor) return null;
    const group = scoped.inline.get(`${anchor.side}:${anchor.line}`);
    const composing = props.commentAt !== undefined && props.commentAt.path === file.path && props.commentAt.layer === section.layer
      && props.commentAt.side === anchor.side && props.commentAt.line === anchor.line;
    if (!group && !composing) return null;
    return (
      <div className="diff-comment-slot">
        {group?.map((comment) => (
          <CommentCard key={comment.id} comment={comment} stale={false} draft={props.editing[comment.id]}
            disabled={disabled} onEditComment={props.onEditComment} onUpdateComment={props.onUpdateComment} onRemoveComment={props.onRemoveComment} />
        ))}
        {composing && props.commentAt && <CommentComposer anchor={props.commentAt} text={props.commentText} disabled={disabled}
          onText={props.onCommentText} onClose={props.onCloseComposer} onSave={props.onSaveComment} />}
      </div>
    );
  }

  return (
    <>
      <div className="diff-file-header">
        <Icon name="file" />
        <span className="diff-file-path" title={file.path}>
          {dir && <span className="change-dir">{dir}</span>}
          <span className="change-base">{base}</span>
        </span>
        {file.oldPath && <em className="diff-tag">from {file.oldPath}</em>}
        {file.binary && <em className="diff-tag">binary</em>}
        {section.truncated && <em className="diff-tag">truncated</em>}
        <ActionButtons entry={entry} disabled={disabled} onAction={props.onAction} />
      </div>
      <div className="diff-scroll" aria-label={"Diff for " + file.path}>
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={section.layer + ":" + file.path}
            className="diff-body"
            initial={reduce ? false : { opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={transition}
          >
            {file.binary ? (
              <div className="diff-note"><Icon name="file" /><span>Binary file — no text diff</span></div>
            ) : section.hunks.length === 0 ? (
              <div className="diff-note"><pre>{section.diff || "No text diff available"}</pre></div>
            ) : (
              <>
                {scoped.orphans.length > 0 && (
                  <div className="diff-orphans">
                    <span>Comments on lines that moved</span>
                    {scoped.orphans.map((comment) => (
                      <CommentCard key={comment.id} comment={comment} stale={true} draft={props.editing[comment.id]}
                        disabled={disabled} onEditComment={props.onEditComment} onUpdateComment={props.onUpdateComment} onRemoveComment={props.onRemoveComment} />
                    ))}
                  </div>
                )}
                {section.hunks.map((hunk: GitDiffHunk) => (
                  <div key={hunk.id} className="diff-hunk-block">
                    <div className="diff-hunk">
                      <span className="diff-hunk-label">{hunkLabel(hunk)}</span>
                      <code className="diff-hunk-raw">{hunk.header}</code>
                      <span className="diff-hunk-actions"><ActionButtons entry={entry} hunkId={hunk.id} disabled={disabled} onAction={props.onAction} /></span>
                    </div>
                    {hunk.lines.map((line, index) => (
                      <Fragment key={index}>
                        <div className={"diff-line " + line.kind}>
                          <span className="diff-gutter">
                            <button
                              type="button"
                              className="diff-comment-button"
                              aria-label={`Comment on ${file.path} line ${line.newLine ?? line.oldLine}`}
                              disabled={!lineAnchor(line)}
                              onClick={() => props.onAddComment(line)}
                            ><Icon name="plus" /></button>
                            <span className="diff-line-number">{line.oldLine ?? ""}</span>
                            <span className="diff-line-number">{line.newLine ?? ""}</span>
                          </span>
                          <code>{line.text || " "}</code>
                        </div>
                        {commentsAfter(line)}
                      </Fragment>
                    ))}
                  </div>
                ))}
              </>
            )}
          </motion.div>
        </AnimatePresence>
      </div>
    </>
  );
}
