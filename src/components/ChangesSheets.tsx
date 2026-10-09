import type { DiffComment, GitChangeFile, GitPrInfo, TaskMode } from "../types";
import { isFresh } from "../changes-utils";
import { Icon } from "./Icons";

/**
 * The Changes panel's pull request form and pending-comments list. Both are presentational: the caller owns the fields and wraps async work in `run`
 * (busy flag plus error capture).
 */

type Run = (work: () => Promise<unknown>) => Promise<void>;

interface PrFormProps {
  pr?: GitPrInfo;
  base: string;
  title: string;
  body: string;
  draft: boolean;
  /** The title and body are being drafted from the branch's commits. */
  preparing: boolean;
  disabled: boolean;
  remote: string;
  onFields: (patch: Partial<{ base: string; title: string; body: string; draft: boolean }>) => void;
  onRetryPrepare: () => void;
  onCreate: (remote: string, base: string, title: string, body: string, draft: boolean) => Promise<string>;
  onOpenPr: (url: string) => void;
  run: Run;
}

export function PrForm(props: PrFormProps) {
  const { pr } = props;
  if (props.preparing) {
    return <div className="dock-form"><div className="dock-note dock-preparing"><Icon name="spark" className="spinning" /> Drafting title and description…</div></div>;
  }
  if (!pr) {
    return (
      <div className="dock-form">
        <small className="dock-note">Title and description are drafted from the branch's commits.</small>
        <div className="dock-actions">
          <span className="dock-spacer" />
          <button type="button" className="secondary-button compact" disabled={props.disabled || !props.remote} onClick={props.onRetryPrepare}>Prepare pull request</button>
        </div>
      </div>
    );
  }
  if (pr.existingUrl) {
    return (
      <div className="dock-form">
        <div className="dock-row"><Icon name="pullRequest" /><span className="dock-note">A pull request already exists for this branch.</span></div>
        <div className="dock-actions">
          <span className="dock-spacer" />
          <button type="button" className="secondary-button compact" onClick={() => props.onOpenPr(pr.existingUrl!)}><Icon name="external" /> Open pull request</button>
        </div>
      </div>
    );
  }
  return (
    <div className="dock-form">
      <div className="dock-note">{pr.repo} · {pr.head} → <input aria-label="PR base branch" className="dock-inline" value={props.base} onChange={(event) => props.onFields({ base: event.target.value })} /></div>
      <input aria-label="PR title" placeholder="Title" value={props.title} onChange={(event) => props.onFields({ title: event.target.value })} />
      <textarea aria-label="PR description" placeholder="Description" value={props.body} onChange={(event) => props.onFields({ body: event.target.value })} />
      <div className="dock-actions">
        <label className="dock-check"><input type="checkbox" checked={props.draft} onChange={(event) => props.onFields({ draft: event.target.checked })} /> Draft</label>
        <span className="dock-spacer" />
        <button type="button" className="primary-button compact" disabled={props.disabled || !props.base.trim() || !props.title.trim()}
          onClick={() => void props.run(() => props.onCreate(props.remote, props.base, props.title, props.body, props.draft))}>
          Create pull request
        </button>
      </div>
    </div>
  );
}

interface CommentsListProps {
  comments: DiffComment[];
  /** The current changes, to mark comments whose lines moved since. */
  files: GitChangeFile[];
  disabled: boolean;
  mode: TaskMode;
  editText: Record<string, string>;
  setEditText: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  onComments: (comments: DiffComment[]) => Promise<void>;
  onAddressComments: (comments: DiffComment[]) => Promise<boolean>;
  run: Run;
}

export function CommentsList(props: CommentsListProps) {
  const { comments, files, editText, setEditText } = props;
  const drop = (id: string) => setEditText((before) => { const next = { ...before }; delete next[id]; return next; });
  return (
    <div className="dock-form">
      {comments.length === 0 && <small className="dock-note">No pending comments — hover a diff line and click +.</small>}
      {comments.map((comment) => {
        const file = files.find((item) => item.path === comment.path);
        const fresh = isFresh(comment, file);
        const draft = editText[comment.id];
        return (
          <div className="dock-comment" key={comment.id}>
            <div className="diff-comment-meta">
              <span>{comment.path}:{comment.line} · {comment.layer}{comment.side === "old" ? " · removed" : ""}</span>
              {!fresh && <em>changed since comment</em>}
            </div>
            {draft !== undefined ? (
              <>
                <textarea aria-label={`Edit comment ${comment.path}:${comment.line}`} value={draft}
                  onChange={(event) => setEditText((before) => ({ ...before, [comment.id]: event.target.value }))} />
                <div className="diff-comment-actions">
                  <button type="button" className="compact" onClick={() => drop(comment.id)}>Cancel</button>
                  <button type="button" className="compact" disabled={props.disabled || !draft.trim()}
                    onClick={() => void props.run(async () => { await props.onComments(comments.map((item) => item.id === comment.id ? { ...item, text: draft.trim() } : item)); drop(comment.id); })}>Save</button>
                </div>
              </>
            ) : (
              <>
                <p>{comment.text}</p>
                <div className="diff-comment-actions">
                  <button type="button" className="compact" onClick={() => setEditText((before) => ({ ...before, [comment.id]: comment.text }))}>Edit</button>
                  <button type="button" className="compact danger" disabled={props.disabled}
                    onClick={() => void props.run(() => props.onComments(comments.filter((item) => item.id !== comment.id)))}>Remove</button>
                </div>
              </>
            )}
          </div>
        );
      })}
      {comments.length > 0 && (
        <div className="dock-actions">
          <span className="dock-note">{comments.length} pending</span>
          <span className="dock-spacer" />
          <button type="button" className="primary-button compact" disabled={props.disabled}
            onClick={() => void props.run(() => props.onAddressComments(comments))}>
            {props.mode === "build" ? "Address comments" : "Plan fixes"}
          </button>
        </div>
      )}
    </div>
  );
}
