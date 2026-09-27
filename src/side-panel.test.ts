import { describe, expect, it } from "vitest";
import { CHANGES_VIEW, rememberedChanges, sameView, swapDirection, swapKey, toggleView, viewForChat, viewKey, type SidePanelView } from "./side-panel";

const scout: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 0 };
const sibling: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 1 };
const browser: SidePanelView = { kind: "browser", taskId: "chat-1" };

describe("side panel views", () => {
  it("tells views apart by what they show", () => {
    expect(viewKey(CHANGES_VIEW)).toBe("changes");
    expect(viewKey(browser)).toBe("browser:chat-1");
    expect(viewKey(scout)).toBe("subagent:chat-1:call-1#0");
    expect(sameView(scout, { ...scout })).toBe(true);
    expect(sameView(scout, sibling)).toBe(false);
    expect(sameView(null, null)).toBe(true);
    expect(sameView(CHANGES_VIEW, null)).toBe(false);
  });

  it("slides siblings within one page, later ones from the right, and returns to Changes from the left", () => {
    expect(swapKey(scout)).toBe(swapKey(sibling));
    expect(swapKey(scout)).not.toBe(swapKey({ ...scout, toolCallId: "call-2" }));
    expect(swapDirection(scout, sibling)).toBe(1);
    expect(swapDirection(sibling, scout)).toBe(-1);
    expect(swapDirection(CHANGES_VIEW, scout)).toBe(1);
    expect(swapDirection(scout, CHANGES_VIEW)).toBe(-1);
    expect(swapDirection(null, CHANGES_VIEW)).toBe(1);
  });

  it("swaps whatever is showing for the view a trigger opens, and closes it when it is already showing", () => {
    expect(toggleView(null, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(CHANGES_VIEW, scout)).toBe(scout);
    expect(toggleView(scout, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(scout, sibling)).toBe(sibling);
    expect(toggleView(scout, { ...scout })).toBeNull();
    expect(toggleView(CHANGES_VIEW, CHANGES_VIEW)).toBeNull();
  });

  it("remembers Changes opening and the panel closing, but not a sub-agent showing", () => {
    expect(rememberedChanges(CHANGES_VIEW)).toBe(true);
    expect(rememberedChanges(null)).toBe(false);
    expect(rememberedChanges(scout)).toBeUndefined();
    expect(rememberedChanges(browser)).toBeUndefined();
  });

  it("keeps a sub-agent only in its own chat, falling back to the remembered Changes state", () => {
    expect(viewForChat(scout, "chat-1", false)).toBe(scout);
    expect(viewForChat(scout, "chat-2", true)).toBe(CHANGES_VIEW);
    expect(viewForChat(scout, "chat-2", false)).toBeNull();
    expect(viewForChat(scout, undefined, true)).toBe(CHANGES_VIEW);
    expect(viewForChat(browser, "chat-1", false)).toBe(browser);
    expect(viewForChat(browser, "chat-2", true)).toBe(CHANGES_VIEW);
    expect(viewForChat(browser, "chat-2", false)).toBeNull();
    expect(viewForChat(CHANGES_VIEW, "chat-2", false)).toBe(CHANGES_VIEW);
    expect(viewForChat(null, "chat-2", true)).toBeNull();
  });
});
