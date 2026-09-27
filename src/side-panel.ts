/**
 * The right-hand side panel shows one view at a time: the chat's Git changes, browser, or one
 * sub-agent's transcript. Views are this union; adding one means a member here, a view
 * component rendered in `SidePanel`, and a trigger that opens it (`toggleView`).
 *
 * Only Changes is durable. Whether it is open is remembered across launches and chats
 * (`wackcode:changesOpen`); browser and sub-agent views belong to one chat and fall back to
 * that remembered state when the chat changes or the call leaves the conversation.
 */
export type SidePanelView =
  | { kind: "changes" }
  | { kind: "browser"; taskId: string }
  | { kind: "subagent"; taskId: string; toolCallId: string; index: number };

export const CHANGES_VIEW: SidePanelView = { kind: "changes" };

/** Stable per view: keys the panel's swap animation and tells two views apart. */
export function viewKey(view: SidePanelView): string {
  return view.kind === "subagent" ? `subagent:${view.taskId}:${view.toolCallId}#${view.index}` : view.kind === "browser" ? `browser:${view.taskId}` : view.kind;
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
 * A later sibling comes from the right and an earlier one from the left; returning from a
 * sub-agent to Changes reads as going back.
 */
export function swapDirection(from: SidePanelView | null, to: SidePanelView | null): 1 | -1 {
  if (from?.kind === "subagent" && to?.kind === "subagent" && swapKey(from) === swapKey(to)) return to.index < from.index ? -1 : 1;
  return from?.kind === "subagent" && to?.kind === "changes" ? -1 : 1;
}

export function sameView(a: SidePanelView | null, b: SidePanelView | null): boolean {
  return a === null || b === null ? a === b : viewKey(a) === viewKey(b);
}

/** A trigger's click: its view replaces whatever is showing, or closes the panel if it is the one showing. */
export function toggleView(current: SidePanelView | null, next: SidePanelView): SidePanelView | null {
  return sameView(current, next) ? null : next;
}

/** What the remembered Changes state becomes when `view` shows; undefined leaves it as it was. */
export function rememberedChanges(view: SidePanelView | null): boolean | undefined {
  if (view === null) return false;
  return view.kind === "changes" ? true : undefined;
}

/** The view to show in `taskId`'s chat: the current one if it can stay, else the remembered one. */
export function viewForChat(view: SidePanelView | null, taskId: string | undefined, changesOpen: boolean): SidePanelView | null {
  if (view?.kind === "subagent" || view?.kind === "browser") return view.taskId === taskId ? view : changesOpen ? CHANGES_VIEW : null;
  return view;
}
