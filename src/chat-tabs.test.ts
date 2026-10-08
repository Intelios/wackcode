import { describe, expect, it } from "vitest";
import { activateTab, addDraftTab, bindTab, closeTab, cycleTab, draftTab, EMPTY_CHAT_TABS, openChatTab, patchTab, removeDraftProject, removeTaskTabs, reopenTab, reorderTab } from "./chat-tabs";

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
