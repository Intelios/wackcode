import { useEffect, useState } from "react";
import type { DiffComment, GitChangeFile, GitChanges, GitDiffSection, GitGeneratedMessage, GitPrInfo, GitPublishInfo, TaskMode } from "../types";
import { Icon } from "./Icons";

type Action = "stage" | "unstage" | "discard";
interface Props {
  changes?: GitChanges;
  loading: boolean;
  busy: boolean;
  width: number;
  mode: TaskMode;
  canReview: boolean;
  reviewReason?: string;
  comments: DiffComment[];
  onWidthChange: (width: number) => void;
  onClose: () => void;
  onRefresh: () => void;
  onSettings: () => void;
  onReview: () => Promise<boolean>;
  onAction: (file: GitChangeFile, section: GitDiffSection, action: Action, hunkId?: number) => Promise<void>;
  onCommit: (message: string, revision: string) => Promise<void>;
  onGenerate: () => Promise<GitGeneratedMessage>;
  onPublishInfo: () => Promise<GitPublishInfo>;
  onPush: (remote?: string) => Promise<void>;
  onPreparePr: (remote: string) => Promise<GitPrInfo>;
  onCreatePr: (remote: string, base: string, title: string, body: string, draft: boolean) => Promise<string>;
  onOpenPr: (url: string) => void;
  onComments: (comments: DiffComment[]) => Promise<void>;
  onAddressComments: (comments: DiffComment[]) => Promise<boolean>;
}

export function ChangesPanel(props: Props) {
  const { changes, loading, busy, width, mode, canReview, reviewReason, comments } = props;
  const [selected, setSelected] = useState<{ path: string; layer: "staged" | "working" }>();
  const [message, setMessage] = useState("");
  const [messageRevision, setMessageRevision] = useState<string>();
  const [publish, setPublish] = useState<GitPublishInfo>();
  const [remote, setRemote] = useState("");
  const [pr, setPr] = useState<GitPrInfo>();
  const [prTitle, setPrTitle] = useState("");
  const [prBase, setPrBase] = useState("");
  const [prBody, setPrBody] = useState("");
  const [draft, setDraft] = useState(false);
  const [commentAt, setCommentAt] = useState<Omit<DiffComment, "id" | "text">>();
  const [commentText, setCommentText] = useState("");
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const files = changes?.files ?? [];
  const entries = files.flatMap((file) => file.sections.map((section) => ({ file, section })));
  const current = entries.find((entry) => entry.file.path === selected?.path && entry.section.layer === selected.layer) ?? entries[0];
  const staged = entries.filter((entry) => entry.section.layer === "staged");
  const working = entries.filter((entry) => entry.section.layer === "working");
  const disabled = busy || localBusy;

  useEffect(() => {
    if (!changes?.isGit) return;
    void props.onPublishInfo().then((info) => {
      setPublish(info);
      setRemote((before) => info.remotes.includes(before) ? before : info.remotes[0] ?? "");
    }).catch(() => undefined);
    // Fetch when the checkout or branch changes, while preserving an in-progress form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changes?.root, changes?.branch]);

  async function run(work: () => Promise<unknown>) {
    setLocalBusy(true); setError("");
    try { await work(); }
    catch (reason) { setError(String(reason)); }
    finally { setLocalBusy(false); }
  }

  function startResize(event: React.PointerEvent) {
    const startX = event.clientX;
    const startWidth = width;
    const move = (moveEvent: PointerEvent) => props.onWidthChange(Math.max(290, Math.min(720, startWidth + startX - moveEvent.clientX)));
    const end = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
  }

  function actionButtons(file: GitChangeFile, section: GitDiffSection, hunkId?: number) {
    const partial = hunkId !== undefined;
    const allowed = !partial || (file.hunkable && !section.truncated);
    const locked = disabled || file.status === "conflict" || !allowed;
    const reason = file.status === "conflict" ? "Resolve this conflict first" : !allowed ? "Whole-file actions only" : undefined;
    return section.layer === "staged" ? (
      <button type="button" disabled={locked} title={reason} onClick={() => void run(() => props.onAction(file, section, "unstage", hunkId))}>Unstage</button>
    ) : (
      <>
        <button type="button" disabled={locked} title={reason} onClick={() => void run(() => props.onAction(file, section, "stage", hunkId))}>Stage</button>
        <button type="button" disabled={locked} title={reason} onClick={() => void run(() => props.onAction(file, section, "discard", hunkId))}>Discard</button>
      </>
    );
  }

  function addComment(file: GitChangeFile, section: GitDiffSection, line: { kind: string; text: string; oldLine: number | null; newLine: number | null }) {
    const side = line.kind === "deletion" ? "old" : "new";
    const number = side === "old" ? line.oldLine : line.newLine;
    if (number === null) return;
    setCommentAt({ path: file.path, layer: section.layer, side, line: number, excerpt: line.text.slice(0, 1000), revision: section.revision });
    setCommentText("");
  }

  async function saveComment() {
    if (!commentAt || !commentText.trim()) return;
    await run(async () => {
      await props.onComments([...comments, { ...commentAt, id: crypto.randomUUID(), text: commentText.trim() }]);
      setCommentAt(undefined); setCommentText("");
    });
  }

  async function preparePr() {
    if (!remote) return;
    await run(async () => {
      const info = await props.onPreparePr(remote);
      setPr(info); setPrTitle(info.title); setPrBase(info.base); setPrBody(info.body);
    });
  }

  const hasStaged = files.some((file) => file.staged);
  return (
    <aside className="changes-panel" style={{ width }}>
      <div className="panel-resizer" onPointerDown={startResize} />
      <header className="changes-header">
        <div><h3>Changes <span>{files.length}</span></h3></div>
        <div><button className="icon-button" onClick={props.onRefresh} aria-label="Refresh changes"><Icon name="refresh" className={loading ? "spinning" : ""} /></button><button className="icon-button" onClick={props.onClose} aria-label="Close changes panel">×</button></div>
      </header>
      {error && <div className="error-banner">{error}</div>}
      <div className="changes-toolbar">
        <button type="button" className="secondary-button" disabled={disabled || !changes?.isGit || files.length === 0 || !canReview} title={reviewReason} onClick={() => void run(() => props.onReview())}>Review changes</button>
        {!canReview && reviewReason && <button type="button" className="link-button" onClick={props.onSettings}>{reviewReason}</button>}
      </div>
      {!changes?.isGit ? (
        <div className="panel-empty"><Icon name="git" /><strong>No Git repository</strong><span>Chat and editing still work. Changes can’t be summarized here.</span></div>
      ) : (
        <>
          {files.length === 0 ? <div className="panel-empty"><span className="clean-check">✓</span><strong>Working tree clean</strong><span>No staged, unstaged, or untracked files.</span></div> : (
            <>
              <div className="changed-files">
                {([ ["Staged", staged], ["Working tree", working] ] as const).map(([label, group]) => group.length > 0 && <div key={label}>
                  <h4>{label} <span>{group.length}</span></h4>
                  {group.map(({ file, section }) => <button type="button" key={section.layer + ":" + file.path} className={current?.file.path === file.path && current.section.layer === section.layer ? "active" : ""} onClick={() => setSelected({ path: file.path, layer: section.layer })} title={file.path}>
                    <span className={"status-letter " + file.status}>{file.status[0]?.toUpperCase()}</span><span>{file.path}</span>
                  </button>)}
                </div>)}
              </div>
              {current && <>
                <div className="diff-file-header"><span title={current.file.path}>{current.file.oldPath ? current.file.oldPath + " → " : ""}{current.file.path}</span>{current.file.binary && <em>binary</em>}{current.section.truncated && <em>truncated</em>}{actionButtons(current.file, current.section)}</div>
                <div className="diff-view" aria-label={"Diff for " + current.file.path}>
                  {current.file.binary ? <pre>Binary file</pre> : current.section.hunks.length === 0 ? <pre>{current.section.diff || "No text diff available"}</pre> : current.section.hunks.map((hunk) => <div key={hunk.id}>
                    <div className="diff-hunk"><span>{hunk.header}</span>{actionButtons(current.file, current.section, hunk.id)}</div>
                    {hunk.lines.map((line, index) => <div className={"diff-line " + line.kind} key={index}>
                      <button type="button" className="diff-comment-button" aria-label={"Comment on " + current.file.path + " line " + (line.newLine ?? line.oldLine)} disabled={line.oldLine === null && line.newLine === null} onClick={() => addComment(current.file, current.section, line)}>＋</button>
                      <span className="diff-line-number">{line.oldLine ?? ""}</span><span className="diff-line-number">{line.newLine ?? ""}</span><code>{line.text || " "}</code>
                    </div>)}
                  </div>)}
                </div>
              </>}
            </>
          )}
          {commentAt && <div className="changes-form"><strong>Comment on {commentAt.path}:{commentAt.line}</strong><textarea aria-label="Diff comment" value={commentText} onChange={(event) => setCommentText(event.target.value)} /><div><button type="button" onClick={() => setCommentAt(undefined)}>Cancel</button><button type="button" disabled={!commentText.trim() || disabled} onClick={() => void saveComment()}>Save comment</button></div></div>}
          {comments.length > 0 && <div className="changes-form"><strong>Pending comments ({comments.length})</strong>
            {comments.map((comment) => {
              const fresh = files.some((file) => file.path === comment.path && file.sections.some((section) => section.layer === comment.layer && section.revision === comment.revision));
              return <div className="pending-comment" key={comment.id}><small>{comment.path}:{comment.line} · {comment.layer}{!fresh && " · changed since comment"}</small>
                <textarea aria-label={"Edit comment " + comment.path + ":" + comment.line} value={editing[comment.id] ?? comment.text} onChange={(event) => setEditing((before) => ({ ...before, [comment.id]: event.target.value }))} />
                <div><button type="button" disabled={disabled || !editing[comment.id]?.trim()} onClick={() => void run(() => props.onComments(comments.map((item) => item.id === comment.id ? { ...item, text: editing[item.id].trim() } : item)))}>Save</button><button type="button" disabled={disabled} onClick={() => void run(() => props.onComments(comments.filter((item) => item.id !== comment.id)))}>Remove</button></div>
              </div>;
            })}
            <button type="button" className="primary-button" disabled={disabled} onClick={() => void run(() => props.onAddressComments(comments))}>{mode === "build" ? "Address comments" : "Plan fixes"}</button>
          </div>}
          <div className="changes-form"><strong>Commit</strong><textarea aria-label="Commit message" placeholder="Commit message" value={message} onChange={(event) => { setMessage(event.target.value); setMessageRevision(undefined); }} />
            {messageRevision && messageRevision !== changes.stagedRevision && <small>Staged changes have changed since this message was generated.</small>}
            <div><button type="button" disabled={disabled || !hasStaged} onClick={() => void run(async () => { const generated = await props.onGenerate(); setMessage(generated.message); setMessageRevision(generated.revision); })}>Generate message</button><button type="button" disabled={disabled || !hasStaged || !message.trim()} onClick={() => void run(async () => { await props.onCommit(message, changes.stagedRevision); setMessage(""); setMessageRevision(undefined); })}>Commit</button></div>
          </div>
          <div className="changes-form"><strong>Publish {publish?.branch ?? ""}</strong>{publish?.upstream && <small>Push to {publish.upstream}</small>}
            <select aria-label="Git remote" value={remote} onChange={(event) => setRemote(event.target.value)}>{publish?.remotes.map((name) => <option key={name}>{name}</option>)}</select>
            <div><button type="button" disabled={disabled || !publish?.branch || (!publish?.upstream && !remote)} onClick={() => void run(async () => { await props.onPush(publish?.upstream ? undefined : remote); setPublish(await props.onPublishInfo()); })}>Push</button>
              <button type="button" disabled={disabled || !publish?.branch || !remote} onClick={() => void preparePr()}>Create PR</button></div>
          </div>
          {pr && <div className="changes-form"><strong>GitHub PR · {pr.repo}</strong>{pr.existingUrl ? <button type="button" onClick={() => props.onOpenPr(pr.existingUrl!)}>Open existing PR</button> : <>
            <input aria-label="PR base branch" value={prBase} onChange={(event) => setPrBase(event.target.value)} />
            <input aria-label="PR title" value={prTitle} onChange={(event) => setPrTitle(event.target.value)} />
            <textarea aria-label="PR description" value={prBody} onChange={(event) => setPrBody(event.target.value)} />
            <label><input type="checkbox" checked={draft} onChange={(event) => setDraft(event.target.checked)} /> Draft</label>
            <button type="button" className="primary-button" disabled={disabled || !prBase.trim() || !prTitle.trim()} onClick={() => void run(async () => { const url = await props.onCreatePr(remote, prBase, prTitle, prBody, draft); setPr({ ...pr, existingUrl: url }); })}>Create PR</button>
          </>}</div>}
        </>
      )}
    </aside>
  );
}
