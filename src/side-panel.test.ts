import { describe, expect, it } from "vitest";
import { CHANGES_VIEW, TERMINAL_VIEW, rememberedView, sameView, swapDirection, swapKey, toggleView, viewForChat, viewKey, type SidePanelView } from "./side-panel";

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
    expect(swapDirection(CHANGES_VIEW, TERMINAL_VIEW)).toBe(1);
    expect(swapDirection(TERMINAL_VIEW, CHANGES_VIEW)).toBe(-1);
    expect(swapDirection(TERMINAL_VIEW, scout)).toBe(1);
  });

  it("swaps whatever is showing for a trigger's view, and closes the active view", () => {
    expect(toggleView(null, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(CHANGES_VIEW, browser)).toBe(browser);
    expect(toggleView(browser, TERMINAL_VIEW)).toBe(TERMINAL_VIEW);
    expect(toggleView(TERMINAL_VIEW, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(scout, sibling)).toBe(sibling);
    expect(toggleView(scout, { ...scout })).toBeNull();
    expect(toggleView(CHANGES_VIEW, CHANGES_VIEW)).toBeNull();
    expect(toggleView(TERMINAL_VIEW, TERMINAL_VIEW)).toBeNull();
  });

  it("remembers durable views and closing, but not task-bound views", () => {
    expect(rememberedView(CHANGES_VIEW)).toBe("changes");
    expect(rememberedView(TERMINAL_VIEW)).toBe("terminal");
    expect(rememberedView(null)).toBeNull();
    expect(rememberedView(scout)).toBeUndefined();
    expect(rememberedView(browser)).toBeUndefined();
  });

  it("keeps task-bound views in their own chat and falls back to the remembered durable view", () => {
    expect(viewForChat(scout, "chat-1", null)).toBe(scout);
    expect(viewForChat(scout, "chat-2", "changes")).toBe(CHANGES_VIEW);
    expect(viewForChat(scout, "chat-2", "terminal")).toBe(TERMINAL_VIEW);
    expect(viewForChat(browser, "chat-1", null)).toBe(browser);
    expect(viewForChat(browser, "chat-2", "terminal")).toBe(TERMINAL_VIEW);
    expect(viewForChat(browser, "chat-2", null)).toBeNull();
    expect(viewForChat(TERMINAL_VIEW, "chat-2", null)).toBe(TERMINAL_VIEW);
    expect(viewForChat(CHANGES_VIEW, "chat-2", null)).toBe(CHANGES_VIEW);
    expect(viewForChat(null, "chat-2", "terminal")).toBeNull();
  });
});
