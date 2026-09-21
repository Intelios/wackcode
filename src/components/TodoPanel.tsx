import { useEffect, useRef, useState } from "react";
import type { TodoTask } from "../types";
import { Icon } from "./Icons";

interface Props {
  tasks?: TodoTask[];
  /** A run is in progress — completed tasks fold away at the start of each run. */
  busy?: boolean;
}

const GLYPHS: Record<TodoTask["status"], string> = {
  pending: "○",
  in_progress: "◐",
  completed: "✓",
  deleted: "✗"
};

/**
 * The live todo list, pinned above the composer. Renders the state the built-in todo
 * extension publishes over `todo_state` — the desktop stand-in for the TUI overlay in
 * `@juicesharp/rpiv-todo` v2.11.0 (MIT) `todo-overlay.ts`: the same row anatomy (status
 * glyph, activeForm in parentheses, `chains #dep` suffix, tree prefixes, `#id` prefix only
 * when dependencies exist) and the same completed-task fading — a finished task stays
 * visible for the turn it finished in, then folds behind the "+N done" summary at the next
 * run. Upstream's row budget becomes a max-height with scroll, and the collapse hotkey
 * becomes the chevron button. The panel hides itself when the list is empty.
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
          <span className="todo-panel-title">
            <span aria-hidden="true">{active ? "●" : "○"}</span>
            {`Todos (${doneCount}/${total})`}
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
          <ul className="todo-panel-list">
            {shown.map((task, index) => (
              <li key={task.id} className={`todo-row ${task.status}`}>
                <span className="todo-tree" aria-hidden="true">{index === shown.length - 1 ? "└─" : "├─"}</span>
                <span className="todo-glyph" aria-hidden="true">{GLYPHS[task.status]}</span>
                {showIds && <code className="todo-id">{`#${task.id}`}</code>}
                <span className="todo-subject" title={task.subject}>{task.subject}</span>
                {task.status === "in_progress" && task.activeForm && (
                  <span className="todo-active-form">{`(${task.activeForm})`}</span>
                )}
                {task.blockedBy && task.blockedBy.length > 0 && (
                  <span className="todo-blocks">{`⛓ ${task.blockedBy.map((id) => `#${id}`).join(",")}`}</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
