import { describe, expect, it } from "vitest";
import { backgroundPhase, glyphGrid, isBackgroundClick } from "./dynamic-background";
import type { NormalizedMessage, TaskRuntime } from "./types";

const partial = (type: "thinking" | "text" | "tool-call", durationMs?: number): TaskRuntime => ({
  partial: { blocks: [{ type, durationMs }] } as NormalizedMessage
});

describe("selected chat background activity", () => {
  it("distinguishes live reasoning, answers, and tools without inspecting text", () => {
    expect(backgroundPhase({ status: "running", runtime: partial("thinking") })).toBe("thinking");
    expect(backgroundPhase({ status: "running", runtime: partial("thinking", 12) })).toBe("working");
    expect(backgroundPhase({ status: "running", runtime: partial("text") })).toBe("answering");
    expect(backgroundPhase({ status: "running", runtime: partial("tool-call") })).toBe("working");
    expect(backgroundPhase({ status: "running" })).toBe("working");
  });
  it("uses calm motion for input requests and Settings, even during a send", () => {
    expect(backgroundPhase({ status: "running", waiting: true, sending: true, runtime: partial("thinking") })).toBe("idle");
    expect(backgroundPhase({ status: "running", settings: true, runtime: partial("text") })).toBe("idle");
    expect(backgroundPhase({ sending: true })).toBe("working");
  });
  it("lets lifecycle signals override stale streamed blocks", () => {
    for (const status of ["idle", "stopping", "interrupted", "error"] as const) {
      expect(backgroundPhase({ status, runtime: partial("thinking") })).toBe("idle");
    }
    expect(backgroundPhase({ status: "running", runtime: { ...partial("thinking"), compaction: { reason: "manual", startedAt: 1_000 } } })).toBe("working");
    expect(backgroundPhase({ status: "running", runtime: { ...partial("text"), workActivity: { parent: "idle", subagents: 1, pendingResults: 0 } } })).toBe("background");
    expect(backgroundPhase({ status: "running", runtime: { workActivity: { parent: "waiting", subagents: 1, pendingResults: 0 } } })).toBe("working");
    expect(backgroundPhase({ status: "running", runtime: { workActivity: { parent: "idle", subagents: 0, pendingResults: 0 } } })).toBe("idle");
    expect(backgroundPhase({ status: "running", runtime: { ...partial("thinking"), activity: "starting", workActivity: { parent: "idle", subagents: 0, pendingResults: 0 } } })).toBe("working");
    // An idle destination does not inherit the previous chat's working state.
    expect(backgroundPhase({})).toBe("idle");
  });
});

it("bounds the glyph count on Retina, large displays and thin windows", () => {
  for (const [width, height] of [[1200, 800], [7680, 4320], [100, 10000], [1, 1]]) {
    const grid = glyphGrid(width, height);
    expect(grid.columns * grid.rows).toBeLessThanOrEqual(12000);
    expect(grid.columns * grid.cell).toBeGreaterThanOrEqual(width);
    expect(grid.rows * grid.cell * 1.6).toBeGreaterThanOrEqual(height);
  }
});

it("reacts only to designated empty space, never descendants or drag regions", () => {
  const space = document.createElement("div");
  space.setAttribute("data-dynamic-space", "");
  space.innerHTML = '<span>Text</span><button>Action</button><div data-tauri-drag-region><div data-dynamic-space></div></div><div role="dialog"><div data-dynamic-space></div></div>';
  expect(isBackgroundClick(space)).toBe(true);
  for (const child of space.querySelectorAll("*")) expect(isBackgroundClick(child)).toBe(false);
  expect(isBackgroundClick(null)).toBe(false);
});
