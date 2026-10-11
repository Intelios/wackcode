import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { orderProjects } from "../project-display";
import { areaDirection, type Area } from "../areas";
import { formatRelativeTime } from "../chat-utils";
import { groupByRecency, matchingChats, newestChats } from "../sidebar-utils";
import type { ProjectRecord, TaskRecord } from "../types";
import { Icon } from "./Icons";
import { ArchivedList } from "./ArchivedList";
import { AreaSwitch } from "./AreaSwitch";
import { chatMenu } from "./chat-menu";
import { useContextMenu } from "./ui/ContextMenu";
import type { MenuEntry } from "./ui/Menu";
import { MenuButton } from "./ui/MenuButton";
import { Tooltip } from "./ui/Tooltip";
import { DevBadge } from "./DevBadge";
import { useConfirmAction } from "./ui/useConfirmAction";
import { TextSwap } from "./TextSwap";

export type TaskAction = "rename" | "worktree" | "fork" | "reveal" | "copy" | "archive" | "unarchive" | "delete" | "delete-direct";
export type ProjectAction = "reveal" | "remove" | "pin" | "unpin";

/** Collapse-key for the "No project" group, which has no ProjectRecord id. */
export const NO_PROJECT_KEY = "__no_project__";

const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

type SidebarPage = "chats" | "archived";

interface SidebarProps {
  projects: ProjectRecord[];
  /** Pinned projects float to the top. */
  pinnedProjectIds: ReadonlySet<string>;
  /** The area on screen. Code lists chats by project; Chat lists them by when they were last used. */
  area: Area;
  /** The other area has a chat waiting on an answer. */
  areaAttention?: Area;
  onSwitchArea: (area: Area) => void;
  /** This area's chats only. */
  tasks: TaskRecord[];
  selectedTaskId?: string;
  /** While open, the Archived view replaces the chat list; the footer tile and the header ✕ toggle it. */
  archivedOpen: boolean;
  /** Task IDs that have a pending extension dialog waiting for a response. */
  pendingDialogTaskIds: ReadonlySet<string>;
  /** Per-chat nonce bumping when the title model names it: the row's name crossfades. */
  titlePulses: Record<string, number>;
  /** Group keys (project ids or NO_PROJECT_KEY) whose chats are hidden. */
  collapsedProjectIds: ReadonlySet<string>;
  onSelectTask: (id: string) => void;
  onNewChat: (project: ProjectRecord | null) => void;
  onNewDraft: () => void;
  onAddProject: () => void;
  onToggleArchived: () => void;
  onToggleProjectCollapsed: (key: string) => void;
  onOpenSettings: () => void;
  onTaskAction: (task: TaskRecord, action: TaskAction) => void;
  onProjectAction: (project: ProjectRecord, action: ProjectAction) => void;
  onRenameTask: (taskId: string, name: string) => void;
  /** Archive every open chat in one group: a project id, or null for "No project". */
  onArchiveAll: (projectId: string | null) => void;
  onDeleteAllArchived: () => void;
  /** Dev build: the DEV badge sits in the title bar. */
  devBuild?: boolean;
}

/** Pages slide by their order (chats, archived): forward enters from the right. A change
 *  of area slides the same way, by the areas' order in the switch. */
const pageVariants = {
  enter: ({ direction, reduce }: { direction: 1 | -1; reduce: boolean }) => (reduce ? { opacity: 0 } : { opacity: 0, x: 16 * direction }),
  center: { opacity: 1, x: 0 },
  exit: ({ direction, reduce }: { direction: 1 | -1; reduce: boolean }) => (reduce ? { opacity: 0 } : { opacity: 0, x: -16 * direction })
};

export function Sidebar({ projects, pinnedProjectIds, area, areaAttention, onSwitchArea, tasks, selectedTaskId, archivedOpen, pendingDialogTaskIds, titlePulses, collapsedProjectIds, onSelectTask, onNewChat, onNewDraft, onAddProject, onToggleArchived, onToggleProjectCollapsed, onOpenSettings, onTaskAction, onProjectAction, onRenameTask, onArchiveAll, onDeleteAllArchived, devBuild }: SidebarProps) {
  const contextMenu = useContextMenu();
  const [renamingId, setRenamingId] = useState<string>();
  const [renameValue, setRenameValue] = useState("");
  const [query, setQuery] = useState("");
  const searchInput = useRef<HTMLInputElement>(null);
  const [now, setNow] = useState(() => new Date());
  // Relative timestamps stay current even while the app is idle.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, []);
  const { confirming, confirm, setConfirming } = useConfirmAction();
  const reduce = useReducedMotion() ?? false;
  const page: SidebarPage = archivedOpen ? "archived" : "chats";
  // The direction is fixed when the page changes and held through the swap: App re-renders
  // often, and recomputing it mid-animation would send the incoming page in from the wrong side.
  const swap = useRef<{ page: SidebarPage; area: Area; direction: 1 | -1 }>({ page, area, direction: 1 });
  if (swap.current.area !== area) swap.current = { page, area, direction: areaDirection(swap.current.area, area) };
  else if (swap.current.page !== page) swap.current = { page, area, direction: page === "archived" ? 1 : -1 };
  const direction = swap.current.direction;
  const archivedTasks = tasks.filter((task) => task.archived);
  const hasArchived = archivedTasks.length > 0;
  const searching = query.trim().length > 0;
  const matchingTasks = useMemo(() => matchingChats(tasks, projects, query), [tasks, projects, query]);
  const openTasks = useMemo(() => newestChats(matchingTasks.filter((task) => !task.archived)), [matchingTasks]);
  // Archived chats never appear inline; they live in the Archived view only.
  const looseTasks = openTasks.filter((task) => task.projectId === null);
  const recencyGroups = useMemo(() => area === "chat" ? groupByRecency(openTasks, now) : [], [area, openTasks, now]);
  const isCollapsed = (key: string) => !searching && collapsedProjectIds.has(key);

  function startRename(task: TaskRecord) {
    setConfirming(null);
    setRenamingId(task.id);
    setRenameValue(task.name);
  }

  function commitRename(taskId: string) {
    const name = renameValue.trim();
    setRenamingId(undefined);
    if (name) onRenameTask(taskId, name);
  }

  function handleArchiveClick(event: React.MouseEvent, task: TaskRecord) {
    event.stopPropagation();
    const action: TaskAction = task.archived ? "unarchive" : "archive";
    if (confirm(task.id, action)) onTaskAction(task, action);
  }

  function handleDeleteClick(event: React.MouseEvent, task: TaskRecord) {
    event.stopPropagation();
    if (confirm(task.id, "delete")) onTaskAction(task, "delete-direct");
  }

  function renderTask(task: TaskRecord) {
    const isConfirmingArchive = confirming?.taskId === task.id && (confirming.action === "archive" || confirming.action === "unarchive");
    const isConfirmingDelete = confirming?.taskId === task.id && confirming.action === "delete";
    // The Chat area orders by last use, so that is the time its rows show.
    const stamp = area === "chat" ? task.lastActivityAt ?? task.createdAt : task.createdAt;
    const age = formatRelativeTime(stamp, now);
    const createdLabel = age ? `${area === "chat" && task.lastActivityAt ? "Last active" : "Created"} ${new Date(stamp).toLocaleString()}` : undefined;

    return renamingId === task.id ? (
      <div className="task-item renaming" key={task.id}>
        <input
          autoFocus
          aria-label="Chat name"
          value={renameValue}
          onChange={(event) => setRenameValue(event.target.value)}
          onBlur={() => commitRename(task.id)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commitRename(task.id);
            if (event.key === "Escape") setRenamingId(undefined);
          }}
        />
      </div>
    ) : (
      <div
        key={task.id}
        role="button"
        aria-label={task.name}
        tabIndex={0}
        className={`task-item ${selectedTaskId === task.id ? "active" : ""}`}
        onClick={() => onSelectTask(task.id)}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          // Swallow the keypress or it leaks into the chat that opens — its terminal takes it
          // as an extra prompt line.
          event.preventDefault();
          onSelectTask(task.id);
        }}
        onDoubleClick={() => startRename(task)}
        onContextMenu={(event) => contextMenu(event, chatMenu(task, projects.find((project) => project.id === task.projectId), () => startRename(task), onTaskAction), "Chat menu")}
      >
        <span className={`task-status ${task.lastError ? "error" : task.status}`} />
        <span className="task-details">
          <span className="task-label">
            <span className="task-name"><TextSwap text={task.name} swapKey={titlePulses[task.id] ?? 0} variant="row" /></span>
            {pendingDialogTaskIds.has(task.id) && <span className="sidebar-question-dot" title="Waiting for your answer" />}
          </span>
          <span className="task-meta">
            {age && (
              <time dateTime={stamp} title={createdLabel} aria-label={createdLabel}>
                {age}
              </time>
            )}
            {task.mode === "plan" && <span className="task-mode-chip">Plan</span>}
            {task.mode === "ultraplan" && <span className="task-mode-chip ultra">Ultra Plan</span>}
            {task.usesWorktree && <Icon name="branch" className="task-branch-icon" />}
          </span>
        </span>
        <span className="task-actions" onClick={(event) => event.stopPropagation()} onDoubleClick={(event) => event.stopPropagation()}>
          <Tooltip label="Archive chat" disabled={Boolean(isConfirmingArchive)}>
            <button
              type="button"
              className={`ghost-button row-menu${isConfirmingArchive ? " confirming task-confirming" : ""}`}
              onClick={(event) => handleArchiveClick(event, task)}
              aria-label={isConfirmingArchive ? `Confirm archive ${task.name}` : `Archive ${task.name}`}
            >
              {isConfirmingArchive ? (
                <span className="confirm-label">Archive?</span>
              ) : (
                <span className="row-menu-icon"><Icon name="archive" /></span>
              )}
            </button>
          </Tooltip>

          <Tooltip label="Delete chat" disabled={Boolean(isConfirmingDelete)}>
            <button
              type="button"
              className={`ghost-button row-menu danger${isConfirmingDelete ? " confirming task-confirming" : ""}`}
              onClick={(event) => handleDeleteClick(event, task)}
              aria-label={isConfirmingDelete ? `Confirm delete ${task.name}` : `Delete ${task.name}`}
            >
              {isConfirmingDelete ? (
                <span className="confirm-label">Delete?</span>
              ) : (
                <span className="row-menu-icon"><Icon name="trash" /></span>
              )}
            </button>
          </Tooltip>
        </span>
      </div>
    );
  }

  function renderHeading({ name, groupKey, plusLabel, title, groupTasks, pinned, onPlus, menu }: {
    name: string;
    groupKey: string;
    plusLabel: string;
    title?: string;
    groupTasks: TaskRecord[];
    pinned?: boolean;
    onPlus: () => void;
    menu?: MenuEntry[];
  }) {
    const collapsed = isCollapsed(groupKey);
    return (
      <div className="project-heading" onContextMenu={(event) => contextMenu(event, menu ?? [], "Project menu")} title={title} onClick={() => { if (!searching) onToggleProjectCollapsed(groupKey); }}>
        <button
          type="button"
          className="ghost-button project-chevron"
          aria-expanded={!collapsed}
          disabled={searching}
          aria-label={collapsed ? `Expand ${name}` : `Collapse ${name}`}
          onClick={(event) => { event.stopPropagation(); onToggleProjectCollapsed(groupKey); }}
        >
          <Icon name="chevron" />
        </button>
        <span className="project-name">{name}</span>
        {pinned && <Icon name="pin" className="project-pin" aria-label="Pinned" />}
        {collapsed && groupTasks.length > 0 && <span className="project-count">{groupTasks.length}</span>}
        {collapsed && groupTasks.some((task) => pendingDialogTaskIds.has(task.id)) && (
          <span className="sidebar-question-dot" title="A chat in this group is waiting for your answer" />
        )}
        <span className="project-actions" onClick={(event) => event.stopPropagation()}>
          {menu && <MenuButton className="ghost-button" label={`${name} menu`} items={menu} />}
          <Tooltip label={plusLabel}>
            <button type="button" className="ghost-button" onClick={onPlus} aria-label={plusLabel}>
              <Icon name="plus" />
            </button>
          </Tooltip>
        </span>
      </div>
    );
  }

  // Chat mode has no projects: one flat list, most recently used first, in calendar groups.
  const recencyList = (
    <>
      {searching && openTasks.length === 0 && (
        <div className="sidebar-empty" role="status">No chats match your search.</div>
      )}
      {!searching && openTasks.length === 0 && (
        <div className="sidebar-empty">Ask anything. A chat here has no project: just a model, a few tools and a scratchpad.</div>
      )}
      {recencyGroups.map((group) => (
        <section className="project-group open recency-group" key={group.key} aria-label={group.label}>
          <h2 className="recency-heading">{group.label}</h2>
          {group.tasks.map((task) => renderTask(task))}
        </section>
      ))}
    </>
  );

  const chatList = area === "chat" ? recencyList : (
    <>
      {searching && openTasks.length === 0 && (
        <div className="sidebar-empty" role="status">No chats match your search.</div>
      )}
      {!searching && projects.length === 0 && looseTasks.length === 0 && (
        <div className="sidebar-empty">Start a new chat — with a project folder or without one.</div>
      )}
      {orderProjects(projects, pinnedProjectIds).map((project) => {
        const projectTasks = openTasks.filter((task) => task.projectId === project.id);
        if (searching && projectTasks.length === 0) return null;
        const pinned = pinnedProjectIds.has(project.id);
        return (
          <motion.section
            layout={reduce ? false : "position"}
            transition={{ type: "spring", stiffness: 420, damping: 36 }}
            className={`project-group ${isCollapsed(project.id) ? "" : "open"}${pinned ? " pinned" : ""}`}
            key={project.id}
          >
            {renderHeading({
              name: project.name,
              groupKey: project.id,
              plusLabel: `New chat in ${project.name}`,
              title: project.path,
              groupTasks: projectTasks,
              pinned,
              onPlus: () => onNewChat(project),
              menu: [
                { label: "New chat", icon: <Icon name="plus" />, onSelect: () => onNewChat(project) },
                { label: "Archive all chats", icon: <Icon name="archive" />, disabled: projectTasks.length === 0, onSelect: () => onArchiveAll(project.id) },
                { label: pinned ? "Unpin project" : "Pin project", icon: <Icon name="pin" />, onSelect: () => onProjectAction(project, pinned ? "unpin" : "pin") },
                "separator",
                { label: "Reveal in Finder", icon: <Icon name="folder" />, onSelect: () => onProjectAction(project, "reveal") },
                { label: "Remove project", icon: <Icon name="trash" />, danger: true, onSelect: () => onProjectAction(project, "remove") }
              ]
            })}
            {!isCollapsed(project.id) && projectTasks.map((task) => renderTask(task))}
          </motion.section>
        );
      })}
      {looseTasks.length > 0 && (
        <section className={`project-group ${isCollapsed(NO_PROJECT_KEY) ? "" : "open"}`}>
          {renderHeading({
            name: "No project",
            groupKey: NO_PROJECT_KEY,
            plusLabel: "New chat with no project",
            groupTasks: looseTasks,
            onPlus: () => onNewChat(null),
            menu: [
              { label: "New chat", icon: <Icon name="plus" />, onSelect: () => onNewChat(null) },
              { label: "Archive all chats", icon: <Icon name="archive" />, onSelect: () => onArchiveAll(null) }
            ]
          })}
          {!isCollapsed(NO_PROJECT_KEY) && looseTasks.map((task) => renderTask(task))}
        </section>
      )}
    </>
  );

  return (
    <aside className="sidebar">
      <div className="titlebar-drag" data-tauri-drag-region>{devBuild && <DevBadge />}</div>
      <div className="sidebar-top">
        <AreaSwitch area={area} attention={areaAttention} onSwitch={onSwitchArea} />
        {archivedOpen ? (
          <div className="archived-header">
            <h2 className="archived-heading">Archived</h2>
            {archivedTasks.length > 0 && <span className="archived-count">{archivedTasks.length}</span>}
            <span className="archived-header-actions">
              {archivedTasks.length > 0 && (
                <Tooltip label="Delete all archived">
                  <button type="button" className="ghost-button row-menu danger" onClick={onDeleteAllArchived} aria-label="Delete all archived chats">
                    <span className="row-menu-icon"><Icon name="trash" /></span>
                  </button>
                </Tooltip>
              )}
              <Tooltip label="Back to chats">
                <button type="button" className="ghost-button" onClick={onToggleArchived} aria-label="Close archived chats">
                  <Icon name="close" />
                </button>
              </Tooltip>
            </span>
          </div>
        ) : (
          <button type="button" className="sidebar-action" onClick={onNewDraft}>
            <Icon name="plus" /> New chat <kbd>⌘N</kbd>
          </button>
        )}
        <div className="sidebar-search">
          <Icon name="search" />
          <input
            ref={searchInput}
            type="search"
            aria-label={archivedOpen ? "Search archived chats" : "Search chats"}
            placeholder="Search chats…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape" && query) {
                event.preventDefault();
                event.stopPropagation();
                setQuery("");
              }
            }}
          />
          {query && (
            <button type="button" className="ghost-button" aria-label="Clear chat search" onClick={() => { setQuery(""); searchInput.current?.focus(); }}>
              <Icon name="close" />
            </button>
          )}
        </div>
      </div>
      <nav className="project-list" aria-label={archivedOpen ? "Archived chats" : area === "chat" ? "Chats" : "Projects and chats"}>
        <AnimatePresence initial={false} mode="wait" custom={{ direction, reduce }}>
          <motion.div
            key={`${area}:${page}`}
            className="sidebar-page"
            custom={{ direction, reduce }}
            variants={pageVariants}
            initial="enter"
            animate="center"
            exit="exit"
            transition={{ duration: 0.16, ease: EASE }}
          >
            {page === "archived" ? (
              <ArchivedList
                tasks={matchingTasks}
                projects={projects}
                searching={searching}
                now={now}
                selectedTaskId={selectedTaskId}
                onSelectTask={onSelectTask}
                onTaskAction={onTaskAction}
              />
            ) : chatList}
          </motion.div>
        </AnimatePresence>
      </nav>
      <div className="sidebar-footer">
        {/* Projects belong to the Code area. */}
        {area === "code" && (
          <Tooltip label={<>Add project <kbd>⌘O</kbd></>}>
            <button type="button" className="sidebar-tile" onClick={onAddProject} aria-label="Add project">
              <Icon name="folder" />
            </button>
          </Tooltip>
        )}
        {hasArchived && (
          <Tooltip label={archivedOpen ? "Hide archived" : "Show archived"}>
            <button
              type="button"
              className={`sidebar-tile${archivedOpen ? " active" : ""}`}
              onClick={onToggleArchived}
              aria-label={archivedOpen ? "Hide archived" : "Show archived"}
              aria-pressed={archivedOpen}
            >
              <Icon name="archive" />
            </button>
          </Tooltip>
        )}
        <Tooltip label={<>Settings <kbd>⌘,</kbd></>}>
          <button type="button" className="sidebar-tile" onClick={onOpenSettings} aria-label="Settings">
            <Icon name="settings" />
          </button>
        </Tooltip>
      </div>
    </aside>
  );
}
