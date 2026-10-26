import { describe, expect, it } from "vitest";
import type { NormalizedMessage } from "./types";
import { parseReplyImageReference, replyImageReference, replyImages } from "./reply-images";

describe("reply image references", () => {
  it("accepts only canonical entry references", () => {
    expect(parseReplyImageReference(replyImageReference("entry_A-1", 3))).toEqual({ entryId: "entry_A-1", index: 3 });
    expect(parseReplyImageReference(replyImageReference("a", 0))).toEqual({ entryId: "a", index: 0 });
    for (const source of [
      "https://example.com/image.png", "/tmp/image.png", "data:image/png;base64,AA==", "file:///tmp/image.png",
      "wackcode-image://entry/a/01", "wackcode-image://entry/a/-1", "wackcode-image://entry/a/1.5",
      "wackcode-image://entry/a/9007199254740992", "wackcode-image://entry/a/0?query", "wackcode-image://entry/a/0#fragment",
      "wackcode-image://entry/%61/0", "wackcode-image://entry/../0", "wackcode-image://entry/a/0\n",
      `wackcode-image://entry/${"a".repeat(257)}/0`
    ]) expect(parseReplyImageReference(source), source).toBeUndefined();
  });

  it("indexes user attachments and tool images by their saved entry and image position", () => {
    const messages: NormalizedMessage[] = [
      { id: "pending", role: "user", blocks: [{ type: "image", thumbnail: "pending" }] },
      { id: "u", entryId: "user-entry", role: "user", blocks: [
        { type: "text", text: "Compare these" }, { type: "image", thumbnail: "before" }, { type: "image", failed: true }
      ] },
      { id: "r", entryId: "result-entry", role: "tool", blocks: [
        { type: "tool-result", toolCallId: "call", text: "Read image" },
        { type: "tool-result", toolCallId: "call", images: [{ imageId: "new", thumbnail: "after" }, { imageId: "preparing" }] }
      ] },
      { id: "a", entryId: "assistant-entry", role: "assistant", blocks: [{ type: "image", thumbnail: "ignored" }] }
    ];
    const images = replyImages(messages);
    expect([...images.keys()]).toEqual([
      replyImageReference("user-entry", 0), replyImageReference("user-entry", 1),
      replyImageReference("result-entry", 0), replyImageReference("result-entry", 1)
    ]);
    expect(images.get(replyImageReference("user-entry", 0))).toMatchObject({ kind: "message", entryId: "user-entry", index: 0, thumbnail: "before" });
    expect(images.get(replyImageReference("user-entry", 1))?.failed).toBe(true);
    expect(images.get(replyImageReference("result-entry", 0))).toMatchObject({ kind: "tool", toolCallId: "call", index: 0, thumbnail: "after" });
    expect(images.get(replyImageReference("result-entry", 1))?.thumbnail).toBeUndefined();
    expect(replyImages(messages.slice(0, 2)).has(replyImageReference("result-entry", 0))).toBe(false);
  });
});
