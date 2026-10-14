import { describe, expect, it, vi } from "vitest";
import { createAutoTitleExtension } from "./builtin/auto-title.js";

describe("title request routing", () => {
  it("uses the owning conversation ID even when generating an isolated title", async () => {
    const publishTitleResult = vi.fn();
    const completeSimple = vi.fn().mockResolvedValue({ stopReason: "stop", content: [{ type: "text", text: "A coding topic" }] });
    const runtime = { getModel: () => ({ id: "gemini", reasoning: false }), completeSimple };
    const { controller } = createAutoTitleExtension({ publishTitleResult } as never);
    controller.start({ attemptId: "attempt", provider: { id: "custom", kind: "custom", models: [] }, modelId: "gemini" } as never,
      "Help with code", {} as never, "/unused", "custom", runtime as never, "conversation-id");
    await vi.waitFor(() => expect(publishTitleResult).toHaveBeenCalledWith("attempt", "A coding topic"));
    expect(completeSimple.mock.calls[0][2].sessionId).toBe("conversation-id");
  });
});
