import { useLayoutEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { orderProjects } from "../project-display";
import { matchesProject, monogram, shortPath } from "../project-display";
import type { ProjectRecord } from "../types";
import { Icon } from "./Icons";
import { Popover } from "./ui/Popover";

/**
 * The new-chat screen's project picker: a trigger with a monogram tile and a searchable popover.
 *
 * Invariants:
 * - Pins are the sidebar's own set (`pinnedProjects` in App); this component only
 *   asks to change them. Pinned projects sort first, via the same `orderProjects`.
 * - ⌘O ("Add project…") is wired globally in App; the footer button just calls `onAddProject`.
 * - The rows live in one flat keyed list (headings inline) so pinning moves a row between groups
 *   without remounting it, and the CSS entrance stagger doesn't replay.
 * - The selection highlight is a single element placed from the active row's `offsetTop`, which
 *   ignores in-flight layout transforms and scrolls with the list.
 * - Hover and the arrow keys share one active row; typing always goes to the search field.
 */

interface ProjectPickerProps {
  projects: ProjectRecord[];
  projectId: string | null;
  pinned: ReadonlySet<string>;
  onSelect: (projectId: string | null) => void;
  onSetPinned: (projectId: string, pinned: boolean) => void;
  onAddProject: () => void;
}

const NONE_KEY = "__none__";
const keyOf = (id: string | null) => id ?? NONE_KEY;

/** Which rows a query shows, in display order: pinned, the rest, then "No project". */
function arrange(projects: ProjectRecord[], pinned: ReadonlySet<string>, query: string) {
  const needle = query.trim().toLowerCase();
  const rows = orderProjects(projects, pinned).filter((project) => matchesProject(project, needle));
  const pinnedRows = rows.filter((project) => pinned.has(project.id));
  const otherRows = rows.filter((project) => !pinned.has(project.id));
  const showNone = needle === "" || "no project".includes(needle);
  const ids: (string | null)[] = [...pinnedRows.map((project) => project.id), ...otherRows.map((project) => project.id), ...(showNone ? [null] : [])];
  return { pinnedRows, otherRows, showNone, ids };
}

const SPRING = { type: "spring", stiffness: 520, damping: 34 } as const;

export function ProjectPicker({ projects, projectId, pinned, onSelect, onSetPinned, onAddProject }: ProjectPickerProps) {
  const reduce = useReducedMotion() ?? false;
  const triggerRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const rowRefs = useRef(new Map<string, HTMLElement>());
  const keyboardNav = useRef(false);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [box, setBox] = useState<{ y: number; h: number } | null>(null);

  const current = projects.find((project) => project.id === projectId);
  const { pinnedRows, otherRows, showNone, ids } = arrange(projects, pinned, query);
  const activeIndex = Math.min(active, ids.length - 1);
  const activeKey = activeIndex >= 0 ? keyOf(ids[activeIndex]) : undefined;
  const idsKey = ids.map(keyOf).join("|");

  function show() {
    const all = arrange(projects, pinned, "").ids;
    setOpen(true); setQuery(""); setBox(null);
    setActive(Math.max(0, all.indexOf(projectId)));
    requestAnimationFrame(() => inputRef.current?.focus());
  }

  function choose(id: string | null) {
    setOpen(false);
    onSelect(id);
  }

  // Place the highlight on the active row (and keep it there as pins reorder the list).
  useLayoutEffect(() => {
    if (!open) return;
    const row = activeKey ? rowRefs.current.get(activeKey) : undefined;
    if (!row) { setBox(null); return; }
    setBox((previous) => previous && previous.y === row.offsetTop && previous.h === row.offsetHeight ? previous : { y: row.offsetTop, h: row.offsetHeight });
    if (keyboardNav.current) {
      keyboardNav.current = false;
      row.scrollIntoView?.({ block: "nearest" });
    }
  }, [open, activeKey, idsKey, pinned]);

  function onKeyDown(event: React.KeyboardEvent) {
    const last = ids.length - 1;
    const move = (index: number) => { event.preventDefault(); keyboardNav.current = true; setActive(Math.max(0, Math.min(last, index))); };
    if (event.key === "ArrowDown") move(activeIndex + 1);
    else if (event.key === "ArrowUp") move(activeIndex - 1);
    else if (event.key === "Home") move(0);
    else if (event.key === "End") move(last);
    else if (event.key === "Enter") {
      event.preventDefault();
      if (activeIndex >= 0) choose(ids[activeIndex]);
    }
  }

  function renderRow(project: ProjectRecord | null) {
    const key = keyOf(project?.id ?? null);
    const index = ids.indexOf(project?.id ?? null);
    const selected = (project?.id ?? null) === projectId;
    const isPinned = project ? pinned.has(project.id) : false;
    return (
      <motion.div
        key={key}
        ref={(element: HTMLDivElement | null) => { if (element) rowRefs.current.set(key, element); else rowRefs.current.delete(key); }}
        className="project-row-wrap"
        layout={reduce ? false : "position"}
        transition={SPRING}
      >
        <div
          className={`project-row ${selected ? "selected" : ""} ${key === activeKey ? "active" : ""} ${isPinned ? "pinned" : ""}`}
          style={{ "--i": index } as React.CSSProperties}
          onMouseEnter={() => setActive(index)}
        >
          <button
            type="button"
            className="project-row-main"
            title={project?.path}
            aria-current={selected ? "true" : undefined}
            onClick={() => choose(project?.id ?? null)}
          >
            {project ? (
              <span className="project-tile">{monogram(project.name) ?? <Icon name="folder" />}</span>
            ) : (
              <span className="project-tile none" />
            )}
            <span className="project-row-text">
              <span className="project-row-name">{project?.name ?? "No project"}</span>
              <span className={project ? "project-row-path" : "project-row-sub"}>{project ? shortPath(project.path) : "Chat without a folder"}</span>
            </span>
            {selected && <CheckMark reduce={reduce} />}
          </button>
          {project && (
            <button
              type="button"
              className="repo-pin project-row-pin"
              aria-label={isPinned ? `Unpin ${project.name}` : `Pin ${project.name}`}
              aria-pressed={isPinned}
              title={isPinned ? "Unpin" : "Pin to top"}
              onClick={() => onSetPinned(project.id, !isPinned)}
            ><Icon name="pin" /></button>
          )}
        </div>
      </motion.div>
    );
  }

  const label = current?.name ?? "No project";
  const tileKey = current?.id ?? NONE_KEY;
  const matchCount = pinnedRows.length + otherRows.length;
  // One flat array: React keeps a keyed row's DOM node when pinning moves it between groups.
  const list: React.ReactNode[] = [
    ...(pinnedRows.length > 0 ? [<div key="h-pinned" className="picker-heading">Pinned</div>] : []),
    ...pinnedRows.map(renderRow),
    ...(otherRows.length > 0 ? [<div key="h-projects" className="picker-heading">Projects</div>] : []),
    ...otherRows.map(renderRow),
    ...(projects.length > 0 && matchCount === 0 ? [<div key="no-match" className="branch-empty">No matching projects</div>] : []),
    ...(showNone && matchCount > 0 ? [<div key="divider" className="branch-divider" />] : []),
    ...(showNone ? [renderRow(null)] : [])
  ];
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="project-trigger"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Project: ${label}. Switch project`}
        onClick={() => (open ? setOpen(false) : show())}
      >
        <span className="project-tile-slot">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.span
              key={tileKey}
              className={`project-tile ${current ? "" : "none"}`}
              initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.55 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.55 }}
              transition={SPRING}
            >{current ? monogram(current.name) ?? <Icon name="folder" /> : null}</motion.span>
          </AnimatePresence>
        </span>
        <span className="project-trigger-name">
          <AnimatePresence initial={false} mode="popLayout">
            <motion.span
              key={label}
              initial={reduce ? { opacity: 0 } : { opacity: 0, y: 9 }}
              animate={{ opacity: 1, y: 0 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, y: -9 }}
              transition={SPRING}
            >{label}</motion.span>
          </AnimatePresence>
        </span>
        <svg className="select-chevron" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>
      <Popover anchor={triggerRef} open={open} onClose={() => setOpen(false)} side="top" align="start" className="project-pop">
        <div className="project-picker" role="dialog" aria-label="Choose project">
          <div className="branch-search">
            <input
              ref={inputRef}
              value={query}
              placeholder="Filter projects…"
              aria-label="Filter projects"
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              onChange={(event) => { setQuery(event.target.value); setActive(0); }}
              onKeyDown={onKeyDown}
            />
          </div>
          <motion.div layoutScroll className="project-picker-list">
            {box && (
              <motion.div
                className="project-highlight"
                aria-hidden="true"
                initial={false}
                animate={{ y: box.y, height: box.h }}
                transition={reduce ? { duration: 0 } : SPRING}
              />
            )}
            {projects.length === 0 && (
              <div key="empty" className="picker-empty">
                <strong>No projects yet</strong>
                <span>Add a folder to give chats a codebase to work in.</span>
              </div>
            )}
            {list}
          </motion.div>
          <div className="project-picker-foot">
            <button type="button" className="project-add" onClick={() => { setOpen(false); onAddProject(); }}>
              <Icon name="plus" /> Add project… <kbd>⌘O</kbd>
            </button>
          </div>
        </div>
      </Popover>
    </>
  );
}

/** The current project's tick: it springs in and its stroke draws. */
function CheckMark({ reduce }: { reduce: boolean }) {
  return (
    <motion.svg
      className="project-check"
      viewBox="0 0 24 24"
      aria-hidden="true"
      initial={reduce ? false : { scale: 0.4, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={{ type: "spring", stiffness: 520, damping: 22 }}
    >
      <motion.path
        d="m4.5 12.5 5 5 10-11"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        initial={reduce ? false : { pathLength: 0 }}
        animate={{ pathLength: 1 }}
        transition={{ duration: 0.28, delay: 0.05 }}
      />
    </motion.svg>
  );
}
