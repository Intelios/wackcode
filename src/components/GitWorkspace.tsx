import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { lineAnchor } from "../changes-utils";
import { firstLine, repoWebCopy, syncAction } from "../git-mode";
import type { DiffComment, DiffLayout, GitChangeFile, GitDiffLine, GitDiffSection, GitSyncStatus, TaskRecord } from "../types";
import { DiffView, type CommentAnchor } from "./DiffView";
import { DuckMark } from "./DuckMark";
import { Icon, type IconName } from "./Icons";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface GitWorkspaceProps {
  isGit: boolean;
  loading: boolean;
  error?: string;
  /** The selected changed file; undefined when the tree is clean. */
  file?: GitChangeFile;
  layout: DiffLayout;
  direction: 1 | -1;
  /** The linked chat's pending comments. */
  comments: DiffComment[];
  disabled: boolean;
  onComments: (comments: DiffComment[]) => Promise<void>;
  onDiscard: (file: GitChangeFile, section: GitDiffSection, hunkId?: number) => void;
  sync?: GitSyncStatus;
  agentName: string;
  linked?: TaskRecord;
  onPush: () => void;
  onPull: () => void;
  onPublish: (remote: string) => void;
  onCreatePr: () => void;
  /** The repository's web page for the clean state's "Open in GitHub" card. */
  repoUrl?: string | null;
  onOpenRepo: (url: string) => void;
  onOpenChat: (taskId: string) => void;
  onReveal: () => void;
  divergence?: { ahead: number; behind: number; upstream: string };
  onAskIntegrate: () => void;
  onDismissDivergence: () => void;
  actionError?: string;
  onDismissError: () => void;
  /** Rendered above everything else in the well (History's revert card lives in App). */
  notice?: ReactNode;
}

/** Git mode's main area on the Changes tab: the selected file's diff, full height. */
export const GitWorkspace = memo(function GitWorkspace(props: GitWorkspaceProps) {
  const reduce = useReducedMotion();
  const [commentAt, setCommentAt] = useState<CommentAnchor>();
  const [commentText, setCommentText] = useState("");
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const { file } = props;

  function addComment(section: GitDiffSection, line: GitDiffLine) {
    const anchor = lineAnchor(line);
    if (!file || !anchor || section.layer === "commit") return;
    setCommentAt({ path: file.path, layer: section.layer, side: anchor.side, line: anchor.line, excerpt: line.text.slice(0, 1000), revision: section.revision });
    setCommentText("");
  }

  async function save(work: () => Promise<void>) {
    setSaving(true);
    try { await work(); } finally { setSaving(false); }
  }

  function editComment(id: string, text: string | null) {
    setEditing((before) => {
      const next = { ...before };
      if (text === null) delete next[id];
      else next[id] = text;
      return next;
    });
  }

  return (
    <div className="git-workspace">
      {props.notice}
      {props.divergence && (
        <GitNoticeCard
          icon="branch"
          title={`Your branch and ${props.divergence.upstream} have diverged.`}
          body={`${props.sync?.ahead ?? props.divergence.ahead} local and ${props.sync?.behind ?? props.divergence.behind} remote ${(props.sync?.behind ?? props.divergence.behind) === 1 ? "commit" : "commits"}. Pull only fast-forwards, so nothing was merged.`}
          action={`Ask ${props.agentName} to rebase or merge`}
          onAction={props.onAskIntegrate}
          onDismiss={props.onDismissDivergence}
        />
      )}
      {props.actionError && (
        <div className="error-banner git-error" role="alert">
          <span>{firstLine(props.actionError)}</span>
          <button type="button" className="icon-button" aria-label="Dismiss error" onClick={props.onDismissError}><Icon name="close" /></button>
        </div>
      )}
      {!props.isGit ? (
        <div className="git-empty">
          <Icon name="git" />
          <strong>{props.error ? "Couldn't read this repository" : "Not a Git repository"}</strong>
          <span>{props.error ? firstLine(props.error) : "This project folder isn't tracked by Git, so there's nothing to review here."}</span>
        </div>
      ) : file ? (
        <div className="git-diff-well">
          <DiffView
            file={file}
            sections={file.sections}
            layout={props.layout}
            direction={props.direction}
            comments={props.comments}
            commentAt={commentAt}
            commentText={commentText}
            editing={editing}
            disabled={props.disabled || saving}
            onAddComment={addComment}
            onCommentText={setCommentText}
            onCloseComposer={() => setCommentAt(undefined)}
            onSaveComment={() => {
              if (!commentAt || !commentText.trim()) return;
              const text = commentText.trim();
              void save(async () => {
                await props.onComments([...props.comments, { ...commentAt, id: crypto.randomUUID(), text }]);
                setCommentAt(undefined); setCommentText("");
              });
            }}
            onEditComment={editComment}
            onUpdateComment={(comment, text) => void save(async () => {
              await props.onComments(props.comments.map((item) => item.id === comment.id ? { ...item, text: text.trim() } : item));
              editComment(comment.id, null);
            })}
            onRemoveComment={(id) => void save(() => props.onComments(props.comments.filter((item) => item.id !== id)))}
            onAction={props.onDiscard}
            headerExtra={<FileStats file={file} />}
          />
        </div>
      ) : props.loading ? (
        <div className="git-empty quiet"><span className="panel-loading" aria-hidden="true" /></div>
      ) : (
        <CleanState {...props} />
      )}
    </div>
  );
});

function FileStats({ file }: { file: GitChangeFile }) {
  const add = file.sections.reduce((sum, section) => sum + section.additions, 0);
  const del = file.sections.reduce((sum, section) => sum + section.deletions, 0);
  if (!add && !del) return null;
  return <span className="change-stats diff-file-stats">{add > 0 && <em className="stat-add">+{add}</em>}{del > 0 && <em className="stat-del">−{del}</em>}</span>;
}

/** Nothing to commit: the duck, and what to do next, GitHub Desktop style. */
function CleanState(props: GitWorkspaceProps) {
  const reduce = useReducedMotion();
  const action = syncAction(props.sync);
  const cards: { key: string; icon: IconName; title: string; body: string; label: string; onClick: () => void; primary?: boolean }[] = [];
  if (action.kind === "push") {
    cards.push({ key: "push", icon: "push", primary: true, title: `Push ${action.ahead} ${action.ahead === 1 ? "commit" : "commits"} to ${action.remote}`, body: "Your local commits aren't on the remote yet.", label: `Push ${action.remote}`, onClick: props.onPush });
  } else if (action.kind === "publish") {
    cards.push({ key: "publish", icon: "push", primary: true, title: "Publish your branch", body: `${props.sync?.branch ?? "This branch"} isn't on ${action.remote} yet.`, label: "Publish branch", onClick: () => props.onPublish(action.remote) });
  } else if (action.kind === "pull") {
    cards.push({ key: "pull", icon: "pull", primary: true, title: `Pull ${action.behind} ${action.behind === 1 ? "commit" : "commits"} from ${action.remote}`, body: "The remote has commits you don't have yet.", label: `Pull ${action.remote}`, onClick: props.onPull });
  }
  if (props.sync?.upstream && action.kind !== "publish") {
    cards.push({ key: "pr", icon: "pullRequest", title: "Create a pull request", body: `Open a pull request for ${props.sync.branch ?? "this branch"} with your own gh login.`, label: "Create pull request", onClick: props.onCreatePr });
  }
  const repoUrl = props.repoUrl;
  if (repoUrl) {
    const copy = repoWebCopy(repoUrl);
    cards.push({ key: "repo", icon: "external", title: copy.title, body: copy.body, label: copy.label, onClick: () => props.onOpenRepo(repoUrl) });
  }
  if (props.linked) {
    const linked = props.linked;
    cards.push({ key: "chat", icon: "comment", title: `Back to ${linked.name}`, body: `Ask ${props.agentName} for the next change.`, label: "Open chat", onClick: () => props.onOpenChat(linked.id) });
  }
  cards.push({ key: "reveal", icon: "folder", title: "Open the folder", body: "See the project's files in Finder.", label: "Reveal in Finder", onClick: props.onReveal });

  return (
    <div className="git-clean">
      <motion.div
        className="git-clean-duck"
        initial={reduce ? false : { opacity: 0, y: -28, scale: 0.8 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 16 }}
      >
        <span className="git-clean-ripple" aria-hidden="true" />
        <span className="git-clean-ripple late" aria-hidden="true" />
        <span className="brand-mark"><DuckMark /></span>
      </motion.div>
      <h2>Nothing to commit</h2>
      <p>No local changes in this repository. Here's what you can do next.</p>
      <div className="git-suggestions">
        {cards.map((card, index) => (
          <motion.div
            key={card.key}
            className={`git-suggestion ${card.primary ? "primary" : ""}`}
            initial={reduce ? false : { opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: reduce ? 0 : 0.24, delay: reduce ? 0 : 0.12 + index * 0.04, ease: EASE }}
          >
            <Icon name={card.icon} />
            <div>
              <strong>{card.title}</strong>
              <span>{card.body}</span>
            </div>
            <button type="button" className={`${card.primary ? "primary-button" : "secondary-button"} compact`} onClick={card.onClick}>{card.label}</button>
          </motion.div>
        ))}
      </div>
    </div>
  );
}

/** A warning card at the top of Git mode's well, with a hand-off to the agent. */
export function GitNoticeCard({ icon, title, body, action, onAction, onDismiss }: {
  icon: IconName; title: string; body: string; action: string; onAction: () => void; onDismiss: () => void;
}) {
  const reduce = useReducedMotion();
  return (
    <motion.div
      className="git-card-notice warning"
      role="alert"
      initial={reduce ? false : { opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: reduce ? 0 : 0.22, ease: EASE }}
    >
      <Icon name={icon} />
      <div>
        <strong>{title}</strong>
        <span>{body}</span>
      </div>
      <button type="button" className="secondary-button compact" onClick={onAction}><Icon name="spark" /> {action}</button>
      <button type="button" className="icon-button" aria-label="Dismiss" onClick={onDismiss}><Icon name="close" /></button>
    </motion.div>
  );
}

interface GitActivityProps {
  /** Chats running in the project's folder, linked one first. */
  busy: TaskRecord[];
  /** Chats waiting on an answer from the user (a question or an access card). */
  waiting: ReadonlySet<string>;
  agentName: string;
  onOpenChat: (taskId: string) => void;
}

/**
 * The live strip above Git mode's tabs: who is working in this folder (Git writes wait for
 * them), and, once the run ends, that the answer is waiting in the chat. Git mode hides the
 * transcript, so this is the only sign a hand-off is running or done.
 */
export const GitActivity = memo(function GitActivity({ busy, waiting, agentName, onOpenChat }: GitActivityProps) {
  const reduce = useReducedMotion();
  const [finished, setFinished] = useState<TaskRecord>();
  const last = useRef<TaskRecord | undefined>(undefined);
  const current = busy[0];

  useEffect(() => {
    if (current) setFinished(undefined);
    else if (last.current) setFinished(last.current);
    last.current = current;
  }, [current]);

  const shown = current ?? finished;
  const asking = current ? waiting.has(current.id) : false;
  return (
    <AnimatePresence initial={false}>
      {shown && (
        <motion.div
          key="activity"
          className="git-activity"
          role="status"
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: -10, height: 0 }}
          animate={{ opacity: 1, y: 0, height: "auto" }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: -10, height: 0 }}
          transition={{ duration: reduce ? 0 : 0.24, ease: EASE }}
        >
          <span className={`git-activity-inner ${current ? "running" : "done"}`}>
            {current ? <span className="task-status running" /> : <Icon name="check" />}
            {current ? (
              asking
                ? <span><strong>{agentName}</strong> has a question for you in <strong>{current.name}</strong>.</span>
                : <span><strong>{agentName}</strong> is working in <strong>{current.name}</strong>{busy.length > 1 ? ` and ${busy.length - 1} more` : ""}. Commits wait until it finishes.</span>
            ) : (
              <span><strong>{agentName}</strong> finished in <strong>{shown.name}</strong>.</span>
            )}
            <button type="button" className="text-button" onClick={() => onOpenChat(shown.id)}>Open chat</button>
            {!current && <button type="button" className="icon-button" aria-label="Dismiss" onClick={() => setFinished(undefined)}><Icon name="close" /></button>}
          </span>
        </motion.div>
      )}
    </AnimatePresence>
  );
});
