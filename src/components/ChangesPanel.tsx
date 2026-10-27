import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { LayoutGroup, motion, useReducedMotion } from "motion/react";
import type { DiffComment, GitChangeFile, GitChanges, GitDiffLine, GitDiffSection, GitGeneratedMessage, GitPrInfo, GitPublishInfo, TaskMode } from "../types";
import type { ChangeEntry } from "../changes-utils";
import { changeEntries, lineAnchor } from "../changes-utils";
import { ChangesFileList } from "./ChangesFileList";
import { ChangesDock, type DockTab } from "./ChangesDock";
import { DiffView, type CommentAnchor } from "./DiffView";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

/** The diff's layout as the user last chose it; until they do, it follows the panel's width. */
const DIFF_LAYOUT_KEY = "wackcode:diffLayout";
type DiffLayoutChoice = "auto" | "unified" | "split";
/** From this width the file list sits beside the diff, and Unified/Split is offered. */
export const TWO_PANE_WIDTH = 720;
/** "auto" goes side by side from here. */
const AUTO_SPLIT_WIDTH = 1000;

function storedDiffLayout(): DiffLayoutChoice {
  try {
    const value = localStorage.getItem(DIFF_LAYOUT_KEY);
    return value === "split" || value === "unified" ? value : "auto";
  } catch { return "auto"; }
}

/** The layout a panel this wide shows: always one column when narrow, otherwise the user's pick. */
export function diffLayoutFor(width: number, choice: DiffLayoutChoice): "unified" | "split" {
  if (width < TWO_PANE_WIDTH) return "unified";
  return choice === "auto" ? (width >= AUTO_SPLIT_WIDTH ? "split" : "unified") : choice;
}

interface Props {
  selectedEntry?: { path: string; layer: "staged" | "working" };
  onSelectEntry?: (selection: { path: string; layer: "staged" | "working" }) => void;
  changes?: GitChanges;
  loading: boolean;
  busy: boolean;
  mode: TaskMode;
  canReview: boolean;
  reviewReason?: string;
  comments: DiffComment[];
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
  const reduce = useReducedMotion();
  const root = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [layoutChoice, setLayoutChoice] = useState<DiffLayoutChoice>(storedDiffLayout);
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => setWidth(element.clientWidth);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const diffLayout = diffLayoutFor(width, layoutChoice);
  function chooseLayout(choice: "unified" | "split") {
    setLayoutChoice(choice);
    try { localStorage.setItem(DIFF_LAYOUT_KEY, choice); } catch { /* the choice still holds for this panel */ }
  }

  const entries = changeEntries(changes?.files ?? []);
  const selection = props.onSelectEntry ? props.selectedEntry : selected;
  const current = entries.find((entry) => entry.file.path === selection?.path && entry.section.layer === selection.layer) ?? entries[0];
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
      className={`review-button ${!canReview ? "muted" : ""}`}
      disabled={disabled || !changes?.isGit || files.length === 0}
      aria-disabled={!canReview}
      aria-label="Review"
      onClick={() => (canReview ? void run(() => props.onReview()) : props.onSettings())}
    >
      <Icon name="agents" /><span>Review</span>
    </button>
  );

  return (
    <div className="changes-panel" ref={root}>
      <header className="panel-header">
        <div className="panel-title">
          <h3 className="changes-summary">
            {!changes?.isGit ? "No repository" : (
              <>
                {changes.branch && <><span className="changes-branch" title={changes.branch}>{changes.branch}</span>{" "}</>}
                {files.length === 0 ? <span className="changes-meta">{changes.branch ? "clean" : "Clean"}</span> : (
                  <span className="changes-meta">
                    {files.length} {files.length === 1 ? "file" : "files"} <em className="stat-add">+{totals.add}</em> <em className="stat-del">−{totals.del}</em>
                  </span>
                )}
              </>
            )}
          </h3>
        </div>
        <div className="panel-header-actions">
          {reviewUnavailable && reviewReason ? <Tooltip label={reviewReason}>{review}</Tooltip> : review}
          <button className="icon-button" onClick={props.onRefresh} aria-label="Refresh changes"><Icon name="refresh" className={loading ? "spinning" : ""} /></button>
        </div>
        {loading && <span className="panel-loading" aria-hidden="true" />}
      </header>
      {!changes?.isGit ? (
        <div className="panel-empty"><span className="panel-empty-mark" aria-hidden="true"><Icon name="git" /></span><strong>No Git repository</strong><span>Chat and editing still work. Changes can’t be summarized here.</span></div>
      ) : files.length === 0 ? (
        <div className="panel-empty"><span className="panel-empty-mark good" aria-hidden="true"><Icon name="check" /></span><strong>Working tree clean</strong><span>No staged, unstaged, or untracked files.</span></div>
      ) : (
        <div className="changes-body">
          <ChangesFileList
            entries={entries}
            selected={current ? { path: current.file.path, layer: current.section.layer } : undefined}
            commentCounts={commentCounts}
            disabled={disabled}
            onSelect={(entry) => {
              const selection = { path: entry.file.path, layer: entry.section.layer };
              setSelected(selection);
              if (selection.layer !== "commit") props.onSelectEntry?.({ ...selection, layer: selection.layer });
            }}
            onCommitFile={commitFile}
            onAction={entryAction}
          />
          {current && (
            <div className="changes-diff">
            <DiffView
              layout={diffLayout}
              headerExtra={width >= TWO_PANE_WIDTH && !current.file.binary ? (
                <LayoutGroup id="diff-layout">
                  <div className="segmented diff-layout-switch" role="group" aria-label="Diff layout">
                    {(["unified", "split"] as const).map((option) => (
                      <button key={option} type="button" className={`segmented-option${diffLayout === option ? " selected" : ""}`}
                        aria-pressed={diffLayout === option} aria-label={option === "split" ? "Side by side" : "Unified"}
                        title={option === "split" ? "Side by side" : "Unified"} onClick={() => chooseLayout(option)}>
                        {diffLayout === option && <motion.span layoutId="diff-layout-thumb" className="segmented-thumb"
                          transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 520, damping: 36 }} />}
                        <span className="segmented-label"><Icon name={option} /></span>
                      </button>
                    ))}
                  </div>
                </LayoutGroup>
              ) : undefined}
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
            </div>
          )}
        </div>
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
