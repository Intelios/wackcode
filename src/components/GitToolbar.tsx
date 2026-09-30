import { memo, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { GitNetworkKind } from "../hooks/useGitMode";
import type {
  DiffComment, DiffLayout, GitBranches, GitChangeFile, GitCheckoutKind, GitPrInfo, GitSyncStatus, TaskMode, TaskRecord
} from "../types";
import { BranchPicker } from "./BranchPicker";
import { CommentsList, PrForm } from "./ChangesSheets";
import { GitSyncButton } from "./GitSyncButton";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

interface GitToolbarProps {
  branch: string | null;
  /** Why the branch can't be switched right now (a chat is running here). */
  busyReason?: string;
  onListBranches: () => Promise<GitBranches>;
  onCheckout: (name: string, kind: GitCheckoutKind) => Promise<void>;
  sync?: GitSyncStatus;
  network?: { kind: GitNetworkKind; background: boolean };
  networkError?: { message: string; background: boolean };
  onFetch: () => void;
  onPull: () => void;
  onPush: () => void;
  onPublish: (remote: string) => void;
  onDismissNetworkError: () => void;
  onPreparePr: (remote: string) => Promise<GitPrInfo>;
  onCreatePr: (remote: string, base: string, title: string, body: string, draft: boolean) => Promise<string>;
  onOpenPr: (url: string) => void;
  /** The chat AI actions go through; undefined until the project has one. */
  linked?: TaskRecord;
  linkable: TaskRecord[];
  onLink: (taskId: string) => void;
  onOpenChat: (taskId: string) => void;
  review: { can: boolean; reason?: string };
  onReview: () => Promise<void>;
  onSettings: () => void;
  comments: DiffComment[];
  files: GitChangeFile[];
  mode: TaskMode;
  onComments: (comments: DiffComment[]) => Promise<void>;
  onAddressComments: (comments: DiffComment[]) => Promise<boolean>;
  layout: DiffLayout;
  onLayout: (layout: DiffLayout) => void;
  /** The diff toggle only matters where a diff is showing. */
  showLayout: boolean;
  flash?: { text: string; nonce: number };
  /** Bumped by the clean state's "Create a pull request" card to open the PR form. */
  prRequest: number;
  agentName: string;
  onExit: () => void;
}

/** Git mode's header: branch, sync, the linked chat and its AI actions, and the diff layout. */
export const GitToolbar = memo(function GitToolbar(props: GitToolbarProps) {
  const reduce = useReducedMotion();
  const syncRef = useRef<HTMLDivElement>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  const commentsRef = useRef<HTMLButtonElement>(null);
  const [chatsOpen, setChatsOpen] = useState(false);
  const [commentsOpen, setCommentsOpen] = useState(false);
  const [editText, setEditText] = useState<Record<string, string>>({});
  const [prOpen, setPrOpen] = useState(false);
  const [pr, setPr] = useState<GitPrInfo>();
  const [prFields, setPrFields] = useState({ base: "", title: "", body: "", draft: false });
  const [preparing, setPreparing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [shownFlash, setShownFlash] = useState<{ text: string; nonce: number }>();
  const remote = props.sync?.fetchRemote ?? "";

  useEffect(() => {
    if (!props.flash) return;
    setShownFlash(props.flash);
    const timer = setTimeout(() => setShownFlash(undefined), 1900);
    return () => clearTimeout(timer);
  }, [props.flash]);

  async function run(work: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await work(); }
    catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  }

  function preparePr() {
    if (!remote) return;
    setPreparing(true); setError("");
    void props.onPreparePr(remote)
      .then((info) => { setPr(info); setPrFields({ base: info.base, title: info.title, body: info.body, draft: false }); })
      .catch((reason) => setError(String(reason)))
      .finally(() => setPreparing(false));
  }

  function openPr() {
    setPrOpen(true);
    setPr(undefined);
    preparePr();
  }

  useEffect(() => {
    if (props.prRequest > 0) openPr();
    // Only a new request opens the form, not a changed remote.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.prRequest]);

  const review = (
    <button
      type="button"
      className={`panel-button review-button ${!props.review.can ? "muted" : ""}`}
      disabled={busy || props.files.length === 0}
      aria-disabled={!props.review.can}
      onClick={() => (props.review.can ? void run(() => props.onReview()) : props.onSettings())}
    >
      <Icon name="agents" /> Review
    </button>
  );

  return (
    // The toolbar's bare background and spacer drag the window (the title bar is an overlay).
    <header className="workspace-header git-toolbar" data-tauri-drag-region>
      <div className="git-toolbar-group">
        <BranchPicker
          branch={props.branch}
          variant="toolbar"
          disabledReason={props.busyReason}
          onLoad={props.onListBranches}
          onCheckout={props.onCheckout}
        />
        <div ref={syncRef} className="git-sync-anchor">
          <GitSyncButton
            sync={props.sync}
            network={props.network}
            error={props.networkError}
            busyReason={props.busyReason}
            onFetch={props.onFetch}
            onPull={props.onPull}
            onPush={props.onPush}
            onPublish={props.onPublish}
            onCreatePr={openPr}
            onDismissError={props.onDismissNetworkError}
          />
        </div>
      </div>
      <span className="git-toolbar-spacer" data-tauri-drag-region />
      <AnimatePresence>
        {shownFlash && (
          <motion.span
            key={shownFlash.nonce}
            className="git-flash"
            role="status"
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: -8, scale: 0.94 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: -6 }}
            transition={{ type: "spring", stiffness: 480, damping: 30 }}
          ><Icon name="check" /> {shownFlash.text}</motion.span>
        )}
      </AnimatePresence>
      <div className="git-toolbar-group">
        <button
          ref={chipRef}
          type="button"
          className={`git-linked-chip ${props.linked ? "" : "empty"}`}
          aria-haspopup="dialog"
          aria-expanded={chatsOpen}
          aria-label={props.linked ? `${props.agentName} works via ${props.linked.name}. Change chat` : `No chat in this project yet. Review and comments start one`}
          onClick={() => setChatsOpen((value) => !value)}
        >
          {props.linked && <span className={`task-status ${props.linked.lastError ? "error" : props.linked.status}`} />}
          <span className="git-linked-via">via</span>
          <span className="git-linked-name">{props.linked?.name ?? "New chat"}</span>
          <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
        </button>
        {props.review.reason && !props.review.can ? <Tooltip label={props.review.reason}>{review}</Tooltip> : review}
        {props.comments.length > 0 && (
          <button ref={commentsRef} type="button" className="panel-button git-comments-chip" aria-haspopup="dialog" aria-expanded={commentsOpen}
            aria-label={`${props.comments.length} pending ${props.comments.length === 1 ? "comment" : "comments"}`}
            onClick={() => setCommentsOpen((value) => !value)}>
            <Icon name="comment" /> {props.comments.length}
          </button>
        )}
        {props.showLayout && <LayoutToggle layout={props.layout} onLayout={props.onLayout} />}
        <Tooltip label={<>Exit Git mode <kbd>⌘⇧G</kbd></>}>
          <button type="button" className="icon-button git-exit" aria-label="Exit Git mode" onClick={props.onExit}><Icon name="close" /></button>
        </Tooltip>
      </div>
      {error && (
        <div className="git-toolbar-error" role="alert">
          <span>{error}</span>
          <button type="button" className="icon-button" aria-label="Dismiss error" onClick={() => setError("")}><Icon name="close" /></button>
        </div>
      )}

      <Popover anchor={chipRef} open={chatsOpen} onClose={() => setChatsOpen(false)} align="end">
        <div className="git-chat-picker" role="dialog" aria-label="Choose the chat for AI actions">
          <div className="picker-heading">{props.agentName} works through</div>
          {props.linkable.length === 0 && <div className="branch-empty">No chat in this project yet. Review, comments and Ask start one.</div>}
          {props.linkable.map((task) => (
            <button key={task.id} type="button" className={`picker-item ${task.id === props.linked?.id ? "selected" : ""}`}
              onClick={() => { props.onLink(task.id); setChatsOpen(false); }}>
              <span className={`task-status ${task.lastError ? "error" : task.status}`} />
              <span>{task.name}</span>
              {task.id === props.linked?.id && <Icon name="check" />}
            </button>
          ))}
          {props.linked && (
            <div className="repo-picker-foot">
              <button type="button" className="repo-add" onClick={() => { setChatsOpen(false); props.onOpenChat(props.linked!.id); }}>
                <Icon name="external" /> Open {props.linked.name}
              </button>
            </div>
          )}
        </div>
      </Popover>

      <Popover anchor={commentsRef} open={commentsOpen && props.comments.length > 0} onClose={() => setCommentsOpen(false)} align="end" className="git-comments-pop">
        <CommentsList
          comments={props.comments}
          files={props.files}
          disabled={busy}
          mode={props.mode}
          editText={editText}
          setEditText={setEditText}
          onComments={props.onComments}
          onAddressComments={async (comments) => { const sent = await props.onAddressComments(comments); if (sent) setCommentsOpen(false); return sent; }}
          run={run}
        />
      </Popover>

      <Popover anchor={syncRef} open={prOpen} onClose={() => setPrOpen(false)} align="start" className="git-pr-pop">
        <div role="dialog" aria-label="Create pull request">
          <PrForm
            pr={pr}
            base={prFields.base}
            title={prFields.title}
            body={prFields.body}
            draft={prFields.draft}
            preparing={preparing}
            disabled={busy}
            remote={remote}
            onFields={(patch) => setPrFields((before) => ({ ...before, ...patch }))}
            onRetryPrepare={preparePr}
            onCreate={async (...args) => {
              const url = await props.onCreatePr(...args);
              setPr((before) => before ? { ...before, existingUrl: url } : before);
              return url;
            }}
            onOpenPr={props.onOpenPr}
            run={run}
          />
          {error && <div className="branch-error" role="alert">{error}</div>}
        </div>
      </Popover>
    </header>
  );
});

function LayoutToggle({ layout, onLayout }: { layout: DiffLayout; onLayout: (layout: DiffLayout) => void }) {
  const reduce = useReducedMotion();
  const options: { id: DiffLayout; label: string; icon: "unified" | "split" }[] = [
    { id: "unified", label: "Unified diff", icon: "unified" },
    { id: "split", label: "Split diff", icon: "split" }
  ];
  return (
    <div className="git-layout-toggle" role="group" aria-label="Diff layout">
      {options.map((option) => (
        <Tooltip key={option.id} label={option.label}>
          <button type="button" aria-label={option.label} aria-pressed={layout === option.id} className={layout === option.id ? "active" : ""} onClick={() => onLayout(option.id)}>
            {layout === option.id && (
              <motion.span layoutId="git-layout-pill" className="git-layout-pill" transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 520, damping: 38 }} />
            )}
            <Icon name={option.icon} />
          </button>
        </Tooltip>
      ))}
    </div>
  );
}
