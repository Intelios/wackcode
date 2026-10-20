import { describe, expect, it } from "vitest";
import foldCases from "./builtin/canvas/fold-cases.json" with { type: "json" };
import {
  CANVAS_GUIDE_TOOL_NAME,
  CANVAS_LOOK_PARAMS,
  CANVAS_LOOK_TOOL_NAME,
  CANVAS_TOOL_NAME,
  CANVAS_TOOL_NAMES,
  createCanvasExtension,
} from "./builtin/canvas/index.js";
import {
  MAX_ARTBOARDS_PER_CALL,
  MAX_ARTBOARDS_PER_CANVAS,
  MAX_ARTBOARD_HTML_BYTES,
  type Canvas,
  type CanvasDetails,
  canvasesFromBranch,
  planCall,
  resolveArtboardHtml,
  slugify,
} from "./builtin/canvas/state.js";
import { CANVAS_GUIDE_SECTIONS, buildCanvasPrompt, canvasGuide } from "./builtin/canvas/guide.js";
import { createBuiltinExtensions, SWITCHABLE_BUILTIN_TOOLS } from "./builtin/index.js";
import { CHAT_BUILTIN_TOOLS, chatToolAllowed } from "./builtin/chat-mode/policy.js";
import { APP_COMMAND_NAMES } from "./slash.js";

type Call = { args: unknown; details: CanvasDetails };

/** Accepted calls shaped as Pi saves them: the assistant's call, then its result. */
function branch(calls: Call[], extra: unknown[] = []) {
  return [
    ...calls.flatMap((call, index) => [
      { type: "message", message: { role: "assistant", content: [{ type: "toolCall", id: `call-${index}`, name: CANVAS_TOOL_NAME, arguments: call.args }] } },
      { type: "message", message: { role: "toolResult", toolName: CANVAS_TOOL_NAME, toolCallId: `call-${index}`, details: call.details } },
    ]),
    ...extra,
  ];
}

/** Plans each call against what the branch so far holds, as the tool does. */
function accept(inputs: unknown[]): Call[] {
  const calls: Call[] = [];
  for (const args of inputs) {
    const planned = planCall(canvasesFromBranch(branch(calls)), args);
    if (!planned.ok) throw new Error(planned.error);
    calls.push({ args, details: planned.details });
  }
  return calls;
}

function board(canvas: Canvas) {
  return {
    title: canvas.title,
    lastChanged: canvas.lastChanged,
    artboards: canvas.artboards.map((artboard) => ({
      id: artboard.id,
      name: artboard.name,
      group: artboard.group ?? null,
      width: artboard.width,
      height: artboard.height ?? null,
      html: resolveArtboardHtml(canvas, artboard),
    })),
  };
}

const page = (text: string) => `<main>${text}</main>`;

describe("canvas fold (shared with the renderer)", () => {
  for (const testCase of foldCases.cases) {
    it(testCase.name, () => {
      const canvases = canvasesFromBranch(branch(testCase.calls as Call[]));
      expect(Object.fromEntries([...canvases].map(([id, canvas]) => [id, board(canvas)]))).toEqual(testCase.expect);
    });

    it(`${testCase.name}: the tool plans the same details`, () => {
      expect(accept(testCase.calls.map((call) => call.args)).map((call) => call.details)).toEqual(testCase.calls.map((call) => call.details));
    });
  }

  it("ignores refused calls, other tools and calls with no saved arguments", () => {
    const [call] = accept([{ title: "One", artboards: [{ name: "A", width: 400, html: page("a") }] }]);
    const refused = { type: "message", message: { role: "toolResult", toolName: CANVAS_TOOL_NAME, toolCallId: "x", isError: true, details: call.details } };
    const other = { type: "message", message: { role: "toolResult", toolName: "todo", toolCallId: "y", details: call.details } };
    expect([...canvasesFromBranch(branch([], [refused, other, { type: "session" }])).keys()]).toEqual([]);
    expect(canvasesFromBranch(branch([call])).get("one")?.artboards).toHaveLength(1);
  });

  it("follows a rewind: a branch without the later call has the earlier board", () => {
    const calls = accept([
      { title: "One", artboards: [{ name: "A", width: 400, html: page("a") }] },
      { canvas: "one", artboards: [{ name: "B", width: 400, html: page("b") }] },
    ]);
    expect(canvasesFromBranch(branch(calls)).get("one")?.artboards.map((artboard) => artboard.id)).toEqual(["a", "b"]);
    expect(canvasesFromBranch(branch(calls.slice(0, 1))).get("one")?.artboards.map((artboard) => artboard.id)).toEqual(["a"]);
  });
});

describe("canvas planning", () => {
  const start = { title: "Settings", artboards: [{ name: "Base", width: 1440, height: 900, html: page("base") }] };

  it("starts a canvas with a slug id and dedupes ids", () => {
    const calls = accept([start, { title: "Settings", artboards: [{ name: "Base", width: 800, html: page("x") }, { name: "Base", width: 800, html: page("y") }] }]);
    expect(calls[0].details).toMatchObject({ canvas: "settings", applied: [{ id: "base", op: "add" }] });
    expect(calls[1].details).toMatchObject({ canvas: "settings-2", applied: [{ id: "base" }, { id: "base-2" }] });
    expect(slugify("Ünïcode — Dusk!")).toBe("unicode-dusk");
    expect(slugify("!!!")).toBe("artboard");
  });

  it("needs a title or a known canvas, and something to do", () => {
    const empty = new Map<string, Canvas>();
    expect(planCall(empty, { artboards: [] })).toEqual({ ok: false, error: "Give a title to start a new canvas, or the id of an existing one in canvas." });
    expect(planCall(empty, { canvas: "nope", artboards: [] })).toEqual({ ok: false, error: "There is no canvas “nope” in this chat. Omit canvas to start one." });
    expect(planCall(empty, { title: "Empty" })).toEqual({ ok: false, error: "A new canvas needs at least one artboard." });
    const canvases = canvasesFromBranch(branch(accept([start])));
    expect(planCall(canvases, { canvas: "settings" })).toMatchObject({ ok: false, error: expect.stringContaining("Nothing to change") });
    // A rename alone is a change.
    expect(planCall(canvases, { canvas: "settings", title: "Account settings" })).toMatchObject({ ok: true, details: { title: "Account settings", applied: [] } });
  });

  it("validates new artboards", () => {
    const empty = new Map<string, Canvas>();
    const one = (artboard: Record<string, unknown>) => planCall(empty, { title: "T", artboards: [artboard] });
    expect(one({ width: 400, html: page("x") })).toMatchObject({ ok: false, error: "artboards[0] needs a name." });
    expect(one({ name: "A", html: page("x") })).toMatchObject({ ok: false, error: expect.stringContaining("needs a width") });
    expect(one({ name: "A", width: 400 })).toMatchObject({ ok: false, error: expect.stringContaining("needs html, or basedOn") });
    expect(one({ name: "A", width: 100, html: page("x") })).toMatchObject({ ok: false, error: expect.stringContaining("width must be between") });
    expect(one({ name: "A", width: 400, height: 9000, html: page("x") })).toMatchObject({ ok: false, error: expect.stringContaining("height must be between") });
    expect(one({ name: "A", width: 400, html: "x".repeat(MAX_ARTBOARD_HTML_BYTES + 1) })).toMatchObject({ ok: false, error: expect.stringContaining("over the 64 KiB limit") });
    expect(one({ name: "A", width: "wide", html: page("x") })).toMatchObject({ ok: false, error: "artboards[0].width must be a number of pixels." });
    const many = Array.from({ length: MAX_ARTBOARDS_PER_CALL + 1 }, (_, index) => ({ name: `A${index}`, width: 400, html: page("x") }));
    expect(planCall(empty, { title: "T", artboards: many })).toMatchObject({ ok: false, error: expect.stringContaining(`at most ${MAX_ARTBOARDS_PER_CALL} artboards`) });
  });

  it("caps a call's total markup and a canvas's artboard count", () => {
    const big = "x".repeat(60 * 1024);
    const heavy = Array.from({ length: 4 }, (_, index) => ({ name: `A${index}`, width: 400, html: big }));
    expect(planCall(new Map(), { title: "T", artboards: heavy })).toMatchObject({ ok: false, error: expect.stringContaining("over the 192 KiB limit") });
    const rounds = Array.from({ length: MAX_ARTBOARDS_PER_CANVAS / MAX_ARTBOARDS_PER_CALL }, (_, round) => ({
      ...(round === 0 ? { title: "Full" } : { canvas: "full" }),
      artboards: Array.from({ length: MAX_ARTBOARDS_PER_CALL }, (_, index) => ({ name: `R${round} A${index}`, width: 400, html: page("x") })),
    }));
    const canvases = canvasesFromBranch(branch(accept(rounds)));
    expect(canvases.get("full")?.artboards).toHaveLength(MAX_ARTBOARDS_PER_CANVAS);
    expect(planCall(canvases, { canvas: "full", artboards: [{ name: "One more", width: 400, html: page("x") }] }))
      .toMatchObject({ ok: false, error: expect.stringContaining(`at most ${MAX_ARTBOARDS_PER_CANVAS} artboards`) });
  });

  it("draws a new variant at its base's size unless it gives a width of its own", () => {
    // A live run (2026-10-20) left width off its theme variants and had the call refused.
    const [call] = accept([{
      title: "T",
      artboards: [
        { name: "Base", width: 1280, height: 1100, html: page("b") },
        { name: "Dusk", basedOn: "base", css: "a{}" },
        { name: "Phone", width: 390, basedOn: "base" },
      ],
    }]);
    expect(call.details.applied).toEqual([
      { index: 0, id: "base", op: "add" },
      { index: 1, id: "dusk", op: "add", basedOn: "base", width: 1280, height: 1100 },
      { index: 2, id: "phone", op: "add", basedOn: "base" },
    ]);
    const canvas = canvasesFromBranch(branch([call])).get("t")!;
    expect(canvas.artboards.map((artboard) => [artboard.id, artboard.width, artboard.height ?? "auto"])).toEqual([["base", 1280, 1100], ["dusk", 1280, 1100], ["phone", 390, "auto"]]);
    // A plain artboard still needs its width.
    expect(planCall(new Map(), { title: "T", artboards: [{ name: "A", html: page("a") }] })).toMatchObject({ ok: false, error: expect.stringContaining("needs a width") });
  });

  it("revises only the fields given, and refuses an unknown id", () => {
    const calls = accept([start, { canvas: "settings", artboards: [{ id: "base", height: 1200 }] }]);
    const canvas = canvasesFromBranch(branch(calls)).get("settings")!;
    expect(canvas.artboards[0]).toMatchObject({ id: "base", name: "Base", width: 1440, height: 1200, html: page("base") });
    expect(planCall(canvasesFromBranch(branch(calls)), { canvas: "settings", artboards: [{ id: "ghost", height: 900 }] }))
      .toMatchObject({ ok: false, error: "There is no artboard “ghost” on “Settings”. Omit id to add a new one." });
  });

  it("keeps variants one level deep and their bases in place", () => {
    const calls = accept([{
      title: "Themes",
      artboards: [
        { name: "Base", width: 800, html: page("base") },
        { name: "Dark", width: 800, basedOn: "base", css: ":root{--bg:#000}" },
      ],
    }]);
    const canvases = canvasesFromBranch(branch(calls));
    expect(planCall(canvases, { canvas: "themes", artboards: [{ name: "Darker", width: 800, basedOn: "dark" }] }))
      .toMatchObject({ ok: false, error: expect.stringContaining("which is itself a variant") });
    expect(planCall(canvases, { canvas: "themes", artboards: [{ name: "X", width: 800, basedOn: "nope" }] }))
      .toMatchObject({ ok: false, error: expect.stringContaining("which is not on “Themes”") });
    expect(planCall(canvases, { canvas: "themes", artboards: [{ name: "Y", width: 800, html: page("y"), basedOn: "base" }] }))
      .toMatchObject({ ok: false, error: expect.stringContaining("gives both html and basedOn") });
    expect(planCall(canvases, { canvas: "themes", remove: ["base"] }))
      .toMatchObject({ ok: false, error: expect.stringContaining("can't be removed while “dark” is a variant of it") });
    expect(planCall(canvases, { canvas: "themes", remove: ["base", "dark"] })).toMatchObject({ ok: true, details: { removed: ["base", "dark"] } });
    expect(planCall(canvases, { canvas: "themes", artboards: [{ id: "base", basedOn: "dark" }] }))
      .toMatchObject({ ok: false, error: expect.stringContaining("which is itself a variant") });
    expect(planCall(canvases, { canvas: "themes", remove: ["ghost"] })).toMatchObject({ ok: false, error: "remove names “ghost”, which is not on “Themes”." });
  });
});

/** A minimal ExtensionAPI stand-in. */
function register(host: Record<string, unknown> = {}) {
  const tools: Array<{ name: string; promptGuidelines?: string[]; execute: (id: string, params: unknown, signal?: unknown, onUpdate?: unknown, ctx?: unknown) => Promise<unknown> }> = [];
  const pi = { registerTool: (tool: (typeof tools)[number]) => tools.push(tool), on: () => {} };
  createCanvasExtension({ supportsVision: () => true, redact: (text: string) => text, ...host } as never)(pi as never);
  return Object.fromEntries(tools.map((tool) => [tool.name, tool]));
}

type ToolResult = { content: Array<{ type: string; text?: string; data?: string }>; details: Record<string, unknown> };

describe("canvas tools", () => {
  const start = { title: "Settings", artboards: [{ name: "Base", group: "Themes", width: 1440, height: 900, html: page("base") }] };

  it("registers three tools that share their guidelines", () => {
    const tools = register();
    expect(Object.keys(tools)).toEqual([CANVAS_GUIDE_TOOL_NAME, CANVAS_TOOL_NAME, CANVAS_LOOK_TOOL_NAME]);
    expect(CANVAS_TOOL_NAMES).toEqual([CANVAS_TOOL_NAME, CANVAS_GUIDE_TOOL_NAME, CANVAS_LOOK_TOOL_NAME]);
    expect(tools.canvas.promptGuidelines).toEqual(tools.canvas_look.promptGuidelines);
  });

  it("draws, lists every id for the model, and keeps the markup out of the result", async () => {
    const tools = register();
    const result = await tools.canvas.execute("call-0", start, undefined, undefined, { sessionManager: { getBranch: () => [] } }) as ToolResult;
    expect(result.details).toEqual({ v: 1, canvas: "settings", title: "Settings", applied: [{ index: 0, id: "base", op: "add" }], removed: [] });
    expect(result.content[0].text).toContain("Started canvas “Settings” (id: settings).");
    expect(result.content[0].text).toContain("Added: base — “Base” [Themes] 1440×900.");
    expect(JSON.stringify(result)).not.toContain("<main>");
    await expect(tools.canvas.execute("call-1", { artboards: [] })).rejects.toThrow("Give a title to start a new canvas");
  });

  it("serves the guide whole or by section", async () => {
    const tools = register();
    const whole = await tools.canvas_guide.execute("g", {}) as ToolResult;
    for (const section of ["## Basics", "## Exploring directions", "## Theme explorations", "## Fidelity to the project", "## Craft", "## Checking your work", "## Example"]) {
      expect(whole.content[0].text).toContain(section);
    }
    const part = await tools.canvas_guide.execute("g", { sections: ["themes"] }) as ToolResult;
    expect(part.content[0].text).toContain("## Theme explorations");
    expect(part.content[0].text).not.toContain("## Basics");
    expect(canvasGuide(["nope"])).toBe("# Canvas guide");
    expect(CANVAS_GUIDE_SECTIONS).toHaveLength(7);
  });

  it("looks at the artboards changed last, with screenshots and a report", async () => {
    const requests: unknown[] = [];
    const tools = register({
      canvas: async (request: { artboards: Array<{ id: string; width: number; height?: number }> }) => {
        requests.push(request);
        return { artboards: request.artboards.map((artboard) => ({ id: artboard.id, width: artboard.width, height: artboard.height ?? 600, contentWidth: artboard.width, contentHeight: 1000, errors: ["TypeError: x is undefined"], image: { data: "UE5H", mimeType: "image/png" } })) };
      },
    });
    const calls = accept([start, { canvas: "settings", artboards: [{ name: "Dark", group: "Themes", width: 1440, height: 900, basedOn: "base", css: ":root{--bg:#000}" }] }]);
    const ctx = { sessionManager: { getBranch: () => branch(calls) } };
    const result = await tools.canvas_look.execute("look", { html: true }, undefined, undefined, ctx) as ToolResult;
    expect(requests).toEqual([{ op: "render", images: true, artboards: [{ id: "dark", html: `${page("base")}\n<style>:root{--bg:#000}</style>`, width: 1440, height: 900 }] }]);
    const text = result.content.filter((block) => block.type === "text").map((block) => block.text).join("\n");
    expect(text).toContain("Content overflows the bottom by 100 px.");
    expect(text).toContain("Script errors: TypeError: x is undefined");
    expect(text).toContain("```html\n<main>base</main>");
    expect(result.content.filter((block) => block.type === "image")).toEqual([{ type: "image", data: "UE5H", mimeType: "image/png" }]);
    expect(result.details).toEqual({ v: 1, canvas: "settings", artboards: [{ id: "dark", width: 1440, height: 900, overflowX: 0, overflowY: 100, errors: ["TypeError: x is undefined"] }] });
  });

  it("reports without screenshots for a model that can't see images", async () => {
    const tools = register({
      supportsVision: () => false,
      canvas: async (request: { images: boolean; artboards: Array<{ id: string; width: number }> }) => ({
        artboards: request.artboards.map((artboard) => ({ id: artboard.id, width: artboard.width, height: 700, contentHeight: 700, errors: [], ...(request.images ? { image: { data: "UE5H" } } : {}) })),
      }),
    });
    const ctx = { sessionManager: { getBranch: () => branch(accept([{ title: "One", artboards: [{ name: "A", width: 400, html: page("a") }] }])) } };
    const result = await tools.canvas_look.execute("look", { artboards: ["A"] }, undefined, undefined, ctx) as ToolResult;
    expect(result.content.some((block) => block.type === "image")).toBe(false);
    expect(result.content[0].text).toContain("can't receive images");
    expect(result.details).toMatchObject({ unsupported: "vision" });
  });

  it("looks at four at a time and names the rest instead of refusing", async () => {
    // A live run (2026-10-20) asked for all five artboards and Pi refused the call on maxItems.
    const requests: Array<{ artboards: Array<{ id: string }> }> = [];
    const tools = register({
      canvas: async (request: { artboards: Array<{ id: string; width: number }> }) => {
        requests.push(request);
        return { artboards: request.artboards.map((artboard) => ({ id: artboard.id, width: artboard.width, height: 600, contentHeight: 600, errors: [] })) };
      },
    });
    const five = Array.from({ length: 5 }, (_, index) => ({ name: `A${index}`, width: 400, html: page(`a${index}`) }));
    const ctx = { sessionManager: { getBranch: () => branch(accept([{ title: "Five", artboards: five }])) } };
    const result = await tools.canvas_look.execute("look", { artboards: ["a0", "a1", "a2", "a3", "a4"] }, undefined, undefined, ctx) as ToolResult;
    expect(requests[0].artboards.map((artboard) => artboard.id)).toEqual(["a0", "a1", "a2", "a3"]);
    expect(result.content[0].text).toContain("look again for: a4.");
    const defaults = await tools.canvas_look.execute("look", {}, undefined, undefined, ctx) as ToolResult;
    expect(defaults.content[0].text).toContain("look again for: a4.");
    expect(JSON.stringify(CANVAS_LOOK_PARAMS)).not.toContain("maxItems");
  });

  it("says which canvas when there is none or several", async () => {
    const tools = register({ canvas: async () => ({ artboards: [] }) });
    const none = { sessionManager: { getBranch: () => [] } };
    await expect(tools.canvas_look.execute("look", {}, undefined, undefined, none)).rejects.toThrow("There is no canvas in this chat yet.");
    const two = { sessionManager: { getBranch: () => branch(accept([start, { title: "Other", artboards: [{ name: "A", width: 400, html: page("a") }] }])) } };
    await expect(tools.canvas_look.execute("look", {}, undefined, undefined, two)).rejects.toThrow("This chat has 2 canvases; say which one: settings, other.");
    await expect(tools.canvas_look.execute("look", { canvas: "settings", artboards: ["ghost"] }, undefined, undefined, two)).rejects.toThrow("There is no artboard “ghost” on “Settings”.");
  });
});

describe("canvas wiring", () => {
  it("is switchable and loads only in code chats", () => {
    for (const name of CANVAS_TOOL_NAMES) {
      expect(SWITCHABLE_BUILTIN_TOOLS.has(name)).toBe(true);
      expect(CHAT_BUILTIN_TOOLS.has(name)).toBe(false);
      expect(chatToolAllowed(name, "wackcode")).toBe(false);
    }
    const builtins = createBuiltinExtensions({} as never);
    const names = (list: { name: string }[]) => list.map((extension) => extension.name);
    expect(names(builtins.factories)).toContain("wackcode-canvas");
    expect(names(builtins.chatFactories)).not.toContain("wackcode-canvas");
  });

  it("reserves /canvas and builds its prompt around the whole guide", () => {
    expect(APP_COMMAND_NAMES).toContain("canvas");
    const prompt = buildCanvasPrompt("  explore four themes  ");
    // A live run (2026-10-20) edited the project's tokens.css mid-exploration.
    expect(prompt).toContain("change none of its files");
    expect(prompt).toContain("Exploring changes nothing in the project.");
    expect(prompt).toContain("The user's request:\n\nexplore four themes");
    expect(prompt).toContain("## Theme explorations");
    expect(buildCanvasPrompt("")).toContain("ask what they want to explore");
  });
});
