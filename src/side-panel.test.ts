import { describe, expect, it } from "vitest";
import { BUILD_VIEW, CHANGES_VIEW, GAMES_VIEW, NO_PANEL_TABS, TERMINAL_VIEW, RUN_VIEW, canExpand, closeTab, forChat, hidePanel, openTab, restoreTabs, sameView, showPanel, shownTab, storedTabs, swapDirection, swapKey, toggleTab, updateTab, viewKey, type PanelTabs, type SidePanelView } from "./side-panel";

const scout: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 0 };
const sibling: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 1 };
const browser: SidePanelView = { kind: "browser", taskId: "chat-1" };

describe("side panel views", () => {
  it("tells views apart by what they show", () => {
    expect(viewKey(CHANGES_VIEW)).toBe("changes");
    expect(viewKey(browser)).toBe("browser:chat-1");
    expect(viewKey(TERMINAL_VIEW)).toBe("terminal");
    expect(viewKey(scout)).toBe("subagent:chat-1:call-1#0");
    expect(sameView(scout, { ...scout })).toBe(true);
    expect(sameView(scout, sibling)).toBe(false);
    expect(sameView(CHANGES_VIEW, TERMINAL_VIEW)).toBe(false);
    expect(sameView(null, null)).toBe(true);
    expect(sameView(CHANGES_VIEW, null)).toBe(false);
  });

  it("slides siblings within one page and returns from a sub-agent toward durable views", () => {
    expect(swapKey(scout)).toBe(swapKey(sibling));
    expect(swapKey(scout)).not.toBe(swapKey({ ...scout, toolCallId: "call-2" }));
    expect(swapDirection(scout, sibling)).toBe(1);
    expect(swapDirection(sibling, scout)).toBe(-1);
    expect(swapDirection(CHANGES_VIEW, scout)).toBe(1);
    expect(swapDirection(scout, CHANGES_VIEW)).toBe(-1);
    expect(swapDirection(scout, TERMINAL_VIEW)).toBe(-1);
    expect(swapDirection(null, browser)).toBe(1);
  });

  it("slides durable views in header order", () => {
    expect(swapDirection(BUILD_VIEW, RUN_VIEW)).toBe(1);
    expect(swapDirection(RUN_VIEW, BUILD_VIEW)).toBe(-1);
    expect(swapDirection(RUN_VIEW, CHANGES_VIEW)).toBe(1);
    expect(swapDirection(CHANGES_VIEW, RUN_VIEW)).toBe(-1);
    expect(swapDirection(CHANGES_VIEW, TERMINAL_VIEW)).toBe(1);
    expect(swapDirection(TERMINAL_VIEW, CHANGES_VIEW)).toBe(-1);
    expect(swapDirection(TERMINAL_VIEW, scout)).toBe(1);
    expect(swapDirection(TERMINAL_VIEW, GAMES_VIEW)).toBe(1);
    expect(swapDirection(GAMES_VIEW, RUN_VIEW)).toBe(-1);
  });

  it("expands every view", () => {
    expect(canExpand(browser)).toBe(true);
    expect(canExpand(CHANGES_VIEW)).toBe(true);
    expect(canExpand(scout)).toBe(true);
    expect(canExpand(null)).toBe(false);
  });
});

const keys = (state: PanelTabs) => state.tabs.map(swapKey);

describe("panel tabs", () => {
  it("adds a tab after the front one and brings an open tab forward", () => {
    let state = openTab(NO_PANEL_TABS, CHANGES_VIEW);
    state = openTab(state, TERMINAL_VIEW);
    state = openTab({ ...state, active: "changes" }, browser);
    expect(keys(state)).toEqual(["changes", "browser", "terminal"]);
    expect(state).toMatchObject({ active: "browser", open: true });
    const again = openTab(state, TERMINAL_VIEW);
    expect(keys(again)).toEqual(["changes", "browser", "terminal"]);
    expect(again.active).toBe("terminal");
  });

  it("keeps one tab per sub-agent call and follows its sibling", () => {
    const state = openTab(openTab(NO_PANEL_TABS, scout), sibling);
    expect(state.tabs).toEqual([sibling]);
    expect(shownTab(state)).toBe(sibling);
    const updated = updateTab({ ...state, active: null }, swapKey(scout), () => scout);
    expect(updated.tabs).toEqual([scout]);
    expect(updated.active).toBeNull();
  });

  it("hides the panel when a trigger's tab is in front, and shows it otherwise", () => {
    const state = openTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), TERMINAL_VIEW);
    const hidden = toggleTab(state, TERMINAL_VIEW);
    expect(hidden).toMatchObject({ open: false, active: "terminal" });
    expect(keys(hidden)).toEqual(["changes", "terminal"]);
    expect(shownTab(hidden)).toBeNull();
    expect(toggleTab(hidden, TERMINAL_VIEW)).toMatchObject({ open: true, active: "terminal" });
    expect(toggleTab(state, CHANGES_VIEW)).toMatchObject({ open: true, active: "changes" });
    expect(toggleTab(state, GAMES_VIEW).active).toBe("games");
  });

  it("moves the front to a neighbour on close, and hides the panel with its last tab", () => {
    const state = openTab(openTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), TERMINAL_VIEW), GAMES_VIEW);
    const middle = closeTab({ ...state, active: "terminal" }, "terminal");
    expect(keys(middle)).toEqual(["changes", "games"]);
    expect(middle.active).toBe("games");
    expect(closeTab(state, "games").active).toBe("terminal");
    expect(closeTab(state, "changes").active).toBe("games");
    expect(closeTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), "changes")).toEqual(NO_PANEL_TABS);
    expect(closeTab(state, "nothing")).toBe(state);
  });

  it("hides and shows the panel without losing its tabs", () => {
    const state = openTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), TERMINAL_VIEW);
    const hidden = hidePanel(state);
    expect(hidden.tabs).toBe(state.tabs);
    expect(showPanel(hidden, CHANGES_VIEW)).toMatchObject({ open: true, active: "terminal" });
    expect(showPanel(NO_PANEL_TABS, browser)).toMatchObject({ open: true, active: "browser" });
  });

  it("keeps a chat's own browser and sub-agent tabs, dropping another chat's", () => {
    const state = openTab(openTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), browser), scout);
    expect(forChat(state, "chat-1")).toBe(state);
    const other = forChat(state, "chat-2");
    expect(keys(other)).toEqual(["changes"]);
    expect(other).toMatchObject({ active: "changes", open: true });
    expect(forChat(openTab(NO_PANEL_TABS, browser), "chat-2")).toEqual(NO_PANEL_TABS);
  });

  it("stores durable tabs and restores them, migrating the single-view and Changes-flag formats", () => {
    const state = openTab(openTab(openTab(NO_PANEL_TABS, CHANGES_VIEW), browser), TERMINAL_VIEW);
    expect(storedTabs(state)).toEqual({ tabs: ["changes", "terminal"], active: "terminal", open: true });
    expect(storedTabs({ ...state, active: "browser:chat-1" })).toEqual({ tabs: ["changes", "terminal"], active: "changes", open: true });
    expect(storedTabs(NO_PANEL_TABS)).toEqual({ tabs: [], active: null, open: false });
    const restored = restoreTabs(storedTabs(state), false);
    expect(keys(restored)).toEqual(["changes", "terminal"]);
    expect(restored).toMatchObject({ active: "terminal", open: true });
    expect(restoreTabs({ tabs: ["run", "bogus", "run"], active: "games", open: false }, false)).toEqual({ tabs: [RUN_VIEW], active: "run", open: false });
    expect(restoreTabs("terminal", false)).toEqual({ tabs: [TERMINAL_VIEW], active: "terminal", open: true });
    expect(restoreTabs(null, true)).toEqual(NO_PANEL_TABS);
    expect(restoreTabs(undefined, true)).toEqual({ tabs: [CHANGES_VIEW], active: "changes", open: true });
    expect(restoreTabs(undefined, false)).toEqual(NO_PANEL_TABS);
    expect(restoreTabs({ tabs: [] }, false)).toEqual(NO_PANEL_TABS);
  });
});
