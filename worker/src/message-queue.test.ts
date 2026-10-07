import { describe, expect, it } from "vitest";
import { MessageQueue } from "./message-queue.js";

describe("desktop message queue", () => {
  it("keeps FIFO order and exposes only the text and stable id", () => {
    const queue = new MessageQueue();
    queue.enqueue({ id: "a", text: "first", images: [{ type: "image", mimeType: "image/png", data: "private payload" }] });
    queue.enqueue({ id: "b", text: "second" });
    expect(queue.view()).toEqual([{ id: "a", text: "first" }, { id: "b", text: "second" }]);
    expect(queue.take()?.id).toBe("a");
    expect(queue.take()?.id).toBe("b");
    expect(queue.take()).toBeUndefined();
  });

  it("promotes by id even with duplicate text and retains all payloads", () => {
    const queue = new MessageQueue();
    const images = [{ type: "image" as const, mimeType: "image/png", data: "image" }];
    const presentation = { id: "prompt:brief", name: "brief", arguments: "diff", kind: "command" as const };
    queue.enqueue({ id: "a", text: "same" });
    queue.enqueue({ id: "b", text: "same", literal: true, images, presentation });
    queue.enqueue({ id: "c", text: "last" });
    expect(queue.promote("b", { runId: "steered", startedAt: 12, checkpoint: { id: "checkpoint" } })).toBe(true);
    expect(queue.take()).toEqual({ id: "b", text: "same", literal: true, images, presentation, runId: "steered", startedAt: 12, checkpoint: { id: "checkpoint" } });
    expect(queue.view()).toEqual([{ id: "a", text: "same" }, { id: "c", text: "last" }]);
    expect(queue.promote("b", { runId: "repeat" })).toBe(false);
  });

  it("pauses on Stop without losing input and resumes deliberately", () => {
    const queue = new MessageQueue();
    queue.enqueue({ id: "a", text: "first" });
    queue.pause();
    expect(queue.hasPending).toBe(true);
    expect(queue.hasRunnable).toBe(false);
    expect(queue.take()).toBeUndefined();
    queue.enqueue({ id: "b", text: "second" });
    queue.resume();
    expect(queue.take()?.id).toBe("a");
    expect(queue.take()?.id).toBe("b");
  });

  it("restores all texts once, including a promoted message", () => {
    const queue = new MessageQueue();
    queue.enqueue({ id: "a", text: "first" });
    queue.enqueue({ id: "b", text: "second" });
    queue.promote("b", { runId: "steered" });
    expect(queue.clear()).toEqual(["second", "first"]);
    expect(queue.clear()).toEqual([]);
    expect(queue.hasPending).toBe(false);
  });
});
