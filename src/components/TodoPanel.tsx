import { useEffect, useRef, useState, type ReactNode } from "react";
import type { TodoTask } from "../types";
import { Icon } from "./Icons";
import { GlassPlatter } from "./ui/GlassPlatter";
import { Popover } from "./ui/Popover";

interface Props {
  tasks?: TodoTask[];
  /** A run is in progress — completed tasks fold away at the start of each run. */
  busy?: boolean;
  /** Liquid Glass platter on the pill and its popover (Classic Liquid Glass composer). */
  glass?: boolean;
}

const STATUS_LABEL: Record<TodoTask["status"], string> = {
  pending: "pending",
  in_progress: "in progress",
  completed: "completed",
  deleted: "deleted"
};

interface TodoView {
  /** Non-deleted tasks, in order. */
  visible: TodoTask[];
  /** Rows actually listed: `visible` minus completions folded behind "+N done". */
  shown: TodoTask[];
  /** Completed tasks currently hidden behind the reveal button. */
  hiddenDone: TodoTask[];
  doneCount: number;
  /** Any incomplete task remains (drives the head's live dot and the all-done state). */
  active: boolean;
  /** Show the `#id` prefix on every row once dependencies exist anywhere. */
  showIds: boolean;
  showDone: boolean;
  setShowDone: (value: boolean) => void;
}

/**
 * The list's state, shared by the pinned panel and the pill's popover: completed tasks stay
 * visible for the run they finished in, then fold behind "+N done" when the next run starts.
 */
function useTodoView(tasks: TodoTask[] | undefined, busy: boolean | undefined): TodoView {
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

  return {
    visible,
    shown,
    hiddenDone,
    doneCount: completed.length,
    active: visible.some((task) => task.status !== "completed"),
    showIds: visible.some((task) => task.blockedBy && task.blockedBy.length > 0),
    showDone,
    setShowDone
  };
}

/** One row of the tree: status mark, optional `#id`, subject and the dependency hint. */
function TodoRows({ view }: { view: TodoView }) {
  return (
    <ul className="todo-panel-list" aria-live="polite">
      {view.shown.map((task) => (
        <li key={task.id} className={`todo-row ${task.status}`} aria-label={`${task.subject}, ${STATUS_LABEL[task.status]}`}>
          <span className="todo-mark" aria-hidden="true">
            {task.status === "completed"
              ? <Icon name="check" className="todo-check" />
              : <span className={`todo-mark-circle ${task.status}`} />}
          </span>
          {view.showIds && <code className="todo-id">{`#${task.id}`}</code>}
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
  );
}

/** The list's header line: root dot, "Todos (done/total)" and the "+N done" reveal. */
function TodoHead({ view, busy, children }: { view: TodoView; busy?: boolean; children?: ReactNode }) {
  return (
    <div className="todo-panel-head">
      <span className={`todo-root-dot ${busy && view.active ? "live" : ""}`} aria-hidden="true" />
      <span className="todo-panel-title">
        {"Todos "}
        <span className="todo-panel-count">{`(${view.doneCount}/${view.visible.length})`}</span>
      </span>
      {view.hiddenDone.length > 0 && (
        <button type="button" className="todo-panel-more" onClick={() => view.setShowDone(true)}>
          {`+${view.hiddenDone.length} done`}
        </button>
      )}
      {children}
    </div>
  );
}

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
 *
 * Under the Classic composer styles (`appearance.composerStyle` classic/classicGlass) App
 * replaces this panel with `TodoPill` inside `.composer-pills`; the other styles keep it.
 */
export function TodoPanel({ tasks, busy }: Props) {
  const [collapsed, setCollapsed] = useState(false);
  const view = useTodoView(tasks, busy);

  if (view.visible.length === 0) return null;

  return (
    <div className="todo-panel-wrap">
      <section className={`todo-panel ${view.active ? "" : "done"} ${collapsed ? "collapsed" : ""}`} aria-label="Todos">
        <TodoHead view={view} busy={busy}>
          <button
            type="button"
            className="todo-panel-collapse"
            aria-label={collapsed ? "Expand todos" : "Collapse todos"}
            onClick={() => setCollapsed((value) => !value)}
          >
            <Icon name="chevron" />
          </button>
        </TodoHead>
        {!collapsed && <TodoRows view={view} />}
      </section>
    </div>
  );
}

/**
 * The Classic composer's todo affordance: a floating pill beside the "Working for" timer that
 * compresses the list to its progress and the task in hand, and opens the real tree in a
 * popover above it. The pill's left mark mirrors the row markers: a half-filled ring while a
 * task runs, a plain ring while tasks wait, a drawn check once all are done.
 */
export function TodoPill({ tasks, busy, glass = false }: Props) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const view = useTodoView(tasks, busy);

  const total = view.visible.length;
  if (total === 0) return null;

  const current = view.visible.find((task) => task.status === "in_progress") ?? view.visible.find((task) => task.status === "pending");
  const done = view.doneCount === total;
  const label = done ? "All tasks complete" : current?.subject ?? "";
  const summary = `${view.doneCount} of ${total} tasks done${label ? `, ${label}` : ""}`;

  return (
    <>
      <button
        ref={anchor}
        type="button"
        className={`composer-pill composer-pill-todo${open ? " open" : ""}`}
        aria-expanded={open}
        aria-label={`Todos: ${summary}`}
        onClick={() => setOpen((value) => !value)}
      >
        {glass && <GlassPlatter />}
        <span className="todo-mark" aria-hidden="true">
          {done
            ? <Icon name="check" className="todo-check" />
            : <span className={`todo-mark-circle ${current?.status === "in_progress" ? "in_progress" : "pending"}`} />}
        </span>
        <strong>{`${view.doneCount}/${total}`}</strong>
        <span className="composer-pill-detail"><span>{label}</span></span>
        <Icon name="chevron" className="todo-pill-chevron" />
      </button>
      <Popover anchor={anchor} open={open} onClose={() => setOpen(false)} side="top" align="center" glass={glass} className="todo-popover">
        <section className={`todo-panel todo-popover-panel ${view.active ? "" : "done"}`} role="dialog" aria-label="Todos">
          <TodoHead view={view} busy={busy}>
            <button type="button" className="todo-panel-collapse" aria-label="Close todos" onClick={() => setOpen(false)}>
              <Icon name="chevron" />
            </button>
          </TodoHead>
          <TodoRows view={view} />
        </section>
      </Popover>
    </>
  );
}
