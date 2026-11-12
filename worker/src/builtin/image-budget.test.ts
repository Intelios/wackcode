import { describe, expect, it, vi } from "vitest";
import type { ExtensionAPI, ResizedImage } from "@earendil-works/pi-coding-agent";
import { imageHash } from "../saved-images.js";
import {
  MAX_CONTEXT_IMAGE_BYTES,
  PRUNED_TOOL_IMAGE_TEXT,
  PRUNED_USER_IMAGE_TEXT,
  budgetImages,
  createImageBudgetExtension,
  pruneImages
} from "./image-budget.js";

type Block = { type: string; text?: string; data?: string; mimeType?: string };
type Message = { role: string; toolName?: string; content: Block[] };

const image = (data: string, mimeType = "image/png"): Block => ({ type: "image", data, mimeType });
const text = (value: string): Block => ({ type: "text", text: value });
const tool = (toolName: string, ...content: Block[]): Message => ({ role: "toolResult", toolName, content });
const user = (...content: Block[]): Message => ({ role: "user", content });
const assistant = (): Message => ({ role: "assistant", content: [] });
const imaged = (size: number) => "x".repeat(size);

function images(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content).flatMap((block) => block.type === "image" ? [block.data ?? ""] : []);
}

function placeholders(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content).flatMap((block) => block.type === "text" && (block.text === PRUNED_TOOL_IMAGE_TEXT || block.text === PRUNED_USER_IMAGE_TEXT) ? [block.text] : []);
}

const resized = (data: string): ResizedImage => ({ data, mimeType: "image/jpeg", originalWidth: 1600, originalHeight: 900, width: 800, height: 450, wasResized: true });
const resize = vi.fn(async (_bytes: Uint8Array, _mime: string, options?: { maxBytes?: number }) => resized("y".repeat(options!.maxBytes! - 4)));

function contextHook(decoder = resize) {
  let handler: (event: { messages: Message[] }, ctx: { model?: { input: string[] } }) => Promise<{ messages: Message[] } | undefined>;
  createImageBudgetExtension(true, decoder)({ on: (_event: string, callback: typeof handler) => { handler = callback; } } as unknown as ExtensionAPI);
  return (messages: Message[], input = ["text", "image"]) => handler({ messages }, { model: { input } });
}

describe("image budget", () => {
  it("leaves a chat under the budget untouched", async () => {
    const messages = [tool("read", image(imaged(100))), tool("read", image(imaged(100)))];
    const decoder = vi.fn(resize);
    expect(pruneImages(messages, 1_000)).toBeUndefined();
    expect(await budgetImages(messages, decoder, 1_000)).toBeUndefined();
    expect(decoder).not.toHaveBeenCalled();
  });

  it("never prunes a newest message's images against themselves", () => {
    expect(pruneImages([tool("canvas_look", image(imaged(5_000)), image(imaged(5_000)))], 1_000)).toBeUndefined();
    expect(pruneImages([user(image(imaged(5_000)), image(imaged(5_000)))], 1_000)).toBeUndefined();
  });

  it("drops older tool images in whole chunks, before older attachments", () => {
    const messages = [
      tool("read", image(imaged(200))),
      user(image(imaged(200))),
      tool("read", image(imaged(200))),
      tool("computer_screenshot", image(imaged(200))),
      user(image(imaged(200))),
      assistant(),
      tool("browser_screenshot", image(imaged(200)))
    ];
    const result = pruneImages(messages, 1_000, 2)!;
    expect(placeholders(result)).toEqual([PRUNED_TOOL_IMAGE_TEXT, PRUNED_TOOL_IMAGE_TEXT]);
    expect(result[0].content[0]).toEqual(text(PRUNED_TOOL_IMAGE_TEXT));
    expect(result[2].content[0]).toEqual(text(PRUNED_TOOL_IMAGE_TEXT));
    expect(result[1]).toBe(messages[1]);
    expect(result[4]).toBe(messages[4]);
    expect(result[6]).toBe(messages[6]);
    expect(images(result)).toHaveLength(4);
  });

  it("only drops an older attachment after every older tool image", () => {
    const messages = [user(image(imaged(200))), tool("read", image(imaged(200))), assistant(), tool("read", image(imaged(200))), user(image(imaged(200)))];
    const result = pruneImages(messages, 500, 2)!;
    expect(result[1].content[0]).toEqual(text(PRUNED_TOOL_IMAGE_TEXT));
    expect(result[0].content[0]).toEqual(text(PRUNED_USER_IMAGE_TEXT));
    expect(result[3]).toBe(messages[3]);
    expect(result[4]).toBe(messages[4]);
  });

  it("never rounds a pruning chunk into the newest result's images", () => {
    const messages = [
      tool("read", image(imaged(1_600_000))),
      tool("read", image(imaged(1_600_000))),
      assistant(),
      tool("canvas_look", image(imaged(500_000)), image(imaged(500_000)))
    ];
    const result = pruneImages(messages)!;
    expect(placeholders(result)).toHaveLength(2);
    expect(result[3]).toBe(messages[3]);
    expect(images(result)).toHaveLength(2);
  });

  it("protects all results of the newest image batch, including before text-only results", () => {
    const messages = [
      tool("read", image(imaged(200))),
      assistant(),
      tool("read", text("first"), image(imaged(200))),
      tool("read", text("second"), image(imaged(200))),
      tool("ls", text("files")),
      assistant(),
      tool("grep", text("matches"))
    ];
    const result = pruneImages(messages, 300, 3)!;
    expect(placeholders(result)).toHaveLength(1);
    expect(result.slice(2)).toEqual(messages.slice(2));
    expect(images(result)).toHaveLength(2);
  });

  it("keeps a fresh tool batch when a steered image attachment arrives after it", async () => {
    const messages = [
      tool("read", image(imaged(200))),
      assistant(),
      tool("read", image(imaged(200))),
      tool("read", image(imaged(200))),
      user(text("Compare this too"), image(imaged(200)))
    ];
    const result = (await budgetImages(messages, resize, 300))!;
    expect(placeholders(result)).toHaveLength(1);
    expect(images(result)).toHaveLength(3);
    expect(images(result).reduce((sum, data) => sum + data.length, 0)).toBeLessThanOrEqual(300);
    expect(result[4].content[0]).toEqual(text("Compare this too"));
  });

  it("does not let the latest attachment flicker in and out as tool counts cross chunks", () => {
    const attachment = user(image(imaged(200)));
    const history = [attachment];
    for (let index = 0; index < 12; index++) {
      history.push(assistant(), tool("read_image", image(imaged(200))));
      const result = pruneImages(history, 300, 3) ?? history;
      expect(result[0]).toBe(attachment);
      expect(result.at(-1)).toBe(history.at(-1));
    }
  });

  it("keeps the rewritten prefix byte-identical while older images stay within a chunk", () => {
    const base = [
      tool("read", image(imaged(200))),
      user(image(imaged(200))),
      tool("read", image(imaged(200))),
      tool("computer_screenshot", image(imaged(200))),
      user(image(imaged(200))),
      assistant(),
      tool("browser_screenshot", image(imaged(200)))
    ];
    const first = pruneImages(base, 1_000, 2)!;
    const added = tool("read", image(imaged(200)));
    const next = pruneImages([...base, assistant(), added], 1_000, 2)!;
    expect(JSON.stringify(next.slice(0, base.length))).toBe(JSON.stringify(first));
    expect(next.at(-1)).toBe(added);
  });

  it("never mutates input or non-image blocks", () => {
    const messages = [
      tool("read", text("before"), image(imaged(200)), text("after")),
      tool("read", image(imaged(200))),
      tool("read", image(imaged(200))),
      assistant(),
      tool("read", image(imaged(200)))
    ];
    const before = JSON.stringify(messages);
    const result = pruneImages(messages, 500, 1)!;
    expect(JSON.stringify(messages)).toBe(before);
    expect(result[0]).not.toBe(messages[0]);
    expect(result[0].content[0]).toBe(messages[0].content[0]);
    expect(result[0].content[2]).toBe(messages[0].content[2]);
    expect(result.at(-1)).toBe(messages.at(-1));
  });

  it.each(["read", "read_image"])("fits a fresh four-result %s batch alongside its attachment without hiding any images", async (name) => {
    const messages = [
      user(text("Compare these"), image(imaged(200))),
      assistant(),
      tool("read", image(imaged(200))),
      assistant(),
      ...Array.from({ length: 4 }, (_, index) => tool(name, text(`source ${index}`), image(imaged(200)), text(`reference ${index}`)))
    ];
    const before = JSON.stringify(messages);
    const decoder = vi.fn(resize);
    const result = (await budgetImages(messages, decoder, 500))!;
    expect(placeholders(result)).toHaveLength(1);
    expect(images(result)).toHaveLength(5);
    expect(images(result).reduce((sum, data) => sum + data.length, 0)).toBeLessThanOrEqual(500);
    for (let index = 0; index < 4; index++) {
      expect(result[4 + index].content[0]).toEqual(text(`source ${index}`));
      expect(result[4 + index].content.at(-1)).toEqual(text(`reference ${index}`));
      expect(result[4 + index].content.some((block) => block.text?.includes("viewing copy 800 × 450"))).toBe(true);
    }
    expect(decoder).toHaveBeenCalledTimes(5);
    expect(JSON.stringify(messages)).toBe(before);
  });

  it("fits every attachment in an oversized initial message", async () => {
    const messages = [user(...Array.from({ length: 8 }, () => image(imaged(200))))];
    const result = (await budgetImages(messages, resize, 500))!;
    expect(images(result)).toHaveLength(8);
    expect(placeholders(result)).toHaveLength(0);
    expect(images(result).reduce((sum, data) => sum + data.length, 0)).toBeLessThanOrEqual(500);
    expect(images(messages)).toEqual(Array(8).fill(imaged(200)));
  });

  it("keeps small working-set images without decoding them and reports saved original dimensions", async () => {
    const large = { type: "image" as const, data: imaged(500), mimeType: "image/png" };
    const message = {
      ...user(image("tiny"), text("between"), large),
      wackcodeImages: { v: 1, images: [null, { blob: `${"a".repeat(64)}.png`, mimeType: "image/png", width: 2400, height: 1350, viewWidth: 1600, viewHeight: 900, inlineHash: imageHash(large) }] }
    };
    const decoder = vi.fn(resize);
    const result = (await budgetImages([message], decoder, 300))!;
    expect(decoder).toHaveBeenCalledTimes(1);
    expect(result[0].content[0]).toBe(message.content[0]);
    expect(result[0].content[1]).toBe(message.content[1]);
    expect(result[0].content.at(-1)?.text).toContain("original 2400 × 1350, viewing copy 800 × 450");
    expect(result[0].content.at(-1)?.text).toContain("by 3.00 horizontally and 3.00 vertically to map to the original");
  });

  it("reports an unresizable image explicitly without sending an oversized payload", async () => {
    const messages = [user(image(imaged(500)))];
    const result = (await budgetImages(messages, async () => null, 300))!;
    expect(images(result)).toHaveLength(0);
    expect(result[0].content[0].text).toContain("could not be resized");
    expect(images(messages)).toHaveLength(1);
  });

  it("contains decoder exceptions without abandoning the request budget or losing other images", async () => {
    const messages = [user(image("a".repeat(500)), image("b".repeat(500)), image("c".repeat(500)))];
    const decoder = vi.fn(resize).mockRejectedValueOnce(new Error("Image worker failed"));
    const result = (await budgetImages(messages, decoder, 300))!;
    expect(decoder).toHaveBeenCalledTimes(3);
    expect(images(result)).toHaveLength(2);
    expect(result[0].content[0].text).toContain("could not be resized");
    expect(images(result).reduce((sum, data) => sum + data.length, 0)).toBeLessThanOrEqual(300);
    expect(images(messages)).toHaveLength(3);
  });

  it("caches bounded request copies across calls without rewriting the session", async () => {
    const decoder = vi.fn(resize);
    const context = contextHook(decoder);
    const messages = [user(image("a".repeat(2 * 1024 * 1024))), assistant(), tool("read_image", image("b".repeat(2 * 1024 * 1024)))];
    const first = await context(messages);
    const second = await context(structuredClone(messages));
    expect(first).toEqual(second);
    expect(decoder).toHaveBeenCalledTimes(2);
    expect(images(first!.messages).reduce((sum, data) => sum + data.length, 0)).toBeLessThanOrEqual(MAX_CONTEXT_IMAGE_BYTES);
    expect(images(messages)).toEqual(["a".repeat(2 * 1024 * 1024), "b".repeat(2 * 1024 * 1024)]);
  });

  it("evicts request copies when the cache's entry or byte limit is reached", async () => {
    for (const [count, size] of [[17, 100], [6, MAX_CONTEXT_IMAGE_BYTES - 4]]) {
      const decoder = vi.fn(async () => resized("z".repeat(size)));
      const context = contextHook(decoder);
      const messages = (index: number) => [user(image(String.fromCharCode(65 + index).repeat(MAX_CONTEXT_IMAGE_BYTES + 4)))];
      for (let index = 0; index < count; index++) await context(messages(index));
      await context(messages(0));
      expect(decoder).toHaveBeenCalledTimes(count + 1);
    }
  });

  it("leaves image downgrades to Pi for a non-vision model", async () => {
    const decoder = vi.fn(resize);
    const context = contextHook(decoder);
    const messages = [user(image(imaged(MAX_CONTEXT_IMAGE_BYTES + 1)))];
    expect(await context(messages, ["text"])).toBeUndefined();
    expect(decoder).not.toHaveBeenCalled();
  });

  it("never tells a child to use parent-only image tools", () => {
    const messages = [tool("read", image(imaged(200))), assistant(), tool("read", image(imaged(200)))];
    const result = pruneImages(messages, 300, 1, false)!;
    expect(result[0].content[0].text).toContain("Read the source file again");
    expect(JSON.stringify(result)).not.toContain("read_image");
    expect(JSON.stringify(result)).not.toContain("list_images");
  });
});
