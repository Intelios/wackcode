import { describe, expect, it } from "vitest";
import { COMPUTER_TOOL_NAMES, createComputerUseExtension, formatAct, formatApps } from "./builtin/computer-use/index.js";
import { PRUNED_SCREENSHOT_TEXT, pruneScreenshots, screenshotsToRemove } from "./builtin/computer-use/prune.js";
import type { BuiltinHost } from "./builtin/host.js";

type ToolResult = { content: Array<{ type: string; text?: string; data?: string; mimeType?: string }>; details: unknown };
type Tool = { name: string; executionMode?: string; execute: (...args: unknown[]) => Promise<ToolResult> };

function harness(vision = true, reply: (request: Record<string, unknown>) => unknown = () => ({})) {
  const requests: Record<string, unknown>[] = [];
  const tools = new Map<string, Tool>();
  const handlers = new Map<string, (event: { messages: unknown[] }) => unknown>();
  const host = {
    computer: async (request: Record<string, unknown>) => {
      requests.push(request);
      return reply(request);
    },
    supportsVision: () => vision,
    redact: (value: string) => value.replaceAll("sk-secret", "[redacted]"),
  } as unknown as BuiltinHost;
  const extension = createComputerUseExtension(host);
  extension.factory({
    registerTool: (tool: Tool) => tools.set(tool.name, tool),
    on: (event: string, handler: (event: { messages: unknown[] }) => unknown) => handlers.set(event, handler),
  } as never);
  return { requests, tools, handlers, controller: extension.controller };
}

describe("computer use built-in", () => {
  it("registers five sequential tools that stay inactive until switched on", async () => {
    const { tools, controller, requests } = harness();
    expect([...tools.keys()].sort()).toEqual([...COMPUTER_TOOL_NAMES].sort());
    expect([...tools.values()].every((tool) => tool.executionMode === "sequential")).toBe(true);
    expect(controller.inactiveTools().sort()).toEqual([...COMPUTER_TOOL_NAMES].sort());
    await expect(tools.get("computer_apps")!.execute("call", {}, undefined)).rejects.toThrow("switched off");
    expect(requests).toHaveLength(0);
    controller.configure(true);
    expect(controller.inactiveTools()).toEqual([]);
  });

  it("routes each tool to the native host with only the fields it takes", async () => {
    const { tools, controller, requests } = harness(true, (request) => request.op === "act" ? { results: [] } : { outline: "" });
    controller.configure(true);
    await tools.get("computer_open")!.execute("call", { app: " /Users/me/Build/My App.app " }, undefined);
    await tools.get("computer_snapshot")!.execute("call", { app: "TextEdit", window: 7 }, undefined);
    const actions = [{ kind: "press", ref: "e1-2" }];
    await tools.get("computer_act")!.execute("call", { app: "TextEdit", stateId: "s1", actions, expect: { titleContains: "Saved" } }, undefined);
    expect(requests).toEqual([
      { op: "open", app: "/Users/me/Build/My App.app" },
      { op: "snapshot", app: "TextEdit", window: 7 },
      { op: "act", app: "TextEdit", stateId: "s1", actions, expect: { titleContains: "Saved" } },
    ]);
    await expect(tools.get("computer_act")!.execute("call", { app: "TextEdit", stateId: "s1", actions: [] }, undefined)).rejects.toThrow("at least one action");
  });

  it("returns a screenshot only to vision models, and never keeps the image in details", async () => {
    const blind = harness(false);
    blind.controller.configure(true);
    const text = await blind.tools.get("computer_screenshot")!.execute("call", { app: "TextEdit" }, undefined);
    expect(text.content[0]?.text).toContain("can't receive screenshots");
    expect(blind.requests).toHaveLength(0);

    const seeing = harness(true, () => ({
      stateId: "s4",
      app: { name: "TextEdit", pid: 42 },
      window: { id: 7, title: "Untitled", width: 400, height: 300 },
      image: { mimeType: "image/jpeg", data: "anBn", width: 800, height: 600 },
      coordinateScale: 2,
    }));
    seeing.controller.configure(true);
    const shot = await seeing.tools.get("computer_screenshot")!.execute("call", { app: "TextEdit" }, undefined);
    expect(shot.content).toContainEqual({ type: "image", data: "anBn", mimeType: "image/jpeg" });
    expect(shot.content[0]?.text).toContain("stateId: s4");
    expect(JSON.stringify(shot.details)).not.toContain("anBn");
    expect(shot.details).toMatchObject({ app: "TextEdit", width: 800, coordinateScale: 2, stateId: "s4" });
  });

  it("redacts credentials from what the app showed", async () => {
    const { tools, controller } = harness(true, () => ({ stateId: "s1", app: { name: "App" }, window: { id: 1, title: "sk-secret" }, outline: "[e1-0] textField value=\"sk-secret\"" }));
    controller.configure(true);
    const result = await tools.get("computer_snapshot")!.execute("call", { app: "App" }, undefined);
    expect(result.content[0]?.text).not.toContain("sk-secret");
  });

  it("formats app lists and action results for the model", () => {
    expect(formatApps({ apps: [] })).toBe("No apps with windows are running.");
    expect(formatApps({
      apps: [{ name: "TextEdit", bundleId: "com.apple.TextEdit", pid: 42, access: "granted", frontmost: true, windows: [{ id: 7, title: "Untitled", width: 400, height: 300, main: true }] }],
    })).toBe('TextEdit — com.apple.TextEdit (pid 42) · granted · frontmost\n  window 7 "Untitled" 400×300 (main)');
    const text = formatAct({
      results: [{ index: 0, kind: "press", ok: true, via: "ax" }, { index: 1, kind: "click", ok: false, error: "Another app covers that point." }],
      expect: { met: false, observed: "no matching element" },
    });
    expect(text).toContain("1. press — ok via ax");
    expect(text).toContain("2. click — failed: Another app covers that point.");
    expect(text).toContain("expect: not met — no matching element");
  });

  it("prunes old screenshots from the model's context through the context hook", () => {
    const { handlers } = harness();
    const shot = (id: number) => ({ role: "toolResult", toolName: "computer_screenshot", content: [{ type: "text", text: `shot ${id}` }, { type: "image", data: `img${id}` }] });
    const messages = Array.from({ length: 8 }, (_, index) => shot(index));
    const result = handlers.get("context")!({ messages }) as { messages: typeof messages };
    const images = result.messages.flatMap((message) => message.content).filter((block) => block.type === "image");
    expect(images.map((block) => (block as { data: string }).data)).toEqual(["img5", "img6", "img7"]);
    expect(handlers.get("context")!({ messages: messages.slice(0, 3) })).toBeUndefined();
  });
});

describe("screenshot pruning", () => {
  const shot = (tool: string, id: string) => ({ role: "toolResult", toolName: tool, content: [{ type: "text", text: id }, { type: "image", data: id }] });
  const tools = new Set(["computer_screenshot"]);

  it("removes in whole chunks so the prompt prefix stays stable between them", () => {
    expect([0, 1, 3, 4, 7, 8, 12, 13].map((total) => screenshotsToRemove(total))).toEqual([0, 0, 0, 0, 0, 5, 5, 10]);
    const seven = Array.from({ length: 7 }, (_, index) => shot("computer_screenshot", `s${index}`));
    expect(pruneScreenshots(seven, tools)).toBeUndefined();
    const eight = [...seven, shot("computer_screenshot", "s7")];
    const pruned = pruneScreenshots(eight, tools)!;
    const texts = pruned.flatMap((message) => message.content).filter((block) => block.type === "text").map((block) => (block as { text: string }).text);
    expect(texts.filter((text) => text === PRUNED_SCREENSHOT_TEXT)).toHaveLength(5);
    // Adding one more within the chunk changes only the new tail, not what was pruned.
    const nine = pruneScreenshots([...eight, shot("computer_screenshot", "s8")], tools)!;
    expect(JSON.stringify(nine.slice(0, 8))).toBe(JSON.stringify(pruned));
  });

  it("leaves other tools' images alone and never mutates its input", () => {
    const messages = [
      ...Array.from({ length: 8 }, (_, index) => shot("computer_screenshot", `s${index}`)),
      shot("read", "file"),
      shot("browser_screenshot", "page"),
    ];
    const before = JSON.stringify(messages);
    const pruned = pruneScreenshots(messages, tools)!;
    expect(JSON.stringify(messages)).toBe(before);
    expect(pruned[8]).toBe(messages[8]);
    expect(pruned[9]).toBe(messages[9]);
    expect(pruned[7]).toBe(messages[7]);
    expect(pruned[0]).not.toBe(messages[0]);
  });
});
