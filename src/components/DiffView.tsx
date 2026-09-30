import { Fragment, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { hunkLabel, lineAnchor, sectionComments, splitPath, splitRows, type SplitCell } from "../changes-utils";
import { highlightDiffLines, languageForPath } from "../highlight";
import type { DiffComment, DiffLayout, GitChangeFile, GitDiffHunk, GitDiffLine, GitDiffSection } from "../types";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

export type CommentAnchor = Omit<DiffComment, "id" | "text">;

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const SECTION_LABELS: Record<GitDiffSection["layer"], string> = { staged: "Staged", working: "Not staged", commit: "Commit" };

interface DiffViewProps {
  file: GitChangeFile;
  /** The Changes panel shows one section; Git mode stacks a file's staged and working ones. */
  sections: GitDiffSection[];
  layout?: DiffLayout;
  /** History: no comment gutter and no discard buttons. */
  readOnly?: boolean;
  /** Which way a file swap slides: 1 moving down a list, -1 moving up it. */
  direction?: 1 | -1;
  comments?: DiffComment[];
  /** Anchor of the line whose composer is open, if it belongs to this file. */
  commentAt?: CommentAnchor;
  commentText?: string;
  editing?: Record<string, string>;
  disabled: boolean;
  onAddComment?: (section: GitDiffSection, line: GitDiffLine) => void;
  onCommentText?: (text: string) => void;
  onCloseComposer?: () => void;
  onSaveComment?: () => void;
  /** `text` updates the draft; `null` drops it (Cancel). */
  onEditComment?: (id: string, text: string | null) => void;
  onUpdateComment?: (comment: DiffComment, text: string) => void;
  onRemoveComment?: (id: string) => void;
  onAction?: (file: GitChangeFile, section: GitDiffSection, hunkId?: number) => void;
  /** Extra header content after the path, such as Git mode's line counts. */
  headerExtra?: ReactNode;
}

function ActionButtons({ file, section, hunkId, disabled, onAction }: {
  file: GitChangeFile; section: GitDiffSection; hunkId?: number; disabled: boolean; onAction: NonNullable<DiffViewProps["onAction"]>;
}) {
  const partial = hunkId !== undefined;
  const allowed = !partial || (file.hunkable && !section.truncated);
  const locked = disabled || file.status === "conflict" || !allowed;
  const reason = file.status === "conflict" ? "Resolve this conflict first" : !allowed ? "Whole-file actions only" : undefined;
  const button = <button type="button" className="compact danger" disabled={locked} onClick={() => onAction(file, section, hunkId)}>Discard{partial ? " hunk" : ""}</button>;
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

const noop = () => {};

/** One section's hunks, unified or split, with its inline comments. */
function SectionHunks(props: DiffViewProps & { section: GitDiffSection; commenting: boolean; discarding: boolean }) {
  const { file, section, disabled, commenting, discarding } = props;
  const split = props.layout === "split";
  const scoped = sectionComments(file.path, section, props.comments ?? []);
  const lang = languageForPath(file.path);
  const edit = props.onEditComment ?? noop;
  const update = props.onUpdateComment ?? noop;
  const remove = props.onRemoveComment ?? noop;

  function commentsAfter(line: GitDiffLine) {
    const anchor = lineAnchor(line);
    if (!anchor) return null;
    const group = scoped.inline.get(`${anchor.side}:${anchor.line}`);
    const at = props.commentAt;
    const composing = at !== undefined && at.path === file.path && at.layer === section.layer && at.side === anchor.side && at.line === anchor.line;
    if (!group && !composing) return null;
    return (
      <div className="diff-comment-slot">
        {group?.map((comment) => (
          <CommentCard key={comment.id} comment={comment} stale={false} draft={props.editing?.[comment.id]}
            disabled={disabled} onEditComment={edit} onUpdateComment={update} onRemoveComment={remove} />
        ))}
        {composing && at && <CommentComposer anchor={at} text={props.commentText ?? ""} disabled={disabled}
          onText={props.onCommentText ?? noop} onClose={props.onCloseComposer ?? noop} onSave={props.onSaveComment ?? noop} />}
      </div>
    );
  }

  function gutterButton(line: GitDiffLine) {
    if (!commenting) return null;
    return (
      <button
        type="button"
        className="diff-comment-button"
        aria-label={`Comment on ${file.path} line ${line.newLine ?? line.oldLine}`}
        disabled={!lineAnchor(line)}
        onClick={() => props.onAddComment?.(section, line)}
      ><Icon name="plus" /></button>
    );
  }

  function splitCell(cell: SplitCell | null, side: "old" | "new", nodes: ReactNode[]) {
    if (!cell) return <div className="diff-split-cell empty" aria-hidden="true" />;
    const { line } = cell;
    // Context shows on both sides; its comment button lives on the new side only, where its
    // anchor points, so a comment can't be started twice for one line.
    const canComment = side === "old" ? line.kind === "deletion" : line.kind !== "deletion";
    return (
      <div className={`diff-split-cell ${line.kind}`}>
        <span className="diff-gutter">
          {canComment ? gutterButton(line) : commenting ? <span className="diff-comment-spacer" /> : null}
          <span className="diff-line-number">{(side === "old" ? line.oldLine : line.newLine) ?? ""}</span>
        </span>
        <code>{nodes[cell.index]}</code>
      </div>
    );
  }

  return (
    <>
      {scoped.orphans.length > 0 && (
        <div className="diff-orphans">
          <span>Comments on lines that moved</span>
          {scoped.orphans.map((comment) => (
            <CommentCard key={comment.id} comment={comment} stale={true} draft={props.editing?.[comment.id]}
              disabled={disabled} onEditComment={edit} onUpdateComment={update} onRemoveComment={remove} />
          ))}
        </div>
      )}
      {section.hunks.map((hunk: GitDiffHunk) => {
        // Syntax-highlight the hunk's code lines with the file's own language; in unified view
        // the diff marker stays a plain leading character so it keeps the line's add/del ink.
        const texts = hunk.lines.map((line) => line.text);
        const kinds = hunk.lines.map((line) => (line.kind === "meta" ? "meta" : "code"));
        const lineNodes = highlightDiffLines(texts, kinds, lang, { marker: !split });
        return (
          <div key={hunk.id} className={`diff-hunk-block${split ? " split" : ""}`}>
            <div className="diff-hunk">
              <span className="diff-hunk-label">{hunkLabel(hunk)}</span>
              <code className="diff-hunk-raw">{hunk.header}</code>
              {discarding && props.onAction && (
                <span className="diff-hunk-actions"><ActionButtons file={file} section={section} hunkId={hunk.id} disabled={disabled} onAction={props.onAction} /></span>
              )}
            </div>
            {split ? splitRows(hunk).map((row, index) => row.kind === "meta" ? (
              <div key={index} className="diff-split-meta"><code>{row.cell.line.text}</code></div>
            ) : (
              <Fragment key={index}>
                <div className="diff-split-row">
                  {splitCell(row.left, "old", lineNodes)}
                  {splitCell(row.right, "new", lineNodes)}
                </div>
                {row.left && row.left.line.kind === "deletion" && commentsAfter(row.left.line)}
                {row.right && commentsAfter(row.right.line)}
              </Fragment>
            )) : hunk.lines.map((line, index) => (
              <Fragment key={index}>
                <div className={"diff-line " + line.kind}>
                  <span className="diff-gutter">
                    {gutterButton(line)}
                    <span className="diff-line-number">{line.oldLine ?? ""}</span>
                    <span className="diff-line-number">{line.newLine ?? ""}</span>
                  </span>
                  <code>{lineNodes[index]}</code>
                </div>
                {commentsAfter(line)}
              </Fragment>
            ))}
          </div>
        );
      })}
    </>
  );
}

export function DiffView(props: DiffViewProps) {
  const { file, sections, disabled } = props;
  const reduce = useReducedMotion();
  const direction = props.direction ?? 1;
  const transition = reduce ? { duration: 0 } : { duration: 0.16, ease: EASE };
  const { dir, base } = splitPath(file.path);
  const commenting = !props.readOnly && Boolean(props.onAddComment);
  const discarding = !props.readOnly && Boolean(props.onAction);
  const stacked = sections.length > 1;
  const truncated = sections.some((section) => section.truncated);
  const bodyKey = `${file.path}:${sections.map((section) => section.layer).join("+")}:${props.layout ?? "unified"}`;

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
        {truncated && <em className="diff-tag">truncated</em>}
        {props.headerExtra}
        {discarding && !stacked && sections[0] && props.onAction && (
          <ActionButtons file={file} section={sections[0]} disabled={disabled} onAction={props.onAction} />
        )}
      </div>
      <div className={`diff-scroll${props.layout === "split" ? " split" : ""}`} aria-label={"Diff for " + file.path}>
        <AnimatePresence mode="wait" initial={false} custom={direction}>
          <motion.div
            key={bodyKey}
            className="diff-body"
            custom={direction}
            initial={reduce ? false : { opacity: 0, y: 6 * direction }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 * direction }}
            transition={transition}
          >
            {file.binary ? (
              <div className="diff-note"><Icon name="file" /><span>Binary file — no text diff</span></div>
            ) : sections.every((section) => section.hunks.length === 0) ? (
              <div className="diff-note"><pre>{sections.map((section) => section.diff).join("\n") || "No text diff available"}</pre></div>
            ) : sections.map((section) => (
              <Fragment key={section.layer}>
                {stacked && (
                  <div className="diff-section-label">
                    <span>{SECTION_LABELS[section.layer]}</span>
                    {discarding && props.onAction && <ActionButtons file={file} section={section} disabled={disabled} onAction={props.onAction} />}
                  </div>
                )}
                <SectionHunks {...props} section={section} commenting={commenting} discarding={discarding} />
              </Fragment>
            ))}
          </motion.div>
        </AnimatePresence>
      </div>
    </>
  );
}
