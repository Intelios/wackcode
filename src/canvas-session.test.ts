import { afterEach, describe, expect, it, vi } from "vitest";
import { createCanvasSession } from "./canvas-session";
import { cameraStorageKey } from "./canvas-camera";

afterEach(() => { vi.useRealTimers(); localStorage.clear(); });

describe("Canvas session", () => {
  it("keeps viewer state across remounts and saves the last camera after the board leaves", () => {
    vi.useFakeTimers();
    const session = createCanvasSession();
    expect(session.ensure("task", "board")).toMatchObject({ tool: "select", following: true, fitted: false });
    const camera = { x: 35, y: -12, zoom: 0.7 };
    session.update("board", { camera, selected: "phone", measured: { phone: 900 }, following: false, tool: "hand" });
    expect(session.ensure("task", "board")).toMatchObject({ camera, selected: "phone", measured: { phone: 900 }, following: false, tool: "hand" });
    vi.advanceTimersByTime(300);
    expect(JSON.parse(localStorage.getItem(cameraStorageKey("task", "board"))!)).toEqual(camera);
  });

  it("starts from a saved camera without refitting", () => {
    localStorage.setItem(cameraStorageKey("task", "board"), JSON.stringify({ x: 4, y: 5, zoom: 0.5 }));
    expect(createCanvasSession().ensure("task", "board")).toMatchObject({ camera: { x: 4, y: 5, zoom: 0.5 }, fitted: true });
  });

  it("tells listeners about changes, and ignores patches that change nothing", () => {
    const session = createCanvasSession();
    session.ensure("task", "board");
    const listener = vi.fn();
    session.subscribe(listener);
    session.update("board", { tool: "select" });
    expect(listener).not.toHaveBeenCalled();
    session.update("board", { tool: "hand" });
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
