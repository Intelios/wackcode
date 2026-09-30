import { memo, useEffect, useRef, type ReactNode } from "react";
import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import { splitPath } from "../changes-utils";
import { formatRelativeTime } from "../chat-utils";
import { isCommittable, firstLine, selectionState, type GitTab } from "../git-mode";
import type { GitChangeFile, GitCommit } from "../types";
import { Icon } from "./Icons";
import { Checkbox } from "./ui/Checkbox";
import { useContextClipboard, useContextMenu } from "./ui/ContextMenu";
import type { MenuEntry } from "./ui/Menu";
import { MenuButton } from "./ui/MenuButton";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface GitPanelTopProps {
  switcher: ReactNode;
  tab: GitTab;
  changesCount: number;
  /** Bumps on every commit: the History tab pops and a "+1" floats off it. */
  commitPulse: number;
  onTab: (tab: GitTab) => void;
}

/** The sidebar's top in Git mode: the repository switcher over Changes / History tabs. */
export const GitPanelTop = memo(function GitPanelTop({ switcher, tab, changesCount, commitPulse, onTab }: GitPanelTopProps) {
  const reduce = useReducedMotion();
  const tabs: { id: GitTab; label: string; count?: number }[] = [
    { id: "changes", label: "Changes", count: changesCount },
    { id: "history", label: "History" }
  ];
  return (
    <div className="git-top">
      {switcher}
      <div className="git-tabs" role="tablist" aria-label="Git mode view">
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            className={`git-tab ${tab === item.id ? "active" : ""}`}
            onClick={() => onTab(item.id)}
          >
            {tab === item.id && (
              <motion.span layoutId="git-tab-pill" className="git-tab-pill" transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 520, damping: 38 }} />
            )}
            {/* The space keeps the accessible name "Changes 3" rather than "Changes3". */}
            <span className="git-tab-label">{item.label}{item.count !== undefined && item.count > 0 && <>{" "}<span className="git-tab-count">{item.count}</span></>}</span>
            {item.id === "history" && commitPulse > 0 && (
              <motion.span
                key={commitPulse}
                className="git-tab-pop"
                aria-hidden="true"
                initial={reduce ? { opacity: 0 } : { opacity: 1, y: 6, scale: 0.6 }}
                animate={reduce ? { opacity: 0 } : { opacity: [1, 1, 0], y: -16, scale: 1 }}
                transition={{ duration: 0.9, ease: EASE }}
              >+1</motion.span>
            )}
          </button>
        ))}
      </div>
    </div>
  );
});

interface GitChangesListProps {
  files: GitChangeFile[];
  excluded: ReadonlySet<string>;
  selectedPath?: string;
  /** `path` → pending comment count. */
  commentCounts: Map<string, number>;
  disabled: boolean;
  onSelect: (path: string) => void;
  onToggle: (path: string) => void;
  onToggleAll: () => void;
  onDiscard: (file: GitChangeFile) => void;
  onCopyPath: (path: string) => void;
  onReveal: (path: string) => void;
}

function fileStats(file: GitChangeFile) {
  return file.sections.reduce((sum, section) => ({ add: sum.add + section.additions, del: sum.del + section.deletions }), { add: 0, del: 0 });
}

/**
 * Changed files as one flat list, GitHub Desktop style: a checkbox decides what goes in the
 * commit, a click shows the diff. ↑/↓ move the selection; Space ticks the selected file.
 */
export const GitChangesList = memo(function GitChangesList(props: GitChangesListProps) {
  const contextMenu = useContextMenu();
  const { files, excluded } = props;
  const reduce = useReducedMotion();
  const listRef = useRef<HTMLUListElement>(null);
  const state = selectionState(files, excluded);
  const totals = files.reduce((sum, file) => { const stats = fileStats(file); return { add: sum.add + stats.add, del: sum.del + stats.del }; }, { add: 0, del: 0 });
  const selected = files.find((file) => file.path === props.selectedPath) ?? files[0];

  function fileMenu(file: GitChangeFile): MenuEntry[] {
    return [
      { label: "Show diff", icon: <Icon name="code" />, onSelect: () => props.onSelect(file.path) },
      { label: excluded.has(file.path) ? "Include in commit" : "Exclude from commit", icon: <Icon name="check" />, disabled: props.disabled || !isCommittable(file), onSelect: () => props.onToggle(file.path) },
      "separator",
      { label: "Discard changes…", icon: <Icon name="trash" />, danger: true, disabled: props.disabled || file.status === "conflict", onSelect: () => props.onDiscard(file) },
      "separator",
      { label: "Copy path", icon: <Icon name="copy" />, onSelect: () => props.onCopyPath(file.path) },
      { label: "Reveal in Finder", icon: <Icon name="folder" />, disabled: file.status === "deleted", onSelect: () => props.onReveal(file.path) }
    ];
  }

  function onKeyDown(event: React.KeyboardEvent, file: GitChangeFile) {
    const index = files.indexOf(file);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const next = files[Math.max(0, Math.min(files.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)))];
      if (!next) return;
      props.onSelect(next.path);
      requestAnimationFrame(() => listRef.current?.querySelector<HTMLButtonElement>(`[data-path="${CSS.escape(next.path)}"]`)?.focus());
    } else if (event.key === " ") {
      event.preventDefault();
      if (isCommittable(file)) props.onToggle(file.path);
    }
  }

  return (
    <div className="git-changes">
      <div className="git-changes-head">
        <Checkbox
          checked={state === "all"}
          indeterminate={state === "some"}
          disabled={files.length === 0}
          label="Include all files"
          onChange={props.onToggleAll}
        />
        <span className="git-changes-count">{files.length} changed {files.length === 1 ? "file" : "files"}</span>
        {(totals.add > 0 || totals.del > 0) && (
          <span className="change-stats">
            {totals.add > 0 && <em className="stat-add">+{totals.add}</em>}
            {totals.del > 0 && <em className="stat-del">−{totals.del}</em>}
          </span>
        )}
      </div>
      <ul className="git-file-list" ref={listRef} aria-label="Changed files">
        <LayoutGroup>
          <AnimatePresence initial={false}>
            {files.map((file, index) => {
              const { dir, base } = splitPath(file.path);
              const stats = fileStats(file);
              const active = file.path === selected?.path;
              const included = isCommittable(file) && !excluded.has(file.path);
              const comments = props.commentCounts.get(file.path) ?? 0;
              return (
                <motion.li
                  key={file.path}
                  onContextMenu={(event) => contextMenu(event, fileMenu(file), "File menu")}
                  layout="position"
                  className={`${active ? "active" : ""} ${included ? "" : "excluded"}`}
                  initial={reduce ? false : { opacity: 0, y: -8 }}
                  animate={{ opacity: 1, y: 0 }}
                  // Committed rows zip up toward the History tab, one after another.
                  exit={reduce ? { opacity: 0 } : { opacity: 0, x: -10, y: -16, scale: 0.96, transition: { duration: 0.22, delay: Math.min(index, 8) * 0.03, ease: EASE } }}
                  transition={{ duration: reduce ? 0 : 0.2, ease: EASE }}
                >
                  <Checkbox
                    checked={included}
                    disabled={!isCommittable(file)}
                    label={`Include ${file.path} in commit`}
                    tabIndex={-1}
                    onChange={() => props.onToggle(file.path)}
                  />
                  <button
                    type="button"
                    className="git-file"
                    data-path={file.path}
                    aria-current={active ? "true" : undefined}
                    title={file.status === "conflict" ? `${file.path} — resolve the conflict first` : file.path}
                    tabIndex={active ? 0 : -1}
                    onClick={() => props.onSelect(file.path)}
                    onKeyDown={(event) => onKeyDown(event, file)}
                  >
                    <span className={"status-letter " + file.status}>{file.status[0]?.toUpperCase()}</span>
                    <span className="change-path">
                      {dir && <span className="change-dir">{dir}</span>}
                      <span className="change-base">{base}</span>
                    </span>
                    {comments > 0 && <span className="change-comments" title={`${comments} pending ${comments === 1 ? "comment" : "comments"}`}><Icon name="comment" />{comments}</span>}
                    {(stats.add > 0 || stats.del > 0) && (
                      <span className="change-stats">
                        {stats.add > 0 && <em className="stat-add">+{stats.add}</em>}
                        {stats.del > 0 && <em className="stat-del">−{stats.del}</em>}
                      </span>
                    )}
                  </button>
                  <MenuButton
                    className="ghost-button git-file-menu"
                    label={`Actions for ${file.path}`}
                    items={() => fileMenu(file)}
                  />
                </motion.li>
              );
            })}
          </AnimatePresence>
        </LayoutGroup>
      </ul>
    </div>
  );
});

interface GitHistoryListProps {
  commits: GitCommit[];
  hasMore: boolean;
  loading: boolean;
  error?: string;
  /** Only meaningful with a remote: without one, every commit is "unpushed". */
  showUnpushed: boolean;
  selectedSha?: string;
  onSelect: (sha: string) => void;
  onLoadMore: () => void;
}

/** HEAD's history, newest first; scrolling near the end loads the next page. */
export const GitHistoryList = memo(function GitHistoryList(props: GitHistoryListProps) {
  const contextMenu = useContextMenu();
  const clipboard = useContextClipboard();
  const { commits } = props;
  const reduce = useReducedMotion();
  const listRef = useRef<HTMLUListElement>(null);
  const sentinelRef = useRef<HTMLLIElement>(null);
  const onLoadMore = useRef(props.onLoadMore);
  onLoadMore.current = props.onLoadMore;
  const now = new Date();

  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel || !props.hasMore || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) onLoadMore.current();
    }, { rootMargin: "200px" });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [props.hasMore, commits.length]);

  function onKeyDown(event: React.KeyboardEvent, index: number) {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const next = commits[Math.max(0, Math.min(commits.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)))];
    if (!next) return;
    props.onSelect(next.sha);
    requestAnimationFrame(() => listRef.current?.querySelector<HTMLButtonElement>(`[data-sha="${next.sha}"]`)?.focus());
  }

  if (!props.loading && commits.length === 0) {
    return <div className="git-sidebar-empty">{props.error ?? "No commits yet"}</div>;
  }

  return (
    <ul className="git-history-list" ref={listRef} aria-label="Commits">
      {/* The list may hold the previous page: a failed refresh must still say so. */}
      {props.error && <li className="git-history-error" role="alert">{firstLine(props.error)}</li>}
      <AnimatePresence initial={false}>
        {commits.map((commit, index) => {
          const active = commit.sha === props.selectedSha;
          return (
            <motion.li
              key={commit.sha}
              onContextMenu={(event) => contextMenu(event, [
                { label: "Show commit", icon: <Icon name="commit" />, onSelect: () => props.onSelect(commit.sha) },
                "separator",
                { label: "Copy commit ID", icon: <Icon name="copy" />, disabled: !clipboard, onSelect: () => clipboard?.copyText(commit.sha) },
                { label: "Copy commit message", icon: <Icon name="copy" />, disabled: !clipboard, onSelect: () => clipboard?.copyText([commit.subject, commit.body].filter(Boolean).join("\n\n")) }
              ], "Commit menu")}
              layout="position"
              initial={reduce ? false : { opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: reduce ? 0 : 0.22, delay: reduce ? 0 : Math.min(index, 6) * 0.03, ease: EASE }}
              className={active ? "active" : ""}
            >
              <button
                type="button"
                className="git-commit-row"
                data-sha={commit.sha}
                aria-current={active ? "true" : undefined}
                tabIndex={active || (!props.selectedSha && index === 0) ? 0 : -1}
                onClick={() => props.onSelect(commit.sha)}
                onKeyDown={(event) => onKeyDown(event, index)}
              >
                <span className="git-commit-subject">{commit.subject || "(no message)"}</span>
                <span className="git-commit-meta">
                  <span>{commit.authorName}</span>
                  <span>· {formatRelativeTime(commit.authoredAt, now)}</span>
                  {commit.parents.length > 1 && <em className="diff-tag">merge</em>}
                  {props.showUnpushed && commit.unpushed && <em className="git-unpushed" title="Not pushed yet"><Icon name="push" /></em>}
                </span>
              </button>
            </motion.li>
          );
        })}
      </AnimatePresence>
      {props.hasMore && <li ref={sentinelRef} className="git-history-more">{props.loading ? "Loading…" : ""}</li>}
    </ul>
  );
});
