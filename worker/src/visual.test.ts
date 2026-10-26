import { describe, expect, it, vi } from "vitest";
import {
  MAX_VISUAL_HTML_BYTES, MAX_VISUAL_TITLE, PREVIEW_VISUAL_TOOL_NAME, SHOW_VISUAL_TOOL_NAME,
  VISUAL_GUIDE_LINES, VISUAL_TOOL_NAMES, createVisualExtension, formatSize,
  hasViewportHeightStyles, normalizeShowVisualParams,
} from "./builtin/visual.js";
import { CHAT_BUILTIN_TOOLS, chatToolAllowed } from "./builtin/chat-mode/policy.js";
import { buildChatGuide } from "./builtin/chat-mode/prompt.js";
import { createBuiltinExtensions, SWITCHABLE_BUILTIN_TOOLS } from "./builtin/index.js";
import { stripTailSections } from "./chat-history.js";
import { createPlanModeExtension } from "./builtin/plan-mode/index.js";

function harness(vision = true, canvas = vi.fn(async (request: { artboards: Array<{ id: string; width: number }> }) => ({
  artboards: request.artboards.map((item) => ({ ...item, contentHeight: item.width + 200, errors: [], image: { data: "cG5n", mimeType: "image/png" } })),
}))) {
  type Result = { content: Array<{ type: string; text?: string; data?: string }>; details: Record<string, unknown> };
  const tools = new Map<string, { execute: (id: string, params: unknown) => Promise<Result> }>();
  createVisualExtension({ canvas, workspace: () => undefined, supportsVision: () => vision, redact: (value: string) => value } as never)({
    registerTool: (tool: { name: string; execute: (id: string, params: unknown) => Promise<Result> }) => tools.set(tool.name, tool),
  } as never);
  return { show: tools.get(SHOW_VISUAL_TOOL_NAME)!, preview: tools.get(PREVIEW_VISUAL_TOOL_NAME)!, canvas };
}

describe("visual validation", () => {
  it("normalizes title whitespace and refuses missing/oversized values", () => {
    expect(normalizeShowVisualParams({ title: "  Latency   by region ", html: "<p>hi</p>" })).toEqual({ ok: true, title: "Latency by region", html: "<p>hi</p>" });
    for (const title of [undefined, "   ", "x".repeat(MAX_VISUAL_TITLE + 1)]) expect(normalizeShowVisualParams({ title, html: "<p/>" }).ok).toBe(false);
    expect(normalizeShowVisualParams({ title: "Chart", html: " \n " }).ok).toBe(false);
    expect(normalizeShowVisualParams(null).ok).toBe(false);
  });

  it("allows 128 KiB and counts UTF-8 bytes", () => {
    expect(MAX_VISUAL_HTML_BYTES).toBe(128 * 1024);
    expect(normalizeShowVisualParams({ title: "Chart", html: "é".repeat(MAX_VISUAL_HTML_BYTES / 2 + 1) })).toMatchObject({ ok: false, error: expect.stringContaining("over the 128.0 KB limit") });
    expect(normalizeShowVisualParams({ title: "Chart", html: "x".repeat(MAX_VISUAL_HTML_BYTES) }).ok).toBe(true);
  });

  it.each([
    '<style>main { height: 100vh }</style>', '<div style="min-height:20VH">',
    "<div style='height:calc(100vh - 10px)'>", '<div style=height:50vh>',
    '<style>.a{height:.5vh}.b{height:1e2vh}</style>',
  ])("rejects viewport heights in styles: %s", (html) => {
    expect(normalizeShowVisualParams({ title: "Chart", html })).toMatchObject({ ok: false, error: expect.stringContaining("fixed pixels") });
  });

  it.each([
    '<p>Use 100vh here</p><script>const a="100vh"</script>',
    '<img src="data:image/png;base64,abc123vhdef">',
    '<style>.a{background:url(data:image/png;base64,abc123vhdef);height:20px}</style>',
    '<div style="background:url(\'data:image/png;base64,abc123vhdef\');height:20px">',
    '<style>/* height:100vh */.vh{height:200px}</style>',
  ])("ignores prose, comments and base64: %s", (html) => expect(hasViewportHeightStyles(html)).toBe(false));

  it("formats payload sizes", () => {
    expect([512, 2048, 3 * 1024 * 1024].map(formatSize)).toEqual(["512 bytes", "2.0 KB", "3.0 MB"]);
  });
});

describe("visual tools", () => {
  it("measures all three widths without images and returns only identity and heights", async () => {
    const { show, canvas } = harness();
    const result = await show.execute("call", { title: "Throughput", html: "<p>42</p>" });
    expect(canvas).toHaveBeenCalledWith({ op: "render", images: false, artboards: [360, 560, 720].map((width) => ({ id: `visual-${width}`, kind: "preview", html: "<p>42</p>", width })) }, undefined);
    expect(result.details).toEqual({ v: 1, title: "Throughput", heights: [{ width: 360, height: 560 }, { width: 560, height: 760 }, { width: 720, height: 920 }] });
    expect(result.content[0].text).toContain("Don't announce or restate the page");
    expect(JSON.stringify(result)).not.toContain("<p>42</p>");
  });

  it("publishes without heights when rendering fails", async () => {
    const { show } = harness(true, vi.fn().mockRejectedValue(new Error("Render unavailable")));
    expect((await show.execute("call", { title: "Throughput", html: "<p>42</p>" })).details).toEqual({ v: 1, title: "Throughput" });
  });

  it("refuses invalid markup before calling the host", async () => {
    const { show, preview, canvas } = harness();
    await expect(show.execute("bad", { title: "", html: "<p/>" })).rejects.toThrow("title");
    await expect(preview.execute("bad", { html: '<style>p{height:100vh}</style>' })).rejects.toThrow("vh units");
    for (const width of [239, 1601, "720", NaN]) await expect(preview.execute("bad", { html: "<p/>", width })).rejects.toThrow("width");
    await expect(preview.execute("bad", { html: "<p/>", appearance: "blue" })).rejects.toThrow("appearance");
    expect(canvas).not.toHaveBeenCalled();
  });

  it.each([true, false])("preview returns images only for vision models (%s)", async (vision) => {
    const { preview, canvas } = harness(vision);
    const result = await preview.execute("preview", { html: "<p>hi</p>", width: 560, appearance: "light" });
    expect(canvas).toHaveBeenCalledWith({ op: "render", images: vision, appearance: "light", artboards: [{ id: "visual-preview", kind: "preview", html: "<p>hi</p>", width: 560 }] }, undefined);
    expect(result.details).toMatchObject({ v: 1, width: 560, contentHeight: 760, errors: [] });
    expect(result.content.some((block) => block.type === "image")).toBe(vision);
  });

  it("preview defaults to 720 px and reports redacted script errors", async () => {
    const canvas = vi.fn().mockResolvedValue({ artboards: [{ id: "visual-preview", contentHeight: 1234, errors: ["boom", 123] }] });
    const { preview } = harness(true, canvas);
    const result = await preview.execute("preview", { html: "<p/>" });
    expect(result.details).toMatchObject({ width: 720, contentHeight: 1234, errors: ["boom"] });
    expect(result.content[0].text).toContain("Script errors: boom");
  });
});

describe("visual policy wiring", () => {
  it("switches and allows both tools in Chat mode", () => {
    expect(VISUAL_TOOL_NAMES).toEqual([SHOW_VISUAL_TOOL_NAME, PREVIEW_VISUAL_TOOL_NAME]);
    for (const name of VISUAL_TOOL_NAMES) {
      expect(SWITCHABLE_BUILTIN_TOOLS.has(name)).toBe(true);
      expect(CHAT_BUILTIN_TOOLS.has(name)).toBe(true);
      expect(chatToolAllowed(name, "wackcode")).toBe(true);
    }
    const builtins = createBuiltinExtensions({} as never);
    for (const list of [builtins.factories, builtins.chatFactories]) expect(list.map((extension) => extension.name)).toContain("wackcode-visual");
  });

  it.each(["plan", "ultraplan"] as const)("allows previews in restricted %s", (mode) => {
    const handlers = new Map<string, (event: unknown, context: unknown) => unknown>();
    const { factory, controller } = createPlanModeExtension({ publishPlanState() {} } as never);
    factory({ registerTool() {}, appendEntry() {}, sendMessage() {}, on: (name: string, callback: (event: unknown, context: unknown) => unknown) => handlers.set(name, callback) } as never);
    controller.setMode(mode);
    for (const toolName of VISUAL_TOOL_NAMES) expect(handlers.get("tool_call")!({ toolName }, {})).toBeUndefined();
  });

  it("serves the complete shared guide only while visuals are active", () => {
    const on = buildChatGuide({ activeTools: new Set([SHOW_VISUAL_TOOL_NAME]), hasMcpTools: false, today: "Today" });
    for (const line of VISUAL_GUIDE_LINES) expect(on).toContain(line);
    expect(buildChatGuide({ activeTools: new Set(), hasMcpTools: false, today: "Today" })).not.toContain("Visual replies:");
  });
});

describe("read_chat rendering", () => {
  it("collapses a visual-sent section to a marker", () => {
    const text = 'Pick B\n\n<visual-action-v1>\nSent from the visual "Latency by region".\n</visual-action-v1>';
    expect(stripTailSections(text)).toBe("Pick B\n\n[sent from visual: Latency by region]");
    expect(stripTailSections("just words")).toBe("just words");
  });
});
