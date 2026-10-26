import { describe, expect, it, vi } from "vitest";
import { normalizeMessage, type ImageNormalizer } from "./message-normalization.js";

const imageBlock: ImageNormalizer = () => ({ type: "image", imageId: "image-1" });
const thinking = (text: string) => ({ type: "thinking", thinking: text });

it("keeps saved visual images out of live and cold snapshots without mutating the session", () => {
  const details = { v: 1, title: "Collage", inlined: true, heights: [{ width: 720, height: 450 }], html: '<img src="data:image/png;base64,original">' };
  const message = normalizeMessage({ role: "toolResult", toolName: "show_visual", toolCallId: "visual", content: [], details }, 0, undefined, imageBlock);
  expect(message?.blocks[0].details).toEqual({ v: 1, title: "Collage", inlined: true, heights: [{ width: 720, height: 450 }] });
  expect(details.html).toContain("original");
});

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

  it("shows a gateway's 413 as a sentence, in a live chat and in saved history alike", () => {
    const message = normalizeMessage({
      role: "assistant", content: [], stopReason: "error",
      errorMessage: "413: {\"message\":\"<html><title>413 Request Entity Too Large</title></html>\"}"
    }, 0, undefined, imageBlock);
    expect(message?.errorMessage).toContain("too large for this endpoint");
    expect(message?.errorMessage).not.toContain("<html>");
  });
});

describe("normalizeMessage tool result images", () => {
  const png = { type: "image", data: "aW1hZ2U=", mimeType: "image/png" };
  const result = (content: unknown[], toolName = "read") => normalizeMessage({ role: "toolResult", toolName, toolCallId: "call", content }, 0, undefined,
    (block) => ({ type: "image", imageId: `id-${String(block.data)}`, ...(block.data === "broken" ? { failed: true as const } : block.data === "slow" ? {} : { thumbnail: "data:image/png;base64,cHJldmlldw==" }) }));

  it("puts any tool's images on its one result block, as previews", () => {
    for (const toolName of ["read", "computer_screenshot", "mcp__design__export"]) {
      expect(result([{ type: "text", text: "Read image file [image/png]" }, png], toolName)?.blocks).toEqual([{
        type: "tool-result", text: "Read image file [image/png]", toolName, toolCallId: "call", isError: false,
        images: [{ imageId: "id-aW1hZ2U=", thumbnail: "data:image/png;base64,cHJldmlldw==" }]
      }]);
    }
  });

  it("keeps every image's position, whether its preview is pending or never coming", () => {
    const images = result([{ ...png, data: "slow" }, { ...png, data: "broken" }, png])?.blocks;
    // Image-only: one empty result still closes the call, and the originals stay out of it.
    expect(images).toHaveLength(1);
    expect(images?.[0]).toMatchObject({ type: "tool-result", text: "", toolCallId: "call" });
    expect(images?.[0].images).toEqual([
      { imageId: "id-slow" },
      { imageId: "id-broken", failed: true },
      { imageId: "id-aW1hZ2U=", thumbnail: "data:image/png;base64,cHJldmlldw==" }
    ]);
    expect(JSON.stringify(images)).not.toContain("aW1hZ2U=\"}");
  });

  it("leaves a result without images as it was", () => {
    expect(result([{ type: "text", text: "contents" }])?.blocks[0]).not.toHaveProperty("images");
  });
});
