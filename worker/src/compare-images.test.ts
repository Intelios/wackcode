import { describe, expect, it } from "vitest";
import { COMPARE_IMAGES_GUIDE_LINES, COMPARE_IMAGES_TOOL_NAME, createCompareImagesExtension, normalizeCompareImagesParams, savedComparisonImage } from "./builtin/compare-images.js";
import { createBuiltinExtensions, SWITCHABLE_BUILTIN_TOOLS } from "./builtin/index.js";
import { chatToolAllowed } from "./builtin/chat-mode/policy.js";
import { buildChatGuide } from "./builtin/chat-mode/prompt.js";
import { createPlanModeExtension } from "./builtin/plan-mode/index.js";
import type { EntryLike } from "./tree.js";

const before = "wackcode-image://entry/upload/0";
const after = "wackcode-image://entry/result/1";
const image = (data = "saved-pixels") => ({ type: "image", data, mimeType: "image/png" });
const branch = (): EntryLike[] => [
  { type: "message", id: "upload", parentId: null, message: { role: "user", content: [image()] } },
  { type: "message", id: "result", parentId: "upload", message: { role: "toolResult", toolCallId: "read", content: [image("first"), { type: "text", text: "between" }, image("second")] } },
];

describe("image comparison validation", () => {
  it("requires only two canonical references and normalizes optional labels and title", () => {
    expect(normalizeCompareImagesParams({ before, after })).toEqual({ ok: true, comparison: { v: 1, before, after, title: "Before / After", beforeLabel: "Before", afterLabel: "After" } });
    expect(normalizeCompareImagesParams({ before, after, title: "  New\n layout ", beforeLabel: " Option   A ", afterLabel: "Option B" })).toMatchObject({ ok: true, comparison: { title: "New layout", beforeLabel: "Option A", afterLabel: "Option B" } });
    expect(normalizeCompareImagesParams({ before, after, beforeLabel: "Original", afterLabel: "Edited" })).toMatchObject({ ok: true, comparison: { title: "Original / Edited" } });
  });

  it("refuses paths, URLs, malformed references and empty or overlong labels", () => {
    for (const ref of ["/tmp/image.png", "https://example.com/image.png", "data:image/png;base64,AAAA", "wackcode-image://entry/upload/01", "wackcode-image://entry/upload/-1", undefined]) {
      expect(normalizeCompareImagesParams({ before: ref, after }).ok).toBe(false);
      expect(normalizeCompareImagesParams({ before, after: ref }).ok).toBe(false);
    }
    for (const [key, max] of [["title", 60], ["beforeLabel", 32], ["afterLabel", 32]] as const) {
      for (const value of [" ", 12, "x".repeat(max + 1)]) expect(normalizeCompareImagesParams({ before, after, [key]: value }).ok).toBe(false);
      expect(normalizeCompareImagesParams({ before, after, [key]: "x".repeat(max) }).ok).toBe(true);
    }
    expect(normalizeCompareImagesParams(null).ok).toBe(false);
    const longest = normalizeCompareImagesParams({ before, after, beforeLabel: "a".repeat(32), afterLabel: "b".repeat(32) });
    expect(longest.ok && longest.comparison.title.length).toBe(60);
  });

  it("resolves attachments and tool images by their saved image index, only on this branch", () => {
    expect(savedComparisonImage(before, branch())).toBe(true);
    expect(savedComparisonImage(after, branch())).toBe(true);
    expect(savedComparisonImage("wackcode-image://entry/result/2", branch())).toBe(false);
    expect(savedComparisonImage(before, branch().slice(1))).toBe(false);
    const malformed = branch();
    malformed[1].message = { role: "toolResult", content: [image()] };
    expect(savedComparisonImage("wackcode-image://entry/result/0", malformed)).toBe(false);
    malformed[0].message = { role: "assistant", content: [image()] };
    expect(savedComparisonImage(before, malformed)).toBe(false);
  });

  it("honors context edits and omissions without losing images to compaction", () => {
    const entries = branch();
    entries.push({ type: "compaction", id: "compact", parentId: "result", summary: "Earlier work", firstKeptEntryId: "result" } as EntryLike);
    expect(savedComparisonImage(before, entries)).toBe(true);
    entries.push({ type: "context_edit", id: "edit", parentId: "compact", targetId: "upload", replacement: { content: [{ type: "text", text: "Image removed" }] } } as EntryLike);
    expect(savedComparisonImage(before, entries)).toBe(false);
    entries.push({ type: "context_edit", id: "edit2", parentId: "edit", targetId: "upload", replacement: { content: [image("new")] } } as EntryLike);
    expect(savedComparisonImage(before, entries)).toBe(true);
    entries.push({ type: "context_edit", id: "omit", parentId: "edit2", targetId: "upload", replacement: null } as EntryLike);
    expect(savedComparisonImage(before, entries)).toBe(false);
  });
});

describe("compare_images tool and policy", () => {
  function tool() {
    let registered: { execute: (id: string, input: unknown, signal: AbortSignal | undefined, update: undefined, ctx: unknown) => Promise<{ details: unknown; content: unknown }> };
    createCompareImagesExtension()({ registerTool(value: typeof registered) { registered = value; } } as never);
    return registered!;
  }

  it("saves only the accepted pair, refuses absent sources and never copies pixels", async () => {
    const ctx = { sessionManager: { getBranch: branch } };
    const result = await tool().execute("compare", { before, after }, undefined, undefined, ctx);
    expect(result.details).toMatchObject({ v: 1, before, after, title: "Before / After" });
    expect(JSON.stringify(result)).not.toContain("saved-pixels");
    await expect(tool().execute("compare", { before, after }, undefined, undefined, { sessionManager: { getBranch: () => branch().slice(0, 1) } })).rejects.toThrow("after image is not available");
    await expect(tool().execute("compare", { before, after }, AbortSignal.abort(), undefined, ctx)).rejects.toThrow("stopped");
  });

  it("is switchable and loaded in both parent chat kinds", () => {
    const builtins = createBuiltinExtensions({} as never);
    for (const factories of [builtins.factories, builtins.chatFactories]) expect(factories.map((extension) => extension.name)).toContain("wackcode-compare-images");
    expect(SWITCHABLE_BUILTIN_TOOLS.has(COMPARE_IMAGES_TOOL_NAME)).toBe(true);
    expect(chatToolAllowed(COMPARE_IMAGES_TOOL_NAME, "wackcode")).toBe(true);
  });

  it.each(["plan", "ultraplan"] as const)("is allowed during restricted %s without changing the plan contract", (mode) => {
    const handlers = new Map<string, (event: unknown, context: unknown) => unknown>();
    const { factory, controller } = createPlanModeExtension({ publishPlanState() {} } as never);
    factory({ registerTool() {}, appendEntry() {}, sendMessage() {}, on: (name: string, callback: (event: unknown, context: unknown) => unknown) => handlers.set(name, callback) } as never);
    controller.setMode(mode);
    expect(handlers.get("tool_call")!({ toolName: COMPARE_IMAGES_TOOL_NAME }, {})).toBeUndefined();
  });

  it("serves Chat's complete guidance only while the comparison tool is active", () => {
    const on = buildChatGuide({ activeTools: new Set([COMPARE_IMAGES_TOOL_NAME]), hasMcpTools: false, today: "Today" });
    for (const line of COMPARE_IMAGES_GUIDE_LINES) expect(on).toContain(line);
    expect(buildChatGuide({ activeTools: new Set(), hasMcpTools: false, today: "Today" })).not.toContain(COMPARE_IMAGES_TOOL_NAME);
  });
});
