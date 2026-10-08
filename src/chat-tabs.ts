import type { Area } from "./areas";
import type { ModelChoice } from "./model-utils";
import type { SidePanelView } from "./side-panel";
import type { TaskMode } from "./types";

/** Session-only workspace. A tab's identity and composer key survive draft → chat,
 * including a send completing after the tab was closed. Closing never stops work.
 * Closed records share the same update path so recovery gets the latest draft.
 *
 * One store holds both areas' tabs, each tagged with its `area`. The bar shows one area at a
 * time, `activeId` is always that area's open tab, and `parked` remembers the other area's so a
 * switch comes back to it. Everything positional (close, cycle, reorder, reopen) works within
 * the tab's own area; everything id-targeted (a send finishing, a chat binding) works across
 * both, which is why this is one store and not two.
 *
 * A draft's kind is its tab's area. It is deliberately not a `ChatDraft` field: draft edits
 * rebuild that object field by field and would drop it. */
export interface ChatDraft {
  projectId: string | null;
  useWorktree: boolean;
  choice?: ModelChoice;
  mode?: TaskMode;
}

export interface TabPanelState {
  view: SidePanelView | null;
  width: number;
  browserExpanded: boolean;
  browserRestoreWidth: number;
  changesSelection?: { path: string; layer: "staged" | "working" };
}

export const EMPTY_TAB_PANEL: TabPanelState = {
  view: null, width: 430, browserExpanded: false, browserRestoreWidth: 430
};

export interface ChatTab {
  id: string;
  area: Area;
  composerKey: string;
  taskId?: string;
  draft?: ChatDraft;
  panel: TabPanelState;
  completed?: boolean;
  sending?: boolean;
  error?: string;
}

export interface ClosedTab { tab: ChatTab; index: number }
export interface ChatTabsState {
  tabs: ChatTab[];
  closed: ClosedTab[];
  activeId?: string;
  /** The tab each area had open when it was last left. */
  parked?: Partial<Record<Area, string>>;
}
export const EMPTY_CHAT_TABS: ChatTabsState = { tabs: [], closed: [] };

export function draftTab(id: string, draft: ChatDraft, composerKey = `new:${id}`, area: Area = "code"): ChatTab {
  return { id, area, composerKey, draft, panel: { ...EMPTY_TAB_PANEL } };
}

export function tabsInArea(state: ChatTabsState, area: Area): ChatTab[] {
  return state.tabs.filter((tab) => tab.area === area);
}

/** Leave `from` for `to`: park the open tab and bring back the one `to` was left on. */
export function switchTabArea(state: ChatTabsState, from: Area, to: Area): ChatTabsState {
  if (from === to) return state;
  const parked = { ...state.parked, [from]: state.activeId };
  const candidates = tabsInArea(state, to);
  const restored = candidates.find((tab) => tab.id === parked[to]) ?? candidates.at(-1);
  const next = { ...state, parked, activeId: undefined };
  return restored ? activateTab(next, restored.id) : next;
}

export function activateTab(state: ChatTabsState, id: string): ChatTabsState {
  if (!state.tabs.some((tab) => tab.id === id)) return state;
  return { ...state, activeId: id, tabs: state.tabs.map((tab) => tab.id === id && tab.completed ? { ...tab, completed: false } : tab) };
}

export function openChatTab(state: ChatTabsState, taskId: string, select = true, composerKey?: string, area: Area = "code"): ChatTabsState {
  const existing = state.tabs.find((tab) => tab.taskId === taskId);
  if (existing) return select ? activateTab(state, existing.id) : state;
  const recovered = state.closed.find((entry) => entry.tab.taskId === taskId);
  const tab = recovered?.tab ?? { id: `chat:${taskId}`, area, composerKey: composerKey ?? `task:${taskId}`, taskId, panel: { ...EMPTY_TAB_PANEL } };
  const next = { ...state, tabs: [...state.tabs, tab], closed: state.closed.filter((entry) => entry.tab.taskId !== taskId) };
  return select ? activateTab(next, tab.id) : next;
}

export function addDraftTab(state: ChatTabsState, tab: ChatTab): ChatTabsState {
  return { ...state, tabs: [...state.tabs, tab], activeId: tab.id };
}

export function patchTab(state: ChatTabsState, id: string, patch: Partial<ChatTab>): ChatTabsState {
  return { ...state,
    tabs: state.tabs.map((tab) => tab.id === id ? { ...tab, ...patch } : tab),
    closed: state.closed.map((entry) => entry.tab.id === id ? { ...entry, tab: { ...entry.tab, ...patch } } : entry)
  };
}

export function findTab(state: ChatTabsState, id: string): ChatTab | undefined {
  return state.tabs.find((tab) => tab.id === id) ?? state.closed.find((entry) => entry.tab.id === id)?.tab;
}

export function bindTab(state: ChatTabsState, id: string, taskId: string): ChatTabsState {
  // A just-created chat can also have been opened from the sidebar during dispatch.
  const duplicate = state.tabs.find((tab) => tab.taskId === taskId && tab.id !== id);
  const bound = patchTab(state, id, { taskId, draft: undefined });
  // An explicit sidebar opening must stay selected if its originating draft was
  // already closed (or evicted). A completion cannot select a recovery-only id.
  if (!duplicate || !bound.tabs.some((tab) => tab.id === id)) return bound;
  return { ...bound, tabs: bound.tabs.filter((tab) => tab.id !== duplicate.id),
    activeId: bound.activeId === duplicate.id ? id : bound.activeId };
}

export function closeTab(state: ChatTabsState, id: string): ChatTabsState {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  if (index < 0) return state;
  const tab = state.tabs[index];
  const tabs = state.tabs.filter((item) => item.id !== id);
  // The neighbour that takes over is the next one in the same area's bar.
  const position = tabsInArea(state, tab.area).findIndex((item) => item.id === id);
  const siblings = tabs.filter((item) => item.area === tab.area);
  return { ...state, tabs, closed: [{ tab, index }, ...state.closed.filter((entry) => entry.tab.id !== id)].slice(0, 10),
    activeId: state.activeId === id ? (siblings[position] ?? siblings[position - 1])?.id : state.activeId };
}

/** Reopen the most recently closed tab, of `area` when one is given. */
export function reopenTab(state: ChatTabsState, area?: Area): ChatTabsState {
  const entry = state.closed.find((candidate) => !area || candidate.tab.area === area);
  if (!entry) return state;
  const closed = state.closed.filter((candidate) => candidate !== entry);
  const duplicate = state.tabs.find((tab) => tab.id === entry.tab.id || (entry.tab.taskId && tab.taskId === entry.tab.taskId));
  if (duplicate) return activateTab({ ...state, closed }, duplicate.id);
  const tabs = [...state.tabs];
  tabs.splice(Math.min(entry.index, tabs.length), 0, entry.tab);
  return activateTab({ ...state, tabs, closed }, entry.tab.id);
}

/** Move a tab to `index` within its own area's bar; the other area's tabs keep their places. */
export function reorderTab(state: ChatTabsState, id: string, index: number): ChatTabsState {
  const tab = state.tabs.find((item) => item.id === id);
  if (!tab) return state;
  const bar = state.tabs.filter((item) => item.area === tab.area && item.id !== id);
  bar.splice(Math.max(0, Math.min(index, bar.length)), 0, tab);
  let slot = 0;
  return { ...state, tabs: state.tabs.map((item) => item.area === tab.area ? bar[slot++] : item) };
}

export function cycleTab(state: ChatTabsState, direction: number): ChatTabsState {
  const active = state.tabs.find((tab) => tab.id === state.activeId);
  const bar = active ? tabsInArea(state, active.area) : state.tabs;
  if (!bar.length) return state;
  const index = bar.findIndex((tab) => tab.id === state.activeId);
  return activateTab(state, bar[(Math.max(index, 0) + direction + bar.length) % bar.length].id);
}

export function removeTaskTabs(state: ChatTabsState, ids: Set<string>, deleted: boolean): ChatTabsState {
  let next = state;
  for (const tab of state.tabs) if (tab.taskId && ids.has(tab.taskId)) next = closeTab(next, tab.id);
  return deleted ? { ...next, closed: next.closed.filter((entry) => !entry.tab.taskId || !ids.has(entry.tab.taskId)) } : next;
}

export function removeDraftProject(state: ChatTabsState, projectId: string): ChatTabsState {
  const reconcile = (tab: ChatTab): ChatTab => tab.draft?.projectId === projectId
    ? { ...tab, draft: { ...tab.draft, projectId: null, useWorktree: false } } : tab;
  return { ...state, tabs: state.tabs.map(reconcile), closed: state.closed.map((entry) => ({ ...entry, tab: reconcile(entry.tab) })) };
}
