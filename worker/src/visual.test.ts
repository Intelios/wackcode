import { describe, expect, it } from "vitest";
import {
  MAX_VISUAL_HTML_BYTES,
  MAX_VISUAL_TITLE,
  SHOW_VISUAL_TOOL_NAME,
  VISUAL_GUIDE_LINES,
  VISUAL_TOOL_NAMES,
  createVisualExtension,
  formatSize,
  normalizeShowVisualParams,
} from "./builtin/visual.js";
import { CHAT_BUILTIN_TOOLS, chatToolAllowed } from "./builtin/chat-mode/policy.js";
import { buildChatGuide } from "./builtin/chat-mode/prompt.js";
import { createBuiltinExtensions, SWITCHABLE_BUILTIN_TOOLS } from "./builtin/index.js";
import { stripTailSections } from "./chat-history.js";

describe("show_visual params", () => {
  it("accepts a title and an HTML body, normalising the title's whitespace", () => {
    const parsed = normalizeShowVisualParams({ title: "  Latency   by region ", html: "<p>hi</p>" });
    expect(parsed).toEqual({ ok: true, title: "Latency by region", html: "<p>hi</p>" });
  });

  it("refuses a missing or over-long title, and an empty body", () => {
    expect(normalizeShowVisualParams({ html: "<p/>" })).toEqual({ ok: false, error: "title must be a short non-empty string." });
    expect(normalizeShowVisualParams({ title: "   ", html: "<p/>" })).toEqual({ ok: false, error: "title must be a short non-empty string." });
    expect(normalizeShowVisualParams({ title: "x".repeat(MAX_VISUAL_TITLE + 1), html: "<p/>" })).toEqual({
      ok: false,
      error: `title must be at most ${MAX_VISUAL_TITLE} characters.`,
    });
    expect(normalizeShowVisualParams({ title: "Chart", html: "  \n " })).toEqual({ ok: false, error: "html must be a non-empty HTML body." });
    expect(normalizeShowVisualParams(null)).toEqual({ ok: false, error: "Expected an object with a title and html body." });
  });

  it("caps the body, counting UTF-8 bytes rather than characters", () => {
    const tooBig = "é".repeat(MAX_VISUAL_HTML_BYTES / 2 + 1);
    const refused = normalizeShowVisualParams({ title: "Chart", html: tooBig });
    expect(refused.ok).toBe(false);
    expect(refused.ok === false && refused.error).toContain(`over the ${formatSize(MAX_VISUAL_HTML_BYTES)} limit`);
    expect(normalizeShowVisualParams({ title: "Chart", html: "x".repeat(MAX_VISUAL_HTML_BYTES) }).ok).toBe(true);
  });

  it("formats sizes the way the result and the refusal both quote them", () => {
    expect(formatSize(512)).toBe("512 bytes");
    expect(formatSize(2048)).toBe("2.0 KB");
    expect(formatSize(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("show_visual tool", () => {
  /** A minimal ExtensionAPI stand-in: the extension only registers one tool. */
  function register() {
    const registered: Array<{ name: string; description: string; parameters: unknown; execute: (id: string, params: unknown) => Promise<unknown> }> = [];
    const pi = { registerTool: (tool: (typeof registered)[number]) => registered.push(tool) };
    createVisualExtension()(pi as never);
    return registered[0];
  }

  it("registers one tool that returns the card's identity, not its markup", async () => {
    const tool = register();
    expect(tool.name).toBe(SHOW_VISUAL_TOOL_NAME);
    expect(tool.description).not.toContain("transcript");
    const result = (await tool.execute("call-1", { title: "Throughput", html: "<p>42</p>" })) as {
      content: Array<{ type: string; text: string }>;
      details: { v: number; title: string };
    };
    expect(result.details).toEqual({ v: 1, title: "Throughput" });
    expect(result.content[0].text).toContain('Shown "Throughput" inline in the conversation');
    expect(result.content[0].text).toContain("refer to it briefly");
    // The HTML stays in the call's arguments, which Pi saves; repeating it would double it.
    expect(JSON.stringify(result)).not.toContain("<p>42</p>");
  });

  it("fails a refused call, so the transcript shows the error instead of a card", async () => {
    const tool = register();
    await expect(tool.execute("call-2", { title: "", html: "<p/>" })).rejects.toThrow("title must be a short non-empty string.");
  });
});

describe("show_visual policy wiring", () => {
  it("is switchable, so Settings › Tools can take it away", () => {
    expect(SWITCHABLE_BUILTIN_TOOLS.has(SHOW_VISUAL_TOOL_NAME)).toBe(true);
    expect(VISUAL_TOOL_NAMES).toEqual([SHOW_VISUAL_TOOL_NAME]);
  });

  it("loads in both a code chat and a Chat mode chat", () => {
    const builtins = createBuiltinExtensions({} as never);
    const names = (list: { name: string }[]) => list.map((extension) => extension.name);
    expect(names(builtins.factories)).toContain("wackcode-visual");
    expect(names(builtins.chatFactories)).toContain("wackcode-visual");
  });

  it("is allowed in Chat mode, which has no path or process to widen", () => {
    expect(CHAT_BUILTIN_TOOLS.has(SHOW_VISUAL_TOOL_NAME)).toBe(true);
    expect(chatToolAllowed(SHOW_VISUAL_TOOL_NAME, "wackcode")).toBe(true);
  });

  it("reaches Chat mode's guide only while the tool is active", () => {
    const on = buildChatGuide({ activeTools: new Set([SHOW_VISUAL_TOOL_NAME]), hasMcpTools: false, today: "Today" });
    expect(on).toContain("Visual replies:");
    expect(on).toContain("wackcode.send(text)");
    const off = buildChatGuide({ activeTools: new Set(), hasMcpTools: false, today: "Today" });
    expect(off).not.toContain("Visual replies:");
    // The guide's words are the tool's own guidelines, so the two can never drift apart.
    for (const line of VISUAL_GUIDE_LINES) expect(on).toContain(line.split(" — ")[0].slice(0, 40));
  });
});

describe("read_chat rendering", () => {
  it("collapses a visual call to its title and a visual-sent section to a marker", () => {
    const text = "Pick B\n\n<visual-action-v1>\nSent from the visual \"Latency by region\".\n</visual-action-v1>";
    expect(stripTailSections(text)).toBe("Pick B\n\n[sent from visual: Latency by region]");
  });

  it("leaves an ordinary message alone", () => {
    expect(stripTailSections("just words")).toBe("just words");
  });
});
