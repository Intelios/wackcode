import { useEffect, useRef, useState } from "react";
import type { TodoTask } from "../types";
import { Icon } from "./Icons";

interface Props {
  tasks?: TodoTask[];
  /** A run is in progress — completed tasks fold away at the start of each run. */
  busy?: boolean;
}

const STATUS_LABEL: Record<TodoTask["status"], string> = {
  pending: "pending",
  in_progress: "in progress",
  completed: "completed",
  deleted: "deleted"
};

/**
 * The live todo list, pinned above the composer. Renders the state the built-in todo
 * extension publishes over `todo_state` — the desktop stand-in for the TUI overlay in
 * `@juicesharp/rpiv-todo` v2.11.0 (MIT) `todo-overlay.ts`. Where the TUI draws box-drawing
 * characters, the panel draws the tree for real: a vertical rail grows from the header's
 * root dot through per-row `::before` segments, each row branches to its status marker
 * (a ring for pending, a half-filled accent ring while it runs, a drawn check when done),
 * and the rail fills with the accent behind completed rows so the tree itself shows
 * progress. Behaviour still mirrors the overlay: the activeForm in parentheses, the
 * `link #dep` suffix, the `#id` prefix only when dependencies exist, and the
 * completed-task folding — a finished task stays visible for the turn it finished in,
 * then folds behind the "+N done" summary at the next run. Upstream's row budget is a
 * max-height with scroll, and the collapse hotkey is the chevron button. The panel hides
 * itself when the list is empty.
 */
export function TodoPanel({ tasks, busy }: Props) {
  const [collapsed, setCollapsed] = useState(false);
  const [showDone, setShowDone] = useState(false);
  const [faded, setFaded] = useState<ReadonlySet<number>>(() => new Set());
  const wasBusy = useRef(false);

  const visible = (tasks ?? []).filter((task) => task.status !== "deleted");

  useEffect(() => {
    if (busy && !wasBusy.current) {
      setFaded(new Set(visible.filter((task) => task.status === "completed").map((task) => task.id)));
      setShowDone(false);
    }
    wasBusy.current = busy === true;
    // `visible` is derived from `tasks`; both are listed so a run starting together with
    // fresh tasks sees the freshest list at the fold boundary.
  }, [busy, tasks, visible]);

  const completed = visible.filter((task) => task.status === "completed");
  const hiddenDone = completed.filter((task) => faded.has(task.id) && !showDone);
  const shown = visible.filter((task) => !hiddenDone.includes(task));
  const doneCount = completed.length;
  const total = visible.length;
  const active = visible.some((task) => task.status !== "completed");
  const showIds = visible.some((task) => task.blockedBy && task.blockedBy.length > 0);

  if (total === 0) return null;

  return (
    <div className="todo-panel-wrap">
      <section className={`todo-panel ${active ? "" : "done"} ${collapsed ? "collapsed" : ""}`} aria-label="Todos">
        <div className="todo-panel-head">
          <span className={`todo-root-dot ${busy && active ? "live" : ""}`} aria-hidden="true" />
          <span className="todo-panel-title">
            {"Todos "}
            <span className="todo-panel-count">{`(${doneCount}/${total})`}</span>
          </span>
          {hiddenDone.length > 0 && (
            <button type="button" className="todo-panel-more" onClick={() => setShowDone(true)}>
              {`+${hiddenDone.length} done`}
            </button>
          )}
          <button
            type="button"
            className="todo-panel-collapse"
            aria-label={collapsed ? "Expand todos" : "Collapse todos"}
            onClick={() => setCollapsed((value) => !value)}
          >
            <Icon name="chevron" />
          </button>
        </div>
        {!collapsed && (
          <ul className="todo-panel-list" aria-live="polite">
            {shown.map((task) => (
              <li key={task.id} className={`todo-row ${task.status}`} aria-label={`${task.subject}, ${STATUS_LABEL[task.status]}`}>
                <span className="todo-mark" aria-hidden="true">
                  {task.status === "completed"
                    ? <Icon name="check" className="todo-check" />
                    : <span className={`todo-mark-circle ${task.status}`} />}
                </span>
                {showIds && <code className="todo-id">{`#${task.id}`}</code>}
                <span className="todo-subject" title={task.subject}>{task.subject}</span>
                {task.status === "in_progress" && task.activeForm && (
                  <span className="todo-active-form">{`(${task.activeForm})`}</span>
                )}
                {task.blockedBy && task.blockedBy.length > 0 && (
                  <span
                    className="todo-blocks"
                    title={`Blocked by ${task.blockedBy.map((id) => `#${id}`).join(", ")}`}
                    aria-label={`Blocked by ${task.blockedBy.map((id) => `#${id}`).join(", ")}`}
                  >
                    <Icon name="link" />
                    {task.blockedBy.map((id) => `#${id}`).join(", ")}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
