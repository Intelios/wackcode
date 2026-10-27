import { afterEach, describe, expect, it, vi } from "vitest";
import { createCanvasSession } from "./canvas-session";
import { cameraStorageKey } from "./canvas-camera";

afterEach(() => { vi.useRealTimers(); localStorage.clear(); });

describe("Canvas session", () => {
  it("keeps camera updates independent of ownership locks and saves the last camera after a host leaves", () => {
    vi.useFakeTimers();
    const session = createCanvasSession();
    session.ensure("task", "board");
    const version = session.getVersion();
    const camera = { x: 35, y: -12, zoom: 0.7 };
    session.update("board", { camera, selected: "phone", measured: { phone: 900 }, following: false });
    expect(session.getVersion()).toBe(version);
    session.activate("board");
    expect(session.isLocked("board")).toBe(true);
    expect(session.getVersion()).toBeGreaterThan(version);
    session.deactivate();
    expect(session.isLocked("board")).toBe(false);
    expect(session.get("board")).toMatchObject({ camera, selected: "phone", measured: { phone: 900 }, following: false });
    vi.advanceTimersByTime(300);
    expect(JSON.parse(localStorage.getItem(cameraStorageKey("task", "board"))!)).toEqual(camera);
  });

  it("locks expansion even before its host mounts and releases it without discarding viewer state", () => {
    const session = createCanvasSession();
    session.setExpanded("board");
    expect(session.isLocked("board")).toBe(true);
    expect(session.ensure("task", "board").expanded).toBe(true);
    session.update("board", { selected: "desktop" });
    session.setExpanded(undefined);
    expect(session.isLocked("board")).toBe(false);
    expect(session.get("board")!.selected).toBe("desktop");
  });
});
