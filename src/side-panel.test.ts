import { describe, expect, it } from "vitest";
import { CHANGES_VIEW, TERMINAL_VIEW, rememberedView, sameView, swapDirection, swapKey, toggleView, viewForChat, viewKey, type SidePanelView } from "./side-panel";

const scout: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 0 };
const sibling: SidePanelView = { kind: "subagent", taskId: "chat-1", toolCallId: "call-1", index: 1 };

describe("side panel views", () => {
  it("tells views apart by what they show", () => {
    expect(viewKey(CHANGES_VIEW)).toBe("changes");
    expect(viewKey(TERMINAL_VIEW)).toBe("terminal");
    expect(viewKey(scout)).toBe("subagent:chat-1:call-1#0");
    expect(sameView(scout, { ...scout })).toBe(true);
    expect(sameView(scout, sibling)).toBe(false);
    expect(sameView(CHANGES_VIEW, TERMINAL_VIEW)).toBe(false);
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
    expect(swapDirection(scout, TERMINAL_VIEW)).toBe(-1);
    expect(swapDirection(null, CHANGES_VIEW)).toBe(1);
    expect(swapDirection(null, TERMINAL_VIEW)).toBe(1);
  });

  it("slides durable views in header order: Terminal sits right of Changes", () => {
    expect(swapDirection(CHANGES_VIEW, TERMINAL_VIEW)).toBe(1);
    expect(swapDirection(TERMINAL_VIEW, CHANGES_VIEW)).toBe(-1);
    expect(swapDirection(TERMINAL_VIEW, scout)).toBe(1);
  });

  it("swaps whatever is showing for the view a trigger opens, and closes it when it is already showing", () => {
    expect(toggleView(null, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(CHANGES_VIEW, scout)).toBe(scout);
    expect(toggleView(TERMINAL_VIEW, CHANGES_VIEW)).toBe(CHANGES_VIEW);
    expect(toggleView(scout, TERMINAL_VIEW)).toBe(TERMINAL_VIEW);
    expect(toggleView(scout, sibling)).toBe(sibling);
    expect(toggleView(scout, { ...scout })).toBeNull();
    expect(toggleView(CHANGES_VIEW, CHANGES_VIEW)).toBeNull();
    expect(toggleView(TERMINAL_VIEW, TERMINAL_VIEW)).toBeNull();
  });

  it("remembers which durable view is showing and when the panel is closed, but not a sub-agent", () => {
    expect(rememberedView(CHANGES_VIEW)).toBe("changes");
    expect(rememberedView(TERMINAL_VIEW)).toBe("terminal");
    expect(rememberedView(null)).toBeNull();
    expect(rememberedView(scout)).toBeUndefined();
  });

  it("keeps a sub-agent only in its own chat, falling back to the remembered durable view", () => {
    expect(viewForChat(scout, "chat-1", null)).toBe(scout);
    expect(viewForChat(scout, "chat-2", "changes")).toBe(CHANGES_VIEW);
    expect(viewForChat(scout, "chat-2", "terminal")).toBe(TERMINAL_VIEW);
    expect(viewForChat(scout, "chat-2", null)).toBeNull();
    expect(viewForChat(scout, undefined, "terminal")).toBe(TERMINAL_VIEW);
    expect(viewForChat(TERMINAL_VIEW, "chat-2", null)).toBe(TERMINAL_VIEW);
    expect(viewForChat(CHANGES_VIEW, "chat-2", null)).toBe(CHANGES_VIEW);
    expect(viewForChat(null, "chat-2", "terminal")).toBeNull();
  });
});
