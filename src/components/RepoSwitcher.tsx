import { useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { orderProjects } from "../git-mode";
import type { ProjectRecord } from "../types";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

interface RepoSwitcherProps {
  projects: ProjectRecord[];
  currentId: string;
  /** The current project's branch, shown under its name. */
  branch: string | null;
  pinned: ReadonlySet<string>;
  onSelect: (projectId: string) => void;
  onSetPinned: (projectId: string, pinned: boolean) => void;
  onAddProject: () => void;
}

/**
 * Git mode's "Current repository": the project name with a searchable list of every project,
 * pinned ones first (GitHub Desktop has no way to pin; this is the fix). Projects that aren't
 * Git repositories stay listed but dimmed.
 */
export function RepoSwitcher({ projects, currentId, branch, pinned, onSelect, onSetPinned, onAddProject }: RepoSwitcherProps) {
  const reduce = useReducedMotion();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const current = projects.find((project) => project.id === currentId);
  const needle = query.trim().toLowerCase();
  const rows = orderProjects(projects, pinned).filter((project) => !needle || project.name.toLowerCase().includes(needle) || project.path.toLowerCase().includes(needle));
  const pinnedRows = rows.filter((project) => pinned.has(project.id));
  const otherRows = rows.filter((project) => !pinned.has(project.id));
  const activeId = rows[Math.min(active, rows.length - 1)]?.id;

  function show() {
    setOpen(true); setQuery(""); setActive(Math.max(0, orderProjects(projects, pinned).findIndex((project) => project.id === currentId)));
    requestAnimationFrame(() => inputRef.current?.focus());
  }

  function choose(projectId: string) {
    setOpen(false);
    onSelect(projectId);
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((index) => Math.max(0, Math.min(rows.length - 1, Math.min(index, rows.length - 1) + step)));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (activeId) choose(activeId);
    }
  }

  function renderRow(project: ProjectRecord) {
    const isPinned = pinned.has(project.id);
    const isCurrent = project.id === currentId;
    return (
      <div
        key={project.id}
        className={`repo-row ${isCurrent ? "selected" : ""} ${project.id === activeId ? "active" : ""} ${project.gitRoot ? "" : "not-git"} ${isPinned ? "pinned" : ""}`}
        onMouseEnter={() => setActive(rows.findIndex((row) => row.id === project.id))}
      >
        <button type="button" className="repo-row-main" title={project.path} onClick={() => choose(project.id)}>
          <Icon name={project.gitRoot ? "git" : "folder"} />
          <span className="repo-row-name">{project.name}</span>
          {!project.gitRoot && <em className="branch-hint">not Git</em>}
          {isCurrent && <Icon name="check" className="repo-row-check" />}
        </button>
        <button
          type="button"
          className="repo-pin"
          aria-label={isPinned ? `Unpin ${project.name}` : `Pin ${project.name}`}
          aria-pressed={isPinned}
          title={isPinned ? "Unpin" : "Pin to top"}
          onClick={() => onSetPinned(project.id, !isPinned)}
        ><Icon name="pin" /></button>
      </div>
    );
  }

  const label = current?.name ?? "Choose a repository";
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="repo-trigger"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Current repository: ${label}. Switch repository`}
        onClick={() => (open ? setOpen(false) : show())}
      >
        <span className="repo-trigger-caption">Current repository</span>
        <span className="repo-trigger-name">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.span
              key={label}
              initial={reduce ? { opacity: 0 } : { opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, y: -10 }}
              transition={{ type: "spring", stiffness: 520, damping: 34 }}
            >{label}</motion.span>
          </AnimatePresence>
          {current && pinned.has(current.id) && <Icon name="pin" className="repo-trigger-pin" />}
        </span>
        {branch && <span className="repo-trigger-branch"><Icon name="branch" />{branch}</span>}
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="bottom" align="start" matchWidth className="repo-pop">
        <div className="branch-picker repo-picker" role="dialog" aria-label="Switch repository">
          <div className="branch-search">
            <input
              ref={inputRef}
              value={query}
              placeholder="Filter repositories…"
              aria-label="Filter repositories"
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              onChange={(event) => { setQuery(event.target.value); setActive(0); }}
              onKeyDown={onKeyDown}
            />
          </div>
          <div className="branch-list">
            {pinnedRows.length > 0 && <div className="picker-heading">Pinned</div>}
            {pinnedRows.map(renderRow)}
            {otherRows.length > 0 && <div className="picker-heading">{pinnedRows.length > 0 ? "Projects" : "Repositories"}</div>}
            {otherRows.map(renderRow)}
            {rows.length === 0 && <div className="branch-empty">No matching projects</div>}
          </div>
          <div className="repo-picker-foot">
            <button type="button" className="repo-add" onClick={() => { setOpen(false); onAddProject(); }}>
              <Icon name="plus" /> Add project… <kbd>⌘O</kbd>
            </button>
          </div>
        </div>
      </Popover>
    </>
  );
}
