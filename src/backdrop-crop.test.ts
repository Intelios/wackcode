import { describe, expect, it } from "vitest";
import { CENTRED, drawnSize, isCentred, pan, zoomTo } from "./backdrop-crop";

const frame = { width: 800, height: 500 };

describe("backdrop crop", () => {
  it("draws a wide picture at cover size, and taller for more zoom", () => {
    // 2:1 in a 1.6:1 window: height fits, width overflows.
    expect(drawnSize(frame, 2, 100)).toEqual({ width: 1000, height: 500 });
    expect(drawnSize(frame, 2, 200)).toEqual({ width: 2000, height: 1000 });
    // A tall picture fits the width instead.
    expect(drawnSize(frame, 0.5, 100)).toEqual({ width: 800, height: 1600 });
  });

  it("drags the picture with the pointer, clamped to its edges", () => {
    // 200px of horizontal overflow: dragging right by 100px reveals the left half of it.
    expect(pan(CENTRED, 100, 0, frame, 2).x).toBe(0);
    expect(pan(CENTRED, -50, 0, frame, 2).x).toBe(750);
    expect(pan(CENTRED, 9999, 0, frame, 2).x).toBe(0);
    expect(pan(CENTRED, -9999, 0, frame, 2).x).toBe(1000);
  });

  it("leaves an axis alone when the picture has no room to move on it", () => {
    const moved = pan(CENTRED, 40, 40, frame, 2);
    expect(moved.y).toBe(500);
    expect(moved.x).not.toBe(500);
  });

  it("zooms around the middle of the frame", () => {
    const zoomed = zoomTo(CENTRED, 200, frame, 2);
    expect(zoomed).toEqual({ zoom: 200, x: 500, y: 500 });
    // Off-centre: the point in the middle stays in the middle.
    const left = { zoom: 100, x: 0, y: 500 };
    const before = drawnSize(frame, 2, 100);
    const middle = (0 + frame.width / 2) / before.width;
    const out = zoomTo(left, 200, frame, 2);
    const after = drawnSize(frame, 2, 200);
    const shown = ((out.x / 1000) * (after.width - frame.width) + frame.width / 2) / after.width;
    expect(shown).toBeCloseTo(middle, 2);
  });

  it("keeps the zoom within range and recognises the uncropped state", () => {
    expect(zoomTo(CENTRED, 900, frame, 2).zoom).toBe(400);
    expect(zoomTo(CENTRED, 10, frame, 2).zoom).toBe(100);
    expect(isCentred(CENTRED)).toBe(true);
    expect(isCentred({ ...CENTRED, x: 400 })).toBe(false);
  });
});
