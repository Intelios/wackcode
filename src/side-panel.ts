/**
 * The right-hand side panel shows one view at a time: the chat's Git changes, its terminal,
 * or one sub-agent's transcript. Views are this union; adding one means a member here, a view
 * component rendered in `SidePanel`, and a trigger that opens it (`toggleView`).
 *
 * Changes and Terminal are durable: which of them is showing is remembered across launches and
 * chats (`wackcode:sidePanel`; terminal shows the open chat's own shell, spawning lazily on
 * first attach). A sub-agent view belongs to one chat and falls back to that remembered state
 * when the chat changes or the call leaves the conversation.
 */
export type SidePanelView =
  | { kind: "changes" }
  | { kind: "terminal" }
  | { kind: "subagent"; taskId: string; toolCallId: string; index: number };

export const CHANGES_VIEW: SidePanelView = { kind: "changes" };
export const TERMINAL_VIEW: SidePanelView = { kind: "terminal" };

/** The views the panel can remember showing — the header's two buttons, in its order. */
export type PanelViewKind = "changes" | "terminal";
const PANEL_ORDER: Record<PanelViewKind, number> = { changes: 0, terminal: 1 };

export function durableView(kind: PanelViewKind): SidePanelView {
  return kind === "terminal" ? TERMINAL_VIEW : CHANGES_VIEW;
}

/** Stable per view: keys the panel's swap animation and tells two views apart. */
export function viewKey(view: SidePanelView): string {
  return view.kind === "subagent" ? `subagent:${view.taskId}:${view.toolCallId}#${view.index}` : view.kind;
}

/**
 * Views that animate in as a new page. A call's children share one: moving between sibling tabs
 * slides only the transcript (`swapDirection`), and the tabs stay put.
 */
export function swapKey(view: SidePanelView): string {
  return view.kind === "subagent" ? `subagent:${view.taskId}:${view.toolCallId}` : view.kind;
}

/**
 * Which way a change of view slides: 1 brings the new view in from the right, -1 from the left.
 * Durable views follow the header order (Changes left of Terminal); coming back to either from
 * a sub-agent reads as going back.
 */
export function swapDirection(from: SidePanelView | null, to: SidePanelView | null): 1 | -1 {
  if (from?.kind === "subagent" && to?.kind === "subagent" && swapKey(from) === swapKey(to)) return to.index < from.index ? -1 : 1;
  if (from && to && from.kind !== "subagent" && to.kind !== "subagent") return PANEL_ORDER[to.kind] < PANEL_ORDER[from.kind] ? -1 : 1;
  return from?.kind === "subagent" ? -1 : 1;
}

export function sameView(a: SidePanelView | null, b: SidePanelView | null): boolean {
  return a === null || b === null ? a === b : viewKey(a) === viewKey(b);
}

/** A trigger's click: its view replaces whatever is showing, or closes the panel if it is the one showing. */
export function toggleView(current: SidePanelView | null, next: SidePanelView): SidePanelView | null {
  return sameView(current, next) ? null : next;
}

/**
 * What the remembered durable view becomes when `view` shows: its kind for Changes/Terminal,
 * `null` when the panel closes, `undefined` (leave as it was) while a sub-agent is showing.
 */
export function rememberedView(view: SidePanelView | null): PanelViewKind | null | undefined {
  if (view === null) return null;
  return view.kind === "subagent" ? undefined : view.kind;
}

/** The view to show in `taskId`'s chat: the current one if it can stay, else the remembered one. */
export function viewForChat(view: SidePanelView | null, taskId: string | undefined, remembered: PanelViewKind | null): SidePanelView | null {
  if (view?.kind === "subagent") return view.taskId === taskId ? view : remembered ? durableView(remembered) : null;
  return view;
}
