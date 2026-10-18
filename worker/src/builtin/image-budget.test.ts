import { describe, expect, it } from "vitest";
import {
  PRUNED_TOOL_IMAGE_TEXT,
  PRUNED_USER_IMAGE_TEXT,
  pruneImages
} from "./image-budget.js";

type Block = { type: string; text?: string; data?: string };
type Message = { role: string; toolName?: string; content: Block[] };

const image = (data: string): Block => ({ type: "image", data });
const text = (value: string): Block => ({ type: "text", text: value });
const tool = (toolName: string, ...content: Block[]): Message => ({ role: "toolResult", toolName, content });
const user = (...content: Block[]): Message => ({ role: "user", content });
const imaged = (size: number) => "x".repeat(size);

/** Every image still in the messages, in order. */
function images(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content).flatMap((block) => block.type === "image" ? [block.data ?? ""] : []);
}

function placeholders(messages: readonly Message[]): string[] {
  return messages.flatMap((message) => message.content).flatMap((block) => block.type === "text" && (block.text === PRUNED_TOOL_IMAGE_TEXT || block.text === PRUNED_USER_IMAGE_TEXT) ? [block.text] : []);
}

describe("image budget", () => {
  it("leaves a chat under the budget untouched", () => {
    const messages = [tool("read", image(imaged(100))), tool("read", image(imaged(100)))];
    expect(pruneImages(messages, 1_000)).toBeUndefined();
  });

  it("never prunes one message's image against itself", () => {
    // Nothing older exists to drop: the newest (and only) message keeps its images.
    expect(pruneImages([tool("browser_screenshot", image(imaged(5_000)))], 1_000)).toBeUndefined();
  });

  it("drops the oldest tool images first, whole chunks at a time, and keeps the newest message", () => {
    const messages = [
      tool("read", image(imaged(200))),         // 0: tool, oldest
      user(image(imaged(200))),                 // 1: user
      tool("read", image(imaged(200))),         // 2: tool
      tool("computer_screenshot", image(imaged(200))), // 3: tool
      user(image(imaged(200))),                 // 4: user
      tool("browser_screenshot", image(imaged(200)))   // 5: newest, protected
    ];
    const result = pruneImages(messages, 1_000, 2)!;
    // Removing the two oldest tool images is enough (1200 → 800), and rounds to a chunk of two.
    expect(placeholders(result)).toEqual([PRUNED_TOOL_IMAGE_TEXT, PRUNED_TOOL_IMAGE_TEXT]);
    expect(result[0].content[0]).toEqual({ type: "text", text: PRUNED_TOOL_IMAGE_TEXT });
    expect(result[2].content[0]).toEqual({ type: "text", text: PRUNED_TOOL_IMAGE_TEXT });
    // The older user image and every later image stay: user images only go once tools run out.
    expect(result[1].content[0]).toEqual(image(imaged(200)));
    expect(images(result)).toHaveLength(4);
    expect(images(result)[0]).toBe(imaged(200)); // the user's own image in message 1
    expect(images(result)[3]).toBe(imaged(200)); // the protected newest image
  });

  it("only drops a user image after every older tool image", () => {
    const messages = [
      user(image(imaged(200))),       // 0: user, oldest
      tool("read", image(imaged(200))), // 1: tool
      user(image(imaged(200)))        // 2: newest, protected
    ];
    const result = pruneImages(messages, 300, 2)!;
    expect(result[1].content[0]).toEqual({ type: "text", text: PRUNED_TOOL_IMAGE_TEXT });
    expect(result[0].content[0]).toEqual({ type: "text", text: PRUNED_USER_IMAGE_TEXT });
  });

  it("only prunes inside the newest message after every older image is gone", () => {
    const messages = [
      tool("read", image(imaged(200))),
      tool("browser_screenshot", image(imaged(200)), image(imaged(200)), image(imaged(200)))
    ];
    // 800 over a 500 budget: the older read goes first, then the newest message's oldest image.
    const result = pruneImages(messages, 500, 1)!;
    expect(result[0].content[0]).toEqual({ type: "text", text: PRUNED_TOOL_IMAGE_TEXT });
    expect(result[1].content[0]).toEqual({ type: "text", text: PRUNED_TOOL_IMAGE_TEXT });
    expect(result[1].content[1]).toEqual(image(imaged(200)));
    expect(result[1].content[2]).toEqual(image(imaged(200)));
  });

  it("keeps the rewritten prefix byte-identical while images stay within a chunk", () => {
    const base = [
      tool("read", image(imaged(200))),
      user(image(imaged(200))),
      tool("read", image(imaged(200))),
      tool("computer_screenshot", image(imaged(200))),
      user(image(imaged(200))),
      tool("browser_screenshot", image(imaged(200)))
    ];
    const first = pruneImages(base, 1_000, 2)!;
    // One more image arrives, still inside the same chunk: the pruned prefix must not change.
    const added = tool("read", image(imaged(200)));
    const next = pruneImages([...base, added], 1_000, 2)!;
    expect(JSON.stringify(next.slice(0, base.length))).toBe(JSON.stringify(first));
    expect(next[base.length]).toBe(added);
  });

  it("never mutates its input, and leaves non-image blocks and other messages alone", () => {
    const messages = [
      tool("read", text("before"), image(imaged(200)), text("after")),
      tool("read", image(imaged(200))),
      tool("read", image(imaged(200))),
      tool("read", image(imaged(200)))
    ];
    const before = JSON.stringify(messages);
    const result = pruneImages(messages, 500, 1)!;
    expect(JSON.stringify(messages)).toBe(before);
    expect(result[0]).not.toBe(messages[0]);
    expect(result[0].content[0]).toBe(messages[0].content[0]);
    expect(result[0].content[2]).toBe(messages[0].content[2]);
    expect(result[3]).toBe(messages[3]);
  });
});
