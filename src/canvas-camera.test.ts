import { describe, expect, it } from "vitest";
import { MAX_ZOOM, MIN_ZOOM, cameraStorageKey, fitBounds, intersects, panBy, parseCamera, screenToWorld, zoomAt, zoomTo } from "./canvas-camera";

describe("canvas camera", () => {
  it("zooms about the pointer, keeping the world point under it fixed", () => {
    const camera = { x: 100, y: 50, zoom: 1 };
    const point = { x: 300, y: 200 };
    const before = screenToWorld(camera, point);
    const zoomed = zoomAt(camera, 2, point);
    expect(zoomed.zoom).toBe(2);
    expect(screenToWorld(zoomed, point)).toEqual(before);
  });

  it("clamps zoom both ways", () => {
    expect(zoomAt({ x: 0, y: 0, zoom: 1 }, 1000, { x: 0, y: 0 }).zoom).toBe(MAX_ZOOM);
    expect(zoomAt({ x: 0, y: 0, zoom: 1 }, 0.0001, { x: 0, y: 0 }).zoom).toBe(MIN_ZOOM);
    expect(zoomTo({ x: 0, y: 0, zoom: 0.5 }, 1, { width: 800, height: 600 })).toEqual({ x: -400, y: -300, zoom: 1 });
  });

  it("fits bounds centred without zooming in past the cap", () => {
    const fit = fitBounds({ x: 0, y: 0, width: 2000, height: 1000 }, { width: 1096, height: 696 }, 1, 48);
    expect(fit.zoom).toBe(0.5);
    expect(fit.x).toBe(1096 / 2 - 1000 * 0.5);
    expect(fit.y).toBe(696 / 2 - 500 * 0.5);
    expect(fitBounds({ x: 0, y: 0, width: 10, height: 10 }, { width: 1000, height: 1000 }).zoom).toBe(1);
  });

  it("pans, intersects and restores saved cameras", () => {
    expect(panBy({ x: 1, y: 2, zoom: 1 }, 10, -5)).toEqual({ x: 11, y: -3, zoom: 1 });
    expect(intersects({ x: 0, y: 0, zoom: 1 }, { x: 10, y: 10, width: 10, height: 10 }, { width: 100, height: 100 })).toBe(true);
    expect(intersects({ x: -500, y: 0, zoom: 1 }, { x: 10, y: 10, width: 10, height: 10 }, { width: 100, height: 100 })).toBe(false);
    expect(parseCamera({ x: 1, y: 2, zoom: 9 })).toEqual({ x: 1, y: 2, zoom: MAX_ZOOM });
    expect(parseCamera({ x: 1, y: "2", zoom: 1 })).toBeUndefined();
    expect(parseCamera({ x: 1, y: 2, zoom: 0 })).toBeUndefined();
    expect(parseCamera(null)).toBeUndefined();
    expect(cameraStorageKey("t1", "c1")).toBe("wackcode:canvas:t1:c1");
  });
});
