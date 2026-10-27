/**
 * The right-hand side panel holds tabs, one per view: the chat's Git changes, browser, terminal,
 * project Run/Build output, the Games arcade, or one sub-agent call's transcript. Views are this
 * union; adding one means a member here, a view component rendered in `SidePanel`, a tab label in
 * App's `panelTabItems` and a trigger that opens it (`openTab`/`toggleTab`).
 *
 * A tab is identified by `swapKey`, so each kind is a singleton (one browser per chat) and a
 * sub-agent call's siblings share one tab. Hiding the panel keeps its tabs; closing a tab never
 * stops a terminal, run or browser session. Changes, Terminal, Run, Build and Games are durable:
 * which of them are open is remembered across launches and chats (`wackcode:sidePanel`; Run/Build
 * only attach to an explicitly launched command; a game in progress lives in `games/session.ts`).
 * Browser and sub-agent tabs belong to one chat and drop out when the chat changes (`forChat`).
 */
export type SidePanelView =
  | { kind: "changes" }
  | { kind: "browser"; taskId: string }
  | { kind: "terminal" }
  | { kind: "run" }
  | { kind: "build" }
  | { kind: "games" }
  | { kind: "subagent"; taskId: string; toolCallId: string; index: number };

export const CHANGES_VIEW: SidePanelView = { kind: "changes" };
export const RUN_VIEW: SidePanelView = { kind: "run" };
export const BUILD_VIEW: SidePanelView = { kind: "build" };
export const TERMINAL_VIEW: SidePanelView = { kind: "terminal" };
export const GAMES_VIEW: SidePanelView = { kind: "games" };

/** The views the panel can remember showing, in their header order. */
export type PanelViewKind = "changes" | "terminal" | "run" | "build" | "games";
const PANEL_ORDER: Record<PanelViewKind, number> = { build: 0, run: 1, changes: 2, terminal: 3, games: 4 };

export function isDurable(kind: unknown): kind is PanelViewKind {
  return kind === "changes" || kind === "terminal" || kind === "run" || kind === "build" || kind === "games";
}

export function durableView(kind: PanelViewKind): SidePanelView {
  return kind === "build" ? BUILD_VIEW : kind === "run" ? RUN_VIEW : kind === "terminal" ? TERMINAL_VIEW : kind === "games" ? GAMES_VIEW : CHANGES_VIEW;
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
  if (view.kind === "subagent") return `subagent:${view.taskId}:${view.toolCallId}`;
  return view.kind;
}

/** Views bound to one chat, which never show in another. */
export function isTaskBound(view: SidePanelView | null): view is Extract<SidePanelView, { taskId: string }> {
  return view?.kind === "browser" || view?.kind === "subagent";
}

/** Views that can span the main area with the composer docked below (`panelExpanded`): all of them. */
export function canExpand(view: SidePanelView | null): boolean {
  return view !== null;
}

/** Which way a change of view slides: 1 enters from the right, -1 from the left. */
export function swapDirection(from: SidePanelView | null, to: SidePanelView | null): 1 | -1 {
  if (from?.kind === "subagent" && to?.kind === "subagent" && swapKey(from) === swapKey(to)) {
    return to.index < from.index ? -1 : 1;
  }
  if (from && to && isDurable(from.kind) && isDurable(to.kind)) {
    return PANEL_ORDER[to.kind] < PANEL_ORDER[from.kind] ? -1 : 1;
  }
  return from?.kind === "subagent" && to?.kind !== "subagent" ? -1 : 1;
}

export function sameView(a: SidePanelView | null, b: SidePanelView | null): boolean {
  return a === null || b === null ? a === b : viewKey(a) === viewKey(b);
}

/** The panel's tabs: their order, which one is in front, and whether the panel is showing at all. */
export interface PanelTabs {
  tabs: SidePanelView[];
  /** The front tab's `swapKey`. */
  active: string | null;
  open: boolean;
}

export const NO_PANEL_TABS: PanelTabs = { tabs: [], active: null, open: false };

/** The front tab while the panel is showing. */
export function shownTab(state: PanelTabs): SidePanelView | null {
  return state.open ? state.tabs.find((view) => swapKey(view) === state.active) ?? null : null;
}

/** Show `view`: bring its tab to the front (updating it, e.g. to another sibling) or add it after the front tab. */
export function openTab(state: PanelTabs, view: SidePanelView): PanelTabs {
  const key = swapKey(view);
  const at = state.tabs.findIndex((tab) => swapKey(tab) === key);
  if (at >= 0) {
    const tabs = sameView(state.tabs[at], view) ? state.tabs : state.tabs.map((tab, index) => index === at ? view : tab);
    return { tabs, active: key, open: true };
  }
  const front = state.tabs.findIndex((tab) => swapKey(tab) === state.active);
  const tabs = front < 0 ? [...state.tabs, view] : [...state.tabs.slice(0, front + 1), view, ...state.tabs.slice(front + 1)];
  return { tabs, active: key, open: true };
}

/** A trigger hides the panel when its view is already in front, and shows it otherwise. */
export function toggleTab(state: PanelTabs, view: SidePanelView): PanelTabs {
  return state.open && sameView(shownTab(state), view) ? { ...state, open: false } : openTab(state, view);
}

/** Close one tab. The front moves to its right-hand neighbour (else the left); the last one hides the panel. */
export function closeTab(state: PanelTabs, key: string): PanelTabs {
  const at = state.tabs.findIndex((tab) => swapKey(tab) === key);
  if (at < 0) return state;
  const tabs = state.tabs.filter((_, index) => index !== at);
  if (tabs.length === 0) return NO_PANEL_TABS;
  if (state.active !== key) return { ...state, tabs };
  return { ...state, tabs, active: swapKey(tabs[Math.min(at, tabs.length - 1)]) };
}

export function hidePanel(state: PanelTabs): PanelTabs {
  return state.open ? { ...state, open: false } : state;
}

/** Show the panel again on its front tab, or on `fallback` when it has none. */
export function showPanel(state: PanelTabs, fallback: SidePanelView): PanelTabs {
  return state.tabs.length > 0 && state.active ? { ...state, open: true } : openTab(state, fallback);
}

/** Change the tab `key` holds in place (a sub-agent call's sibling), without bringing it forward. */
export function updateTab(state: PanelTabs, key: string, change: (view: SidePanelView) => SidePanelView): PanelTabs {
  return { ...state, tabs: state.tabs.map((tab) => swapKey(tab) === key ? change(tab) : tab) };
}

/** Another chat is selected: its own browser and sub-agent tabs stay, any other chat's drop. */
export function forChat(state: PanelTabs, taskId: string | undefined): PanelTabs {
  let next = state;
  for (const tab of state.tabs) {
    if (isTaskBound(tab) && tab.taskId !== taskId) next = closeTab(next, swapKey(tab));
  }
  return next;
}

/** What `wackcode:sidePanel` keeps: the durable tabs only. */
export interface StoredPanelTabs { tabs: PanelViewKind[]; active: PanelViewKind | null; open: boolean }

export function storedTabs(state: PanelTabs): StoredPanelTabs {
  const tabs = state.tabs.map((view) => view.kind).filter(isDurable);
  const active = isDurable(state.active) ? state.active : tabs[0] ?? null;
  return { tabs, active, open: state.open && active !== null };
}

/**
 * Bring stored tabs back. Before tabs, the key held one durable kind (or null for closed), and
 * before that a Changes flag (`legacyChangesOpen`).
 */
export function restoreTabs(stored: unknown, legacyChangesOpen: boolean): PanelTabs {
  if (isDurable(stored)) return openTab(NO_PANEL_TABS, durableView(stored));
  if (stored === null) return NO_PANEL_TABS;
  if (stored && typeof stored === "object" && Array.isArray((stored as StoredPanelTabs).tabs)) {
    const { tabs: kinds, active, open } = stored as StoredPanelTabs;
    const tabs = [...new Set(kinds.filter(isDurable))].map(durableView);
    if (tabs.length === 0) return NO_PANEL_TABS;
    const front = isDurable(active) && kinds.includes(active) ? active : swapKey(tabs[0]);
    return { tabs, active: front, open: open === true };
  }
  return legacyChangesOpen ? openTab(NO_PANEL_TABS, CHANGES_VIEW) : NO_PANEL_TABS;
}
