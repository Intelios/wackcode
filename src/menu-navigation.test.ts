import { describe, expect, it, vi } from "vitest";
import type { ExtensionUIRequest } from "./types";
import { performChatNavigation, withoutResolvedDialog } from "./menu-navigation";

describe("native menu chat navigation", () => {
  it("leaves Settings and the draft before selecting the requested chat", () => {
    const calls: string[] = [];
    performChatNavigation("chat-2", {
      dismissSettings: () => calls.push("settings"),
      abandonDraft: () => calls.push("draft"),
      selectTask: (taskId) => calls.push(`select:${taskId}`),
    });
    expect(calls).toEqual(["settings", "draft", "select:chat-2"]);
  });

  it("selects the exact chat id supplied by the native event", () => {
    const selectTask = vi.fn();
    performChatNavigation("target-chat", {
      dismissSettings: vi.fn(),
      abandonDraft: vi.fn(),
      selectTask,
    });
    expect(selectTask).toHaveBeenCalledWith("target-chat");
  });

  it("clears only the dialog resolved by the worker", () => {
    const requests = [
      { taskId: "chat", requestId: "first", method: "confirm", title: "One", message: "One" },
      { taskId: "chat", requestId: "second", method: "confirm", title: "Two", message: "Two" },
    ] satisfies ExtensionUIRequest[];
    expect(withoutResolvedDialog(requests, "first").map((request) => request.requestId)).toEqual(["second"]);
  });
});
