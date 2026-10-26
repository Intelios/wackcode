import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

// Exercise the actual sandbox bridge while the compositor is paused, as WebKit may do for
// an iframe held at opacity: 0 until its first size report arrives.
const documentSource = readFileSync(new URL("../../src-tauri/src/visual_frame.html", import.meta.url), "utf8");
const bridge = documentSource.match(/<script>([\s\S]*?)<\/script>/)![1];

function frame(kind: "reply" | "preview" = "reply") {
  const posts: Array<{ type: string; height: number }> = [];
  const windowHandlers: Record<string, () => void> = {};
  const documentHandlers: Record<string, () => void> = {};
  const timers: Array<() => void> = [];
  const rafs: Array<() => void> = [];
  const root = { scrollHeight: 900, offsetHeight: 0, scrollWidth: 720, getAttribute: () => kind };
  const document = {
    documentElement: root,
    body: null as { scrollHeight: number; scrollWidth: number } | null,
    addEventListener: (type: string, callback: () => void) => { documentHandlers[type] = callback; },
  };
  const window = {
    parent: { postMessage: (message: { type: string; height: number }) => posts.push(message) },
    requestAnimationFrame: vi.fn((callback: () => void) => { rafs.push(callback); return rafs.length; }),
    setTimeout: vi.fn((callback: () => void) => { timers.push(callback); return timers.length; }),
    addEventListener: (type: string, callback: () => void) => { windowHandlers[type] = callback; },
  };
  runInNewContext(bridge, { window, document });
  return {
    posts, window, windowHandlers, documentHandlers,
    content(height: number) { root.offsetHeight = height; document.body = { scrollHeight: height, scrollWidth: 720 }; },
    flushTimers() { while (timers.length) timers.shift()!(); },
    flushFrame() { rafs.shift()?.(); },
  };
}

describe("visual frame first-size reports", () => {
  it.each(["reply", "preview"] as const)("reports %s content on DOMContentLoaded with rAF paused", (kind) => {
    const visual = frame(kind);
    visual.content(120);
    visual.documentHandlers.DOMContentLoaded();
    expect(visual.posts).toEqual([{ type: "wackcode-visual:size", height: 120, width: 720 }]);
    expect(visual.window.requestAnimationFrame).not.toHaveBeenCalled();
  });

  it("reports loaded image dimensions directly while rAF is paused", () => {
    const visual = frame();
    visual.flushTimers();
    visual.content(480);
    visual.windowHandlers.load();
    expect(visual.posts[0]).toMatchObject({ type: "wackcode-visual:size", height: 480 });
    expect(visual.window.requestAnimationFrame).not.toHaveBeenCalled();
  });

  it("uses a timer when content arrives after an empty document has loaded", () => {
    const visual = frame();
    visual.flushTimers();
    visual.documentHandlers.DOMContentLoaded();
    visual.windowHandlers.load();
    expect(visual.posts).toEqual([]);
    visual.content(240);
    visual.windowHandlers.resize();
    visual.flushTimers();
    expect(visual.posts[0]).toMatchObject({ height: 240 });
    expect(visual.window.requestAnimationFrame).not.toHaveBeenCalled();
  });

  it("continues coalescing later size changes into animation frames", () => {
    const visual = frame();
    visual.content(120);
    visual.documentHandlers.DOMContentLoaded();
    visual.flushTimers();
    visual.content(360);
    visual.windowHandlers.resize();
    visual.windowHandlers.resize();
    expect(visual.window.requestAnimationFrame).toHaveBeenCalledTimes(1);
    expect(visual.posts).toHaveLength(1);
    visual.flushFrame();
    expect(visual.posts[1]).toMatchObject({ height: 360 });
  });
});
