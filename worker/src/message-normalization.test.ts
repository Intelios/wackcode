import { describe, expect, it, vi } from "vitest";
import { normalizeMessage, type ImageNormalizer } from "./message-normalization.js";

const imageBlock: ImageNormalizer = () => ({ type: "image", imageId: "image-1" });
const thinking = (text: string) => ({ type: "thinking", thinking: text });

describe("normalizeMessage thinking timing", () => {
  it("maps starts and durations by thinking ordinal, not raw content index", () => {
    const normalizeImage = vi.fn(imageBlock);
    const message = normalizeMessage({
      role: "assistant",
      content: [
        { type: "text", text: "Before" },
        thinking("Closed"),
        "Between",
        { type: "image", data: "image", mimeType: "image/png" },
        thinking("Open"),
        { type: "toolCall", name: "read", id: "call", arguments: { path: "." } },
        thinking("Unclocked")
      ]
    }, 0, [1_200, null, null], normalizeImage, [1_000, 2_500, null]);
    expect(message?.blocks).toEqual([
      { type: "text", text: "Before" },
      { type: "thinking", text: "Closed", durationMs: 1_200 },
      { type: "text", text: "Between" },
      { type: "image", imageId: "image-1" },
      { type: "thinking", text: "Open", startedAt: 2_500 },
      { type: "tool-call", toolName: "read", toolCallId: "call", arguments: { path: "." } },
      { type: "thinking", text: "Unclocked" }
    ]);
    expect(normalizeImage).toHaveBeenCalledExactlyOnceWith({ type: "image", data: "image", mimeType: "image/png" });
  });

  it("omits live starts once a duration is known, including zero", () => {
    const message = normalizeMessage({ role: "assistant", content: [thinking("Instant"), thinking("Done")] },
      0, [0, 500], imageBlock, [1_000, 2_000]);
    expect(message?.blocks).toEqual([
      { type: "thinking", text: "Instant", durationMs: 0 },
      { type: "thinking", text: "Done", durationMs: 500 }
    ]);
    expect(message?.blocks.every((block) => !Object.hasOwn(block, "startedAt"))).toBe(true);
  });

  it("keeps starts optional for saved history and does not copy timing from raw blocks", () => {
    const raw = { role: "assistant", content: [{ ...thinking("History"), startedAt: 100 }] };
    expect(normalizeMessage(raw, 0, undefined, imageBlock)?.blocks).toEqual([{ type: "thinking", text: "History" }]);
    expect(normalizeMessage(raw, 0, [300], imageBlock)?.blocks).toEqual([{ type: "thinking", text: "History", durationMs: 300 }]);
    expect(normalizeMessage(raw, 0, undefined, imageBlock, [0])?.blocks).toEqual([{ type: "thinking", text: "History", startedAt: 0 }]);
  });
});
