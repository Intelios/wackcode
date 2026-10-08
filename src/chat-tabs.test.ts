import { describe, expect, it } from "vitest";
import { activateTab, addDraftTab, bindTab, closeTab, cycleTab, draftTab, EMPTY_CHAT_TABS, openChatTab, patchTab, removeDraftProject, removeTaskTabs, reopenTab, reorderTab, switchTabArea, tabsInArea } from "./chat-tabs";

describe("session chat tabs", () => {
  it("focuses existing chats without changing order or duplicating them", () => {
    let state = openChatTab(openChatTab(EMPTY_CHAT_TABS, "a"), "b");
    state = openChatTab(state, "a");
    expect(state.tabs.map((tab) => tab.taskId)).toEqual(["a", "b"]);
    expect(state.activeId).toBe(state.tabs[0].id);
  });

  it("keeps a draft's identity, independent model, composer key and panel when sent", () => {
    const first = draftTab("one", { projectId: "project", useWorktree: true, choice: { providerId: "p", modelId: "one", thinkingLevel: "high" }, mode: "plan" });
    const second = draftTab("two", { projectId: null, useWorktree: false, choice: { providerId: "p", modelId: "two", thinkingLevel: "off" } });
    let state = addDraftTab(addDraftTab(EMPTY_CHAT_TABS, first), second);
    state = patchTab(state, first.id, { panel: { ...first.panel, view: { kind: "browser", taskId: "task" }, width: 600 } });
    state = bindTab(state, first.id, "task");
    expect(state.activeId).toBe("two");
    expect(state.tabs[0]).toMatchObject({ id: "one", composerKey: "new:one", taskId: "task", panel: { width: 600 } });
    expect(state.tabs[1].draft?.choice?.modelId).toBe("two");
  });

  it("chooses the right neighbor, then the left, without changing inactive selection", () => {
    const state = activateTab(openChatTab(openChatTab(openChatTab(EMPTY_CHAT_TABS, "a"), "b"), "c"), "chat:b");
    expect(closeTab(state, "chat:b").activeId).toBe("chat:c");
    expect(closeTab(activateTab(state, "chat:c"), "chat:c").activeId).toBe("chat:b");
    expect(closeTab(state, "chat:a").activeId).toBe("chat:b");
  });

  it("updates closed records after asynchronous creation without reopening them", () => {
    const draft = draftTab("one", { projectId: null, useWorktree: false });
    let state = closeTab(addDraftTab(EMPTY_CHAT_TABS, draft), "one");
    state = addDraftTab(state, draftTab("two", { projectId: null, useWorktree: false }));
    state = bindTab(state, "one", "created");
    expect(state.tabs).toHaveLength(1);
    expect(state.activeId).toBe("two");
    expect(state.closed[0].tab.taskId).toBe("created");
    state = reopenTab(state);
    expect(state.activeId).toBe("one");
    expect(state.tabs[0].composerKey).toBe("new:one");
  });

  it("recovers the last ten closures in reverse order and deduplicates sidebar reopening", () => {
    let state = EMPTY_CHAT_TABS;
    for (let i = 0; i < 12; i++) state = closeTab(openChatTab(state, String(i)), `chat:${i}`);
    expect(state.closed).toHaveLength(10);
    state = openChatTab(state, "11");
    expect(state.closed[0].tab.taskId).toBe("10");
    state = reopenTab(state);
    expect(state.tabs.map((tab) => tab.taskId)).toEqual(["10", "11"]);
  });

  it("does not select a closed origin over an explicitly opened chat", () => {
    let state = closeTab(addDraftTab(EMPTY_CHAT_TABS, draftTab("origin", { projectId: null, useWorktree: false })), "origin");
    state = openChatTab(state, "created");
    state = bindTab(state, "origin", "created");
    expect(state.activeId).toBe("chat:created");
    expect(state.tabs).toHaveLength(1);
    expect(state.closed[0].tab.taskId).toBe("created");
  });

  it("preserves active selection through reordering and cycles in visible order", () => {
    let state = openChatTab(openChatTab(openChatTab(EMPTY_CHAT_TABS, "a"), "b"), "c");
    state = reorderTab(state, "chat:c", 0);
    expect(state.tabs.map((tab) => tab.taskId)).toEqual(["c", "a", "b"]);
    expect(cycleTab(state, -1).activeId).toBe("chat:b");
    expect(cycleTab(cycleTab(state, 1), 1).activeId).toBe("chat:b");
  });

  describe("across the Code and Chat areas", () => {
    // Interleaved on purpose: code a, chat x, code b, chat y.
    const mixed = () => {
      let state = openChatTab(EMPTY_CHAT_TABS, "a");
      state = openChatTab(state, "x", true, undefined, "chat");
      state = openChatTab(state, "b");
      return openChatTab(state, "y", true, undefined, "chat");
    };
    const ids = (state: ReturnType<typeof mixed>, area: "code" | "chat") => tabsInArea(state, area).map((tab) => tab.taskId);

    it("tags every tab with its area, defaulting to Code", () => {
      const state = mixed();
      expect(ids(state, "code")).toEqual(["a", "b"]);
      expect(ids(state, "chat")).toEqual(["x", "y"]);
      expect(draftTab("d", { projectId: null, useWorktree: false }).area).toBe("code");
      expect(draftTab("d", { projectId: null, useWorktree: false }, undefined, "chat").area).toBe("chat");
    });

    it("remembers each area's open tab across a switch", () => {
      let state = activateTab(mixed(), "chat:x");
      state = switchTabArea(state, "chat", "code");
      // Code was never left on a tab of its own, so its last one opens.
      expect(state.activeId).toBe("chat:b");
      state = switchTabArea(activateTab(state, "chat:a"), "code", "chat");
      expect(state.activeId).toBe("chat:x");
      expect(switchTabArea(state, "chat", "code").activeId).toBe("chat:a");
    });

    it("opens no tab when the area being entered has none, or its parked tab is gone", () => {
      const onlyCode = openChatTab(EMPTY_CHAT_TABS, "a");
      expect(switchTabArea(onlyCode, "code", "chat").activeId).toBeUndefined();
      let state = switchTabArea(activateTab(mixed(), "chat:x"), "chat", "code");
      state = closeTab(state, "chat:x");
      expect(switchTabArea(state, "code", "chat").activeId).toBe("chat:y");
    });

    it("closes onto a neighbour in the same area, never the other one's tab", () => {
      const state = activateTab(mixed(), "chat:x");
      expect(closeTab(state, "chat:x").activeId).toBe("chat:y");
      expect(closeTab(activateTab(state, "chat:y"), "chat:y").activeId).toBe("chat:x");
      const last = closeTab(closeTab(activateTab(state, "chat:y"), "chat:y"), "chat:x");
      // Chat is empty now; Code's tabs are not a fallback.
      expect(last.activeId).toBeUndefined();
      expect(ids(last, "code")).toEqual(["a", "b"]);
    });

    it("cycles and reorders within the open tab's area and leaves the other untouched", () => {
      let state = activateTab(mixed(), "chat:x");
      expect(cycleTab(state, 1).activeId).toBe("chat:y");
      expect(cycleTab(cycleTab(state, 1), 1).activeId).toBe("chat:x");
      state = reorderTab(state, "chat:y", 0);
      expect(ids(state, "chat")).toEqual(["y", "x"]);
      expect(ids(state, "code")).toEqual(["a", "b"]);
      expect(state.tabs.map((tab) => tab.taskId)).toEqual(["a", "y", "b", "x"]);
    });

    it("reopens the last tab closed in the area asked for", () => {
      let state = closeTab(closeTab(mixed(), "chat:x"), "chat:b");
      state = reopenTab(state, "chat");
      expect(state.activeId).toBe("chat:x");
      expect(state.closed.map((entry) => entry.tab.taskId)).toEqual(["b"]);
      expect(reopenTab(state, "chat")).toBe(state);
      expect(reopenTab(state, "code").activeId).toBe("chat:b");
    });

    it("keeps the parked tab through unrelated changes", () => {
      let state = switchTabArea(activateTab(mixed(), "chat:x"), "chat", "code");
      state = closeTab(state, "chat:b");
      state = reorderTab(state, "chat:a", 0);
      expect(switchTabArea(state, "code", "chat").activeId).toBe("chat:x");
    });
  });

  it("archives into recovery, purges deleted chats, and reconciles projects in closed drafts", () => {
    let state = openChatTab(openChatTab(EMPTY_CHAT_TABS, "a"), "b");
    state = removeTaskTabs(state, new Set(["a"]), false);
    expect(state.closed[0].tab.taskId).toBe("a");
    state = removeTaskTabs(state, new Set(["a", "b"]), true);
    expect(state.tabs).toEqual([]);
    expect(state.closed).toEqual([]);
    state = closeTab(addDraftTab(state, draftTab("draft", { projectId: "gone", useWorktree: true, mode: "ultraplan" })), "draft");
    expect(removeDraftProject(state, "gone").closed[0].tab.draft).toEqual({ projectId: null, useWorktree: false, mode: "ultraplan" });
  });
});
