import { useEffect, useRef, useState } from "react";
import type { DiffComment, GitChangeFile, GitChanges, GitDiffLine, GitDiffSection, GitGeneratedMessage, GitPrInfo, GitPublishInfo, TaskMode } from "../types";
import type { ChangeEntry } from "../changes-utils";
import { changeEntries, lineAnchor } from "../changes-utils";
import { ChangesFileList } from "./ChangesFileList";
import { ChangesDock, type DockTab } from "./ChangesDock";
import { DiffView, type CommentAnchor } from "./DiffView";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

interface Props {
  changes?: GitChanges;
  loading: boolean;
  busy: boolean;
  mode: TaskMode;
  canReview: boolean;
  reviewReason?: string;
  comments: DiffComment[];
  onClose: () => void;
  onRefresh: () => void;
  onSettings: () => void;
  onReview: () => Promise<boolean>;
  onAction: (file: GitChangeFile, section: GitDiffSection, hunkId?: number) => Promise<void>;
  onCommit: (message: string, files: string[], revision: string) => Promise<void>;
  onGenerate: () => Promise<GitGeneratedMessage>;
  onPublishInfo: () => Promise<GitPublishInfo>;
  onPush: (remote?: string) => Promise<void>;
  onPreparePr: (remote: string) => Promise<GitPrInfo>;
  onCreatePr: (remote: string, base: string, title: string, body: string, draft: boolean) => Promise<string>;
  onOpenPr: (url: string) => void;
  onComments: (comments: DiffComment[]) => Promise<void>;
  onAddressComments: (comments: DiffComment[]) => Promise<boolean>;
  /** Opens Git mode on this file; only for chats that work in their project's own folder. */
  onOpenGitMode?: (path?: string) => void;
}

/** The side panel's Changes view: the chat's Git changes, their diffs, and the commit dock. */
export function ChangesPanel(props: Props) {
  const { changes, loading, busy, canReview, reviewReason, comments } = props;
  const [selected, setSelected] = useState<{ path: string; layer: GitDiffSection["layer"] }>();
  const [message, setMessage] = useState("");
  const [messageRevision, setMessageRevision] = useState<string>();
  const [publish, setPublish] = useState<GitPublishInfo>();
  const [remote, setRemote] = useState("");
  const [pr, setPr] = useState<GitPrInfo>();
  const [prFields, setPrFields] = useState({ base: "", title: "", body: "", draft: false });
  const [commentAt, setCommentAt] = useState<CommentAnchor>();
  const [commentText, setCommentText] = useState("");
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [dockTab, setDockTab] = useState<DockTab | null>(null);
  /** File paths the commit sheet is scoped to; empty means every changed file. */
  const [commitScope, setCommitScope] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const [flash, setFlash] = useState<string>();
  const flashTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const entries = changeEntries(changes?.files ?? []);
  const current = entries.find((entry) => entry.file.path === selected?.path && entry.section.layer === selected.layer) ?? entries[0];
  const disabled = busy || localBusy;
  const files = changes?.files ?? [];
  // A scoped file that left the change list drops the scope back to all changes.
  const scope = commitScope.filter((path) => files.some((file) => file.path === path));

  const commentCounts = new Map<string, number>();
  for (const comment of comments) {
    const key = `${comment.layer}:${comment.path}`;
    commentCounts.set(key, (commentCounts.get(key) ?? 0) + 1);
  }

  useEffect(() => {
    if (!changes?.isGit) return;
    void props.onPublishInfo().then((info) => {
      setPublish(info);
      setRemote((before) => info.remotes.includes(before) ? before : info.remotes[0] ?? "");
    }).catch(() => undefined);
    // Fetch when the checkout or branch changes, while preserving an in-progress form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changes?.root, changes?.branch]);

  useEffect(() => () => clearTimeout(flashTimer.current), []);

  async function run(work: () => Promise<unknown>) {
    setLocalBusy(true); setError("");
    try { await work(); }
    catch (reason) { setError(String(reason)); }
    finally { setLocalBusy(false); }
  }

  function showFlash(text: string) {
    clearTimeout(flashTimer.current);
    setFlash(text);
    flashTimer.current = setTimeout(() => setFlash(undefined), 1800);
  }

  function entryAction(entry: ChangeEntry, hunkId?: number) {
    void run(() => props.onAction(entry.file, entry.section, hunkId));
  }

  function commitFile(path: string) {
    setCommitScope([path]);
    setDockTab("commit");
  }

  function addComment(line: GitDiffLine) {
    if (!current) return;
    const anchor = lineAnchor(line);
    const layer = current.section.layer;
    if (!anchor || layer === "commit") return;
    setCommentAt({
      path: current.file.path, layer, side: anchor.side, line: anchor.line,
      excerpt: line.text.slice(0, 1000), revision: current.section.revision
    });
    setCommentText("");
  }

  async function saveComment() {
    if (!commentAt || !commentText.trim()) return;
    const text = commentText.trim();
    await run(async () => {
      await props.onComments([...comments, { ...commentAt, id: crypto.randomUUID(), text }]);
      setCommentAt(undefined); setCommentText("");
    });
  }

  function editComment(id: string, text: string | null) {
    setEditing((before) => {
      const next = { ...before };
      if (text === null) delete next[id];
      else next[id] = text;
      return next;
    });
  }

  async function updateComment(comment: DiffComment, text: string) {
    await run(async () => {
      await props.onComments(comments.map((item) => item.id === comment.id ? { ...item, text: text.trim() } : item));
      editComment(comment.id, null);
    });
  }

  async function commit() {
    if (!changes || !message.trim()) return;
    await run(async () => {
      await props.onCommit(message.trim(), scope, changes.changesRevision);
      setMessage(""); setMessageRevision(undefined); setCommitScope([]);
      setDockTab(null);
      showFlash("Committed");
    });
  }

  async function generateMessage() {
    const generated = await props.onGenerate();
    setMessage(generated.message);
    setMessageRevision(generated.revision);
  }

  async function push() {
    await run(async () => {
      await props.onPush(publish?.upstream ? undefined : remote);
      setPublish(await props.onPublishInfo());
      showFlash(`Pushed to ${publish?.upstream ?? remote}`);
    });
  }

  async function preparePr(remoteName: string) {
    const info = await props.onPreparePr(remoteName);
    setPr(info);
    setPrFields({ base: info.base, title: info.title, body: info.body, draft: false });
    return info;
  }

  async function createPr(remoteName: string, base: string, title: string, body: string, draft: boolean) {
    const url = await props.onCreatePr(remoteName, base, title, body, draft);
    setPr((before) => before ? { ...before, existingUrl: url } : before);
    showFlash("Pull request created");
    return url;
  }

  const totals = entries.reduce((sum, entry) => ({ add: sum.add + entry.section.additions, del: sum.del + entry.section.deletions }), { add: 0, del: 0 });
  // The pill stays clickable when the Reviewer is just unconfigured — it opens Settings instead.
  const reviewUnavailable = !canReview || !changes?.isGit || files.length === 0;
  const review = (
    <button
      type="button"
      className={`panel-button review-button ${!canReview ? "muted" : ""}`}
      disabled={disabled || !changes?.isGit || files.length === 0}
      aria-disabled={!canReview}
      onClick={() => (canReview ? void run(() => props.onReview()) : props.onSettings())}
    >
      <Icon name="agents" /> Review
    </button>
  );

  return (
    <div className="changes-panel">
      <header className="panel-header">
        <div className="changes-title">
          <h3>Changes</h3>
          {changes?.isGit && files.length > 0 && (
            <span className="changes-meta">
              {changes.branch && <span className="changes-branch"><Icon name="branch" />{changes.branch}</span>}
              {files.length} {files.length === 1 ? "file" : "files"} · <em className="stat-add">+{totals.add}</em> <em className="stat-del">−{totals.del}</em>
            </span>
          )}
        </div>
        <div className="changes-header-actions">
          {reviewUnavailable && reviewReason ? <Tooltip label={reviewReason}>{review}</Tooltip> : review}
          {props.onOpenGitMode && changes?.isGit && (
            <Tooltip label={<>Open in Git mode <kbd>⌘⇧G</kbd></>}>
              <button className="icon-button" onClick={() => props.onOpenGitMode?.(current?.file.path)} aria-label="Open in Git mode"><Icon name="expand" /></button>
            </Tooltip>
          )}
          <button className="icon-button" onClick={props.onRefresh} aria-label="Refresh changes"><Icon name="refresh" className={loading ? "spinning" : ""} /></button>
          <button className="icon-button" onClick={props.onClose} aria-label="Close changes panel"><Icon name="close" /></button>
        </div>
        {loading && <span className="panel-loading" aria-hidden="true" />}
      </header>
      {!changes?.isGit ? (
        <div className="panel-empty"><Icon name="git" /><strong>No Git repository</strong><span>Chat and editing still work. Changes can’t be summarized here.</span></div>
      ) : files.length === 0 ? (
        <div className="panel-empty"><span className="clean-check">✓</span><strong>Working tree clean</strong><span>No staged, unstaged, or untracked files.</span></div>
      ) : (
        <>
          <ChangesFileList
            entries={entries}
            selected={current ? { path: current.file.path, layer: current.section.layer } : undefined}
            commentCounts={commentCounts}
            disabled={disabled}
            onSelect={(entry) => setSelected({ path: entry.file.path, layer: entry.section.layer })}
            onCommitFile={commitFile}
            onAction={entryAction}
          />
          {current && (
            <DiffView
              file={current.file}
              sections={[current.section]}
              comments={comments}
              commentAt={commentAt}
              commentText={commentText}
              editing={editing}
              disabled={disabled}
              onAddComment={(_, line) => addComment(line)}
              onCommentText={setCommentText}
              onCloseComposer={() => setCommentAt(undefined)}
              onSaveComment={() => void saveComment()}
              onEditComment={editComment}
              onUpdateComment={(comment, text) => void updateComment(comment, text)}
              onRemoveComment={(id) => void run(() => props.onComments(comments.filter((item) => item.id !== id)))}
              onAction={(file, section, hunkId) => entryAction({ file, section }, hunkId)}
            />
          )}
        </>
      )}
      {changes?.isGit && (
        <ChangesDock
          open={dockTab}
          onOpen={setDockTab}
          changesRevision={changes.changesRevision}
          commitScope={scope}
          onScopeClear={() => setCommitScope([])}
          commentCount={comments.length}
          files={files}
          disabled={disabled}
          mode={props.mode}
          flash={flash}
          error={error}
          onDismissError={() => setError("")}
          message={message}
          messageRevision={messageRevision}
          onMessage={(value) => { setMessage(value); setMessageRevision(undefined); }}
          onGenerate={generateMessage}
          onCommit={() => commit()}
          publish={publish}
          remote={remote}
          onRemote={setRemote}
          onPush={push}
          pr={pr}
          prBase={prFields.base}
          prTitle={prFields.title}
          prBody={prFields.body}
          draft={prFields.draft}
          onPrFields={(patch) => setPrFields((before) => ({ ...before, ...patch }))}
          onPreparePr={preparePr}
          onCreatePr={createPr}
          onOpenPr={props.onOpenPr}
          comments={comments}
          onComments={props.onComments}
          onAddressComments={props.onAddressComments}
          run={run}
        />
      )}
    </div>
  );
}
