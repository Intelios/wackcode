import type { ModelChoice } from "./model-utils";
import type { SidePanelView } from "./side-panel";
import type { TaskMode } from "./types";

/** Session-only workspace. A tab's identity and composer key survive draft → chat,
 * including a send completing after the tab was closed. Closing never stops work.
 * Closed records share the same update path so recovery gets the latest draft. */
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
  composerKey: string;
  taskId?: string;
  draft?: ChatDraft;
  panel: TabPanelState;
  completed?: boolean;
  sending?: boolean;
  error?: string;
}

export interface ClosedTab { tab: ChatTab; index: number }
export interface ChatTabsState { tabs: ChatTab[]; closed: ClosedTab[]; activeId?: string }
export const EMPTY_CHAT_TABS: ChatTabsState = { tabs: [], closed: [] };

export function draftTab(id: string, draft: ChatDraft, composerKey = `new:${id}`): ChatTab {
  return { id, composerKey, draft, panel: { ...EMPTY_TAB_PANEL } };
}

export function activateTab(state: ChatTabsState, id: string): ChatTabsState {
  if (!state.tabs.some((tab) => tab.id === id)) return state;
  return { ...state, activeId: id, tabs: state.tabs.map((tab) => tab.id === id && tab.completed ? { ...tab, completed: false } : tab) };
}

export function openChatTab(state: ChatTabsState, taskId: string, select = true, composerKey?: string): ChatTabsState {
  const existing = state.tabs.find((tab) => tab.taskId === taskId);
  if (existing) return select ? activateTab(state, existing.id) : state;
  const recovered = state.closed.find((entry) => entry.tab.taskId === taskId);
  const tab = recovered?.tab ?? { id: `chat:${taskId}`, composerKey: composerKey ?? `task:${taskId}`, taskId, panel: { ...EMPTY_TAB_PANEL } };
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
  return { tabs, closed: [{ tab, index }, ...state.closed.filter((entry) => entry.tab.id !== id)].slice(0, 10),
    activeId: state.activeId === id ? (tabs[index] ?? tabs[index - 1])?.id : state.activeId };
}

export function reopenTab(state: ChatTabsState): ChatTabsState {
  const [entry, ...closed] = state.closed;
  if (!entry) return state;
  const duplicate = state.tabs.find((tab) => tab.id === entry.tab.id || (entry.tab.taskId && tab.taskId === entry.tab.taskId));
  if (duplicate) return activateTab({ ...state, closed }, duplicate.id);
  const tabs = [...state.tabs];
  tabs.splice(Math.min(entry.index, tabs.length), 0, entry.tab);
  return activateTab({ ...state, tabs, closed }, entry.tab.id);
}

export function reorderTab(state: ChatTabsState, id: string, index: number): ChatTabsState {
  const tab = state.tabs.find((item) => item.id === id);
  if (!tab) return state;
  const tabs = state.tabs.filter((item) => item.id !== id);
  tabs.splice(Math.max(0, Math.min(index, tabs.length)), 0, tab);
  return { ...state, tabs };
}

export function cycleTab(state: ChatTabsState, direction: number): ChatTabsState {
  if (!state.tabs.length) return state;
  const index = state.tabs.findIndex((tab) => tab.id === state.activeId);
  return activateTab(state, state.tabs[(Math.max(index, 0) + direction + state.tabs.length) % state.tabs.length].id);
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
