/**
 * The right-hand side panel shows one view at a time: the chat's Git changes, browser,
 * terminal, project Run output, or one sub-agent's transcript. Views are this union; adding one means a member
 * here, a view component rendered in `SidePanel`, and a trigger that opens it (`toggleView`).
 *
 * Changes, Terminal and Run are durable: which of them is showing is remembered across launches and
 * chats (`wackcode:sidePanel`; terminal shows the open chat's own shell, spawning lazily on
 * first attach; Run only attaches to an explicitly launched command). Browser and sub-agent views belong to one chat and fall back to that remembered
 * state when the chat changes or the call leaves the conversation.
 */
export type SidePanelView =
  | { kind: "changes" }
  | { kind: "browser"; taskId: string }
  | { kind: "terminal" }
  | { kind: "run" }
  | { kind: "subagent"; taskId: string; toolCallId: string; index: number };

export const CHANGES_VIEW: SidePanelView = { kind: "changes" };
export const RUN_VIEW: SidePanelView = { kind: "run" };
export const TERMINAL_VIEW: SidePanelView = { kind: "terminal" };

/** The views the panel can remember showing, in their header order. */
export type PanelViewKind = "changes" | "terminal" | "run";
const PANEL_ORDER: Record<PanelViewKind, number> = { run: 0, changes: 1, terminal: 2 };

export function durableView(kind: PanelViewKind): SidePanelView {
  return kind === "run" ? RUN_VIEW : kind === "terminal" ? TERMINAL_VIEW : CHANGES_VIEW;
}

/** Stable per view: keys the panel's swap animation and tells two views apart. */
export function viewKey(view: SidePanelView): string {
  if (view.kind === "subagent") return `subagent:${view.taskId}:${view.toolCallId}#${view.index}`;
  if (view.kind === "browser") return `browser:${view.taskId}`;
  return view.kind;
}

/**
 * Views that animate in as a new page. A call's children share one: moving between sibling tabs
 * slides only the transcript (`swapDirection`), and the tabs stay put.
 */
export function swapKey(view: SidePanelView): string {
  return view.kind === "subagent" ? `subagent:${view.taskId}:${view.toolCallId}` : view.kind;
}

/** Which way a change of view slides: 1 enters from the right, -1 from the left. */
export function swapDirection(from: SidePanelView | null, to: SidePanelView | null): 1 | -1 {
  if (from?.kind === "subagent" && to?.kind === "subagent" && swapKey(from) === swapKey(to)) {
    return to.index < from.index ? -1 : 1;
  }
  if (from && to && (from.kind === "changes" || from.kind === "terminal" || from.kind === "run") && (to.kind === "changes" || to.kind === "terminal" || to.kind === "run")) {
    return PANEL_ORDER[to.kind] < PANEL_ORDER[from.kind] ? -1 : 1;
  }
  return from?.kind === "subagent" && to?.kind !== "subagent" ? -1 : 1;
}

export function sameView(a: SidePanelView | null, b: SidePanelView | null): boolean {
  return a === null || b === null ? a === b : viewKey(a) === viewKey(b);
}

/** A trigger replaces the current view, or closes the panel when its view is already showing. */
export function toggleView(current: SidePanelView | null, next: SidePanelView): SidePanelView | null {
  return sameView(current, next) ? null : next;
}

/**
 * What the remembered durable view becomes: its kind for Changes/Terminal/Run, null when the panel
 * closes, and undefined while a task-bound Browser or sub-agent view is showing.
 */
export function rememberedView(view: SidePanelView | null): PanelViewKind | null | undefined {
  if (view === null) return null;
  return view.kind === "changes" || view.kind === "terminal" || view.kind === "run" ? view.kind : undefined;
}

/** The view to show in `taskId`'s chat: task-bound views fall back to the remembered durable one. */
export function viewForChat(view: SidePanelView | null, taskId: string | undefined, remembered: PanelViewKind | null): SidePanelView | null {
  if (view?.kind === "subagent" || view?.kind === "browser") {
    return view.taskId === taskId ? view : remembered ? durableView(remembered) : null;
  }
  return view;
}
