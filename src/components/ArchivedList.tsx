import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ProjectRecord, TaskRecord } from "../types";
import { formatRelativeTime, sortedArchived } from "../chat-utils";
import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";
import { useConfirmAction } from "./ui/useConfirmAction";

/** Row actions in the Archived view: restore the chat to the normal list, or remove it for good. */
export type ArchivedTaskAction = "unarchive" | "delete-direct";

const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface ArchivedListProps {
  tasks: TaskRecord[];
  projects: ProjectRecord[];
  selectedTaskId?: string;
  onSelectTask: (id: string) => void;
  onTaskAction: (task: TaskRecord, action: ArchivedTaskAction) => void;
}

/**
 * The dedicated Archived view: every archived chat across projects, newest-archived first.
 * Two-line rows — title plus how long it has been archived, then its project with the
 * unarchive and delete actions always visible, since this list exists to manage them.
 */
export function ArchivedList({ tasks, projects, selectedTaskId, onSelectTask, onTaskAction }: ArchivedListProps) {
  const { confirming, confirm } = useConfirmAction();
  const reduce = useReducedMotion();
  const archived = sortedArchived(tasks);

  function handleAction(event: React.MouseEvent, task: TaskRecord, action: "unarchive" | "delete") {
    event.stopPropagation();
    // The delete confirm is armed under "delete"; the dispatch is the direct, no-modal variant.
    if (confirm(task.id, action)) onTaskAction(task, action === "delete" ? "delete-direct" : "unarchive");
  }

  if (archived.length === 0) {
    return <div className="sidebar-empty">No archived chats.</div>;
  }

  return (
    <div className="archived-list">
      <AnimatePresence initial={false}>
        {archived.map((task, index) => {
          const isConfirmingUnarchive = confirming?.taskId === task.id && confirming.action === "unarchive";
          const isConfirmingDelete = confirming?.taskId === task.id && confirming.action === "delete";
          const projectName = task.projectId === null
            ? "No project"
            : projects.find((project) => project.id === task.projectId)?.name ?? "No project";
          return (
            <motion.div
              key={task.id}
              className={`archived-row${selectedTaskId === task.id ? " active" : ""}`}
              role="button"
              tabIndex={0}
              onClick={() => onSelectTask(task.id)}
              onKeyDown={(event) => { if (event.key === "Enter") onSelectTask(task.id); }}
              initial={reduce ? false : { opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, height: 0, marginTop: 0, marginBottom: 0, overflow: "hidden" }}
              transition={{ duration: 0.22, ease: EASE, delay: reduce ? 0 : Math.min(index * 0.025, 0.15) }}
            >
              <div className="archived-row-top">
                <span className="archived-title">{task.name}</span>
                {/* Chats archived before `archivedAt` existed fall back to `updatedAt`, which archive also stamped. */}
                <span className="archived-time">{formatRelativeTime(task.archivedAt ?? task.updatedAt)}</span>
              </div>
              <div className="archived-row-bottom">
                <span className="archived-project"><Icon name="folder" />{projectName}</span>
                <span className="archived-actions" onClick={(event) => event.stopPropagation()}>
                  <Tooltip label="Unarchive chat" disabled={Boolean(isConfirmingUnarchive)}>
                    <button
                      type="button"
                      className={`ghost-button row-menu${isConfirmingUnarchive ? " confirming task-confirming" : ""}`}
                      onClick={(event) => handleAction(event, task, "unarchive")}
                      aria-label={isConfirmingUnarchive ? `Confirm unarchive ${task.name}` : `Unarchive ${task.name}`}
                    >
                      {isConfirmingUnarchive ? (
                        <span className="confirm-label">Unarchive?</span>
                      ) : (
                        <span className="row-menu-icon"><Icon name="unarchive" /></span>
                      )}
                    </button>
                  </Tooltip>
                  <Tooltip label="Delete chat" disabled={Boolean(isConfirmingDelete)}>
                    <button
                      type="button"
                      className={`ghost-button row-menu danger${isConfirmingDelete ? " confirming task-confirming" : ""}`}
                      onClick={(event) => handleAction(event, task, "delete")}
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
            </motion.div>
          );
        })}
      </AnimatePresence>
    </div>
  );
}
