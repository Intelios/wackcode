import { memo, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { splitPath } from "../changes-utils";
import type { DiffLayout, GitChangeFile, GitCommit, GitCommitFile } from "../types";
import { DiffView } from "./DiffView";
import { Icon } from "./Icons";
import { Menu } from "./ui/Menu";
import { Popover } from "./ui/Popover";
import { Tooltip } from "./ui/Tooltip";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface GitCommitViewProps {
  commit?: GitCommit;
  files?: GitCommitFile[];
  truncated: boolean;
  selectedPath?: string;
  diff?: GitChangeFile;
  loading: boolean;
  layout: DiffLayout;
  direction: 1 | -1;
  showUnpushed: boolean;
  undo: { can: boolean; reason?: string };
  revert: { can: boolean; reason?: string };
  agentName: string;
  onSelectFile: (path: string) => void;
  onUndo: () => void;
  onRevert: () => void;
  onAsk: (kind: "explain" | "review") => void;
  onCopy: (text: string, what: string) => void;
}

/** History's main area: one commit's message and actions, its files, and a read-only diff. */
export const GitCommitView = memo(function GitCommitView(props: GitCommitViewProps) {
  const { commit } = props;
  const reduce = useReducedMotion();
  const askRef = useRef<HTMLButtonElement>(null);
  const [askOpen, setAskOpen] = useState(false);

  if (!commit) {
    return <div className="git-empty quiet">{props.loading ? <span className="panel-loading" aria-hidden="true" /> : <span>Select a commit to see what it changed.</span>}</div>;
  }

  const authored = new Date(commit.authoredAt);
  const when = Number.isNaN(authored.getTime()) ? commit.authoredAt : authored.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

  const action = (label: string, icon: "undo" | "rewind", state: { can: boolean; reason?: string }, onClick: () => void) => {
    const button = <button type="button" className="secondary-button compact" disabled={!state.can} onClick={onClick}><Icon name={icon} /> {label}</button>;
    return state.reason && !state.can ? <Tooltip label={state.reason}><span className="change-action-wrap">{button}</span></Tooltip> : button;
  };

  return (
    <div className="git-commit-view">
      <AnimatePresence mode="wait" initial={false}>
        <motion.section
          key={commit.sha}
          className="git-commit-card"
          initial={reduce ? false : { opacity: 0, y: 8 * props.direction }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 * props.direction }}
          transition={{ duration: reduce ? 0 : 0.18, ease: EASE }}
        >
          <div className="git-commit-head">
            <h2>{commit.subject || "(no message)"}</h2>
            <div className="git-commit-actions">
              {action("Undo", "undo", props.undo, props.onUndo)}
              {action("Revert…", "rewind", props.revert, props.onRevert)}
              <button ref={askRef} type="button" className="secondary-button compact" aria-haspopup="menu" aria-expanded={askOpen} onClick={() => setAskOpen((value) => !value)}>
                <Icon name="spark" /> Ask {props.agentName}
                <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
              </button>
              <Popover anchor={askRef} open={askOpen} onClose={() => setAskOpen(false)} align="end">
                <Menu onClose={() => setAskOpen(false)} items={[
                  { label: "Explain this commit", icon: <Icon name="book" />, onSelect: () => props.onAsk("explain") },
                  { label: "Review this commit", icon: <Icon name="agents" />, onSelect: () => props.onAsk("review") }
                ]} />
              </Popover>
            </div>
          </div>
          {commit.body && <p className="git-commit-body">{commit.body}</p>}
          <div className="git-commit-facts">
            <button type="button" className="git-sha" title="Copy the full SHA" onClick={() => props.onCopy(commit.sha, "SHA")}>
              <Icon name="commit" /><code>{commit.shortSha}</code><Icon name="copy" className="git-sha-copy" />
            </button>
            <span title={commit.authorEmail}>{commit.authorName}</span>
            <span>{when}</span>
            {commit.parents.length > 1 && <em className="diff-tag">merge · shown against its first parent</em>}
            {props.showUnpushed && commit.unpushed && <em className="git-unpushed-tag"><Icon name="push" /> Not pushed</em>}
          </div>
        </motion.section>
      </AnimatePresence>
      <div className="git-commit-body-split">
        <ul className="git-commit-files" aria-label="Files in this commit">
          {props.files?.map((file) => {
            const { dir, base } = splitPath(file.path);
            const active = file.path === props.selectedPath;
            return (
              <li key={file.path} className={active ? "active" : ""}>
                <button type="button" className="git-file" aria-current={active ? "true" : undefined} title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path} onClick={() => props.onSelectFile(file.path)}>
                  <span className={"status-letter " + file.status}>{file.status[0]?.toUpperCase()}</span>
                  <span className="change-path">{dir && <span className="change-dir">{dir}</span>}<span className="change-base">{base}</span></span>
                  {file.binary ? <em className="diff-tag">bin</em> : (
                    <span className="change-stats">
                      {(file.additions ?? 0) > 0 && <em className="stat-add">+{file.additions}</em>}
                      {(file.deletions ?? 0) > 0 && <em className="stat-del">−{file.deletions}</em>}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
          {props.truncated && <li className="git-sidebar-empty">Only the first 5,000 files are listed.</li>}
          {props.files && props.files.length === 0 && <li className="git-sidebar-empty">This commit changed no files.</li>}
        </ul>
        <div className="git-diff-well">
          {props.diff && props.diff.path === props.selectedPath ? (
            <DiffView file={props.diff} sections={props.diff.sections} layout={props.layout} direction={props.direction} readOnly disabled />
          ) : (
            <div className="git-empty quiet">{props.loading && <span className="panel-loading" aria-hidden="true" />}</div>
          )}
        </div>
      </div>
    </div>
  );
});
