import { useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { DiffComment, GitChangeFile, GitPrInfo, GitPublishInfo, TaskMode } from "../types";
import { Icon } from "./Icons";
import { Select } from "./ui/Select";
import { CommentsList, PrForm } from "./ChangesSheets";

export type DockTab = "commit" | "publish" | "pr" | "comments";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

const TABS: { id: DockTab; icon: "commit" | "push" | "pullRequest" | "comment"; label: string }[] = [
  { id: "commit", icon: "commit", label: "Commit" },
  { id: "publish", icon: "push", label: "Publish" },
  { id: "pr", icon: "pullRequest", label: "Pull request" },
  { id: "comments", icon: "comment", label: "Comments" }
];

interface ChangesDockProps {
  open: DockTab | null;
  onOpen: (tab: DockTab | null) => void;
  changesRevision: string;
  /** File paths the next commit is scoped to; empty means every changed file. */
  commitScope: string[];
  /** Clears `commitScope` back to all changes. */
  onScopeClear: () => void;
  commentCount: number;
  files: GitChangeFile[];
  disabled: boolean;
  mode: TaskMode;
  /** One-shot confirmation ("Committed", "Pushed to origin/main") shown in the bar. */
  flash?: string;
  error?: string;
  onDismissError: () => void;

  message: string;
  messageRevision?: string;
  onMessage: (value: string) => void;
  /** Sets `message` + `messageRevision` on success; not error-wrapped — call through `run`. */
  onGenerate: () => Promise<void>;
  /** Commits `message` for `commitScope` against `changesRevision`; busy/error-wrapped, flashes "Committed". */
  onCommit: () => Promise<void>;

  publish?: GitPublishInfo;
  remote: string;
  onRemote: (remote: string) => void;
  /** Already busy/error-wrapped; pushes to the current upstream or `remote`. */
  onPush: () => Promise<void>;

  pr?: GitPrInfo;
  prBase: string;
  prTitle: string;
  prBody: string;
  draft: boolean;
  onPrFields: (patch: Partial<{ base: string; title: string; body: string; draft: boolean }>) => void;
  onPreparePr: (remote: string) => Promise<GitPrInfo>;
  onCreatePr: (remote: string, base: string, title: string, body: string, draft: boolean) => Promise<string>;
  onOpenPr: (url: string) => void;

  comments: DiffComment[];
  onComments: (comments: DiffComment[]) => Promise<void>;
  onAddressComments: (comments: DiffComment[]) => Promise<boolean>;
  /** Wraps async work: busy flag + error capture. */
  run: (work: () => Promise<unknown>) => Promise<void>;
}

export function ChangesDock(props: ChangesDockProps) {
  const { open, onOpen, disabled } = props;
  const reduce = useReducedMotion();
  const [preparing, setPreparing] = useState(false);
  // The remote the last prepare ran for, so a failed attempt doesn't re-fire in a loop.
  const [attempted, setAttempted] = useState("");
  const [editText, setEditText] = useState<Record<string, string>>({});
  const hasChanges = props.files.length > 0;
  const upstream = props.publish?.upstream;

  // Preparing a PR means asking the agent for title+body — kick it off when the tab opens.
  useEffect(() => {
    if (open !== "pr" || props.pr || preparing || !props.remote || disabled || attempted === props.remote) return;
    setAttempted(props.remote);
    setPreparing(true);
    void props.run(() => props.onPreparePr(props.remote)).finally(() => setPreparing(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, props.pr, props.remote, attempted, disabled]);

  const summary = props.flash ?? (hasChanges
    ? `${props.files.length} changed`
    : "No changes");

  return (
    <div className="changes-dock">
      {props.error && (
        <div className="error-banner dock-error">
          <span>{props.error}</span>
          <button type="button" className="icon-button" aria-label="Dismiss error" onClick={props.onDismissError}><Icon name="close" /></button>
        </div>
      )}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            key="dock-sheet"
            className="dock-sheet"
            initial={reduce ? false : { opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: reduce ? 0 : 0.22, ease: EASE }}
          >
            <div className="dock-sheet-inner" role="tabpanel">
              {open === "commit" && <CommitSheet {...props} hasChanges={hasChanges} />}
              {open === "publish" && <PublishSheet {...props} upstream={upstream} />}
              {open === "pr" && <PrSheet {...props} preparing={preparing} onRetryPrepare={() => setAttempted("")} />}
              {open === "comments" && <CommentsSheet {...props} editText={editText} setEditText={setEditText} />}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      <div className="dock-bar" role="tablist" aria-label="Change actions">
        <span className={`dock-summary ${props.flash ? "flash" : ""}`}>{summary}</span>
        <span className="dock-spacer" />
        {TABS.map(({ id, icon, label }) => {
          const count = id === "comments" ? props.commentCount : 0;
          const hidden = id === "comments" && count === 0;
          const unavailable = id === "commit" ? false : id === "publish" ? !props.publish?.branch : false;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={open === id}
              className={`dock-tab ${open === id ? "active" : ""} ${hidden ? "hidden" : ""}`}
              disabled={unavailable}
              title={id === "publish" && !props.publish?.branch ? "Check out a branch to publish" : undefined}
              onClick={() => onOpen(open === id ? null : id)}
            >
              <Icon name={icon} />
              <span>{label}{count > 0 && ` ${count}`}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function CommitSheet(props: ChangesDockProps & { hasChanges: boolean }) {
  const [generating, setGenerating] = useState(false);
  const stale = props.messageRevision !== undefined && props.messageRevision !== props.changesRevision;
  const scoped = props.commitScope.length > 0;
  return (
    <div className="dock-form">
      <div className="dock-row dock-scope-row">
        <span className="dock-note">Committing</span>
        {scoped ? props.commitScope.map((path) => (
          <span key={path} className="dock-scope-chip" title={path}>
            {path}
            <button type="button" className="icon-button" aria-label={`Commit all changes instead of ${path}`} onClick={props.onScopeClear}><Icon name="close" /></button>
          </span>
        )) : <span className="dock-scope-chip">All changes</span>}
      </div>
      <textarea
        aria-label="Commit message"
        placeholder="Summary"
        value={props.message}
        onChange={(event) => props.onMessage(event.target.value)}
        onKeyDown={(event) => { if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && props.message.trim() && props.hasChanges) void props.onCommit(); }}
      />
      {stale && <small className="dock-note">Changes moved since this message was generated.</small>}
      <div className="dock-actions">
        <button type="button" className="secondary-button compact" disabled={props.disabled || !props.hasChanges || generating}
          onClick={() => { setGenerating(true); void props.run(() => props.onGenerate()).finally(() => setGenerating(false)); }}>
          <Icon name="spark" /> Generate
        </button>
        <span className="dock-spacer" />
        <button type="button" className="primary-button compact" disabled={props.disabled || !props.hasChanges || !props.message.trim()}
          onClick={() => void props.onCommit()}>
          Commit
        </button>
      </div>
    </div>
  );
}

function PublishSheet(props: ChangesDockProps & { upstream?: string | null }) {
  const { publish } = props;
  return (
    <div className="dock-form">
      <div className="dock-row">
        <span className="dock-branch"><Icon name="branch" /> {publish?.branch ?? "No branch"}</span>
        <span className="dock-note">{props.upstream ? `Pushes to ${props.upstream}` : "First push sets the upstream"}</span>
      </div>
      {!props.upstream && (publish?.remotes.length ?? 0) > 0 && (
        <Select
          aria-label="Git remote"
          value={props.remote}
          options={(publish?.remotes ?? []).map((name) => ({ value: name, label: name }))}
          onChange={props.onRemote}
          matchWidth
        />
      )}
      {!props.upstream && publish?.remotes.length === 0 && <small className="dock-note">No remotes configured — add one with `git remote add`.</small>}
      <div className="dock-actions">
        <span className="dock-spacer" />
        <button type="button" className="primary-button compact" disabled={props.disabled || !publish?.branch || (!props.upstream && !props.remote)}
          onClick={() => void props.onPush()}>
          <Icon name="push" /> Push{props.upstream ? ` to ${props.upstream}` : ""}
        </button>
      </div>
    </div>
  );
}

function PrSheet(props: ChangesDockProps & { preparing: boolean; onRetryPrepare: () => void }) {
  return (
    <PrForm
      pr={props.pr}
      base={props.prBase}
      title={props.prTitle}
      body={props.prBody}
      draft={props.draft}
      preparing={props.preparing}
      disabled={props.disabled}
      remote={props.remote}
      onFields={props.onPrFields}
      onRetryPrepare={props.onRetryPrepare}
      onCreate={props.onCreatePr}
      onOpenPr={props.onOpenPr}
      run={props.run}
    />
  );
}

function CommentsSheet(props: ChangesDockProps & { editText: Record<string, string>; setEditText: React.Dispatch<React.SetStateAction<Record<string, string>>> }) {
  return (
    <CommentsList
      comments={props.comments}
      files={props.files}
      disabled={props.disabled}
      mode={props.mode}
      editText={props.editText}
      setEditText={props.setEditText}
      onComments={props.onComments}
      onAddressComments={props.onAddressComments}
      run={props.run}
    />
  );
}
