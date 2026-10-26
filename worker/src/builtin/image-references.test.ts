import { describe, expect, it } from "vitest";
import type { ExtensionAPI, SessionProjection } from "@earendil-works/pi-coding-agent";
import { createBuiltinExtensions } from "./index.js";
import type { BuiltinHost } from "./host.js";
import { annotateImageReferences, createImageReferencesExtension, imageReferenceUrl, parseImageReference } from "./image-references.js";
import { pruneImages, PRUNED_TOOL_IMAGE_TEXT } from "./image-budget.js";

type Message = { role: string; timestamp: number; toolCallId?: string; toolName?: string; content: Array<{ type: string; text?: string; data?: string; mimeType?: string }> };
const image = (data: string, mimeType = "image/png") => ({ type: "image", data, mimeType });
const text = (value: string) => ({ type: "text", text: value });
const user = (...content: Message["content"]): Message => ({ role: "user", timestamp: 1, content });
const tool = (name: string, ...content: Message["content"]): Message => ({ role: "toolResult", toolName: name, toolCallId: "call", timestamp: 2, content });
const projection = (...sources: Array<[string, Message]>) => ({
  entries: sources.map(([id, message]) => ({ sourceEntry: { type: "message", id, message }, messages: [message] })),
}) as unknown as Pick<SessionProjection, "entries">;

describe("image references", () => {
  it("accepts only exact bounded entry references with canonical safe image indices", () => {
    expect(imageReferenceUrl("aB_9-z", 0)).toBe("wackcode-image://entry/aB_9-z/0");
    expect(parseImageReference("wackcode-image://entry/aB_9-z/12")).toEqual({ entryId: "aB_9-z", index: 12 });
    expect(parseImageReference(imageReferenceUrl("a".repeat(256), Number.MAX_SAFE_INTEGER))).toEqual({ entryId: "a".repeat(256), index: Number.MAX_SAFE_INTEGER });
    for (const invalid of [
      "wackcode-image://entry/a/01", "wackcode-image://entry/a/-1", "wackcode-image://entry/a/+1", "wackcode-image://entry/a/1.0",
      "wackcode-image://entry/a/9007199254740992", "wackcode-image://entry/a/0?x=1", "wackcode-image://entry/a/0#x",
      "wackcode-image://entry/%61/0", "wackcode-image://entry/a%2Fb/0", "wackcode-image://entry/../0", "wackcode-image://entry/a/0/",
      "wackcode-image://entry//0", `wackcode-image://entry/${"a".repeat(257)}/0`, "WACKCODE-IMAGE://entry/a/0", "https://entry/a/0",
      "wackcode-image://tool/a/0", "file:///tmp/a.png", "wackcode-image://entry/a/0\n", undefined,
    ]) expect(parseImageReference(invalid), String(invalid)).toBeUndefined();
    expect(imageReferenceUrl("../a", 0)).toBeUndefined();
    expect(imageReferenceUrl("a", -1)).toBeUndefined();
    expect(imageReferenceUrl("a", 0.5)).toBeUndefined();
    expect(imageReferenceUrl("a", Infinity)).toBeUndefined();
  });

  it("labels cloned attachments and every tool's saved images without changing transcript originals", () => {
    const attachment = user(text("Compare these"), image("first"), text("between"), image("second", "image/jpeg"));
    for (const name of ["read", "browser_screenshot", "computer_screenshot", "mcp__server__image", "package_tool"]) {
      const result = tool(name, image("tool-image"));
      const saved = projection(["user-id", attachment], ["result-id", result]);
      const messages = structuredClone([attachment, result]);
      const before = JSON.stringify({ saved, messages });
      const annotated = annotateImageReferences(messages, saved)!;
      expect(annotated[0].content.at(-1)).toEqual(text("\n\n<wackcode-image-references-v1>\nImage 1: wackcode-image://entry/user-id/0\nImage 2: wackcode-image://entry/user-id/1\n</wackcode-image-references-v1>"));
      expect(annotated[1].content.at(-1)?.text).toContain("Image 1: wackcode-image://entry/result-id/0");
      expect(annotated[0].content[1]).toBe(messages[0].content[1]);
      expect(JSON.stringify({ saved, messages })).toBe(before);
      expect(annotateImageReferences(annotated, saved)).toBeUndefined();
    }
  });

  it("matches projected context edits and package prose changes without relying on message position", () => {
    const original = user(text("Original"), image("old"));
    const edited = user(text("Edited"), image("replacement"));
    const saved = projection(["entry", original]);
    saved.entries[0].messages = [edited as SessionProjection["messages"][number]];
    const cloned = structuredClone(edited);
    cloned.content[0] = text("Package changed the text");
    const unrelated = { ...user(text("Other")), timestamp: 3 };
    const annotated = annotateImageReferences([unrelated, cloned], saved)!;
    expect(annotated[0]).toBe(unrelated);
    expect(annotated[1].content.at(-1)?.text).toContain("wackcode-image://entry/entry/0");
    expect(annotateImageReferences([original], saved)).toBeUndefined();
  });

  it("skips unknown, ambiguous, transformed and unsaved sources", () => {
    const original = user(image("one"), image("two"));
    expect(annotateImageReferences([original], projection())).toBeUndefined();
    expect(annotateImageReferences([original], projection(["one", original], ["two", structuredClone(original)]))).toBeUndefined();
    expect(annotateImageReferences([{ ...original, timestamp: 9 }], projection(["entry", original]))).toBeUndefined();
    expect(annotateImageReferences([user(image("two"), image("one"))], projection(["entry", original]))).toBeUndefined();
    expect(annotateImageReferences([user(image("changed"), image("two"))], projection(["entry", original]))).toBeUndefined();
    expect(annotateImageReferences([original], projection(["../entry", original]))).toBeUndefined();
    const nested = projection(["entry", original]);
    nested.entries[0].sourceEntry.type = "custom_message";
    expect(annotateImageReferences([original], nested)).toBeUndefined();
    const sourceTool = tool("read", image("tool"));
    expect(annotateImageReferences([{ ...sourceTool, toolCallId: "other-call" }], projection(["entry", sourceTool]))).toBeUndefined();
    expect(annotateImageReferences([{ ...sourceTool, role: "assistant" }], projection(["entry", sourceTool]))).toBeUndefined();
  });

  it("does not shift image indices around malformed image blocks", () => {
    const malformed = user({ type: "image", mimeType: "image/png" }, image("valid"));
    expect(annotateImageReferences([malformed], projection(["entry", malformed]))).toBeUndefined();
  });

  it("leaves model-visible references after image-budget pruning", () => {
    const messages = [tool("read", image("old-image")), { ...user(image("new-image")), timestamp: 3 }];
    const annotated = annotateImageReferences(messages, projection(["tool-id", messages[0]], ["user-id", messages[1]]))!;
    const pruned = pruneImages(annotated, 10, 1)!;
    expect(pruned[0].content[0]).toEqual(text(PRUNED_TOOL_IMAGE_TEXT));
    expect(pruned[0].content.at(-1)?.text).toContain("wackcode-image://entry/tool-id/0");
    expect(pruned[1].content.at(-1)?.text).toContain("wackcode-image://entry/user-id/0");
  });

  it("installs one parent hook before the budget in both chat kinds", () => {
    const builtins = createBuiltinExtensions({} as BuiltinHost);
    for (const factories of [builtins.factories, builtins.chatFactories]) {
      const names = factories.map((factory) => factory.name);
      expect(names.filter((name) => name === "wackcode-image-references")).toHaveLength(1);
      expect(names.indexOf("wackcode-image-references") + 1).toBe(names.indexOf("wackcode-image-budget"));
    }
    let handler: ((event: { messages: Message[] }, ctx: { sessionManager: { buildSessionProjection(): Pick<SessionProjection, "entries"> } }) => { messages: Message[] } | undefined) | undefined;
    createImageReferencesExtension()({ on: (_event: string, callback: typeof handler) => { handler = callback; } } as unknown as ExtensionAPI);
    const newest = user(image("newest"));
    const result = handler!({ messages: structuredClone([newest]) }, { sessionManager: { buildSessionProjection: () => projection(["newest-entry", newest]) } });
    expect(result?.messages[0].content.at(-1)?.text).toContain("wackcode-image://entry/newest-entry/0");
  });
});
