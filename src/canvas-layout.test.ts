import { describe, expect, it } from "vitest";
import { ARTBOARD_GAP, AUTO_HEIGHT, GROUP_BAND, LABEL_BAND, ROW_GAP, boundsOf, layoutCanvas } from "./canvas-layout";
import type { CanvasArtboard, CanvasBoard } from "./canvas-state";

const board = (artboards: CanvasArtboard[]): CanvasBoard => ({ id: "c", title: "C", artboards, lastChanged: [] });
const art = (id: string, group: string | undefined, width: number, height?: number): CanvasArtboard => ({ id, name: id, width, height, ...(group ? { group } : {}) });

describe("layoutCanvas", () => {
  it("puts each group in a row, in order of first appearance", () => {
    const layout = layoutCanvas(board([art("a", "Themes", 1280, 800), art("p", "Phone", 390, 844), art("b", "Themes", 1280, 800)]));
    const top = GROUP_BAND + LABEL_BAND;
    expect(layout.artboards).toEqual([
      { id: "a", x: 0, y: top, width: 1280, height: 800 },
      { id: "b", x: 1280 + ARTBOARD_GAP, y: top, width: 1280, height: 800 },
      { id: "p", x: 0, y: top + 800 + ROW_GAP + top, width: 390, height: 844 },
    ]);
    expect(layout.groups.map((group) => group.label)).toEqual(["Themes", "Phone"]);
    expect(layout.bounds).toEqual({ x: 0, y: 0, width: 1280 * 2 + ARTBOARD_GAP, height: top + 800 + ROW_GAP + top + 844 });
  });

  it("never moves existing artboards sideways when one is added", () => {
    const before = layoutCanvas(board([art("a", "A", 400, 300), art("b", "B", 400, 300)]));
    const after = layoutCanvas(board([art("a", "A", 400, 300), art("b", "B", 400, 300), art("c", "A", 400, 300)]));
    for (const id of ["a", "b"]) {
      expect(after.artboards.find((item) => item.id === id)?.x).toBe(before.artboards.find((item) => item.id === id)?.x);
    }
  });

  it("draws ungrouped artboards in an unlabelled row and auto heights from measurements", () => {
    const layout = layoutCanvas(board([art("a", undefined, 600), art("b", undefined, 600)]), { b: 1234 });
    expect(layout.groups).toEqual([{ x: 0, y: 0, width: 1200 + ARTBOARD_GAP }]);
    expect(layout.artboards.map((item) => [item.y, item.height])).toEqual([[LABEL_BAND, AUTO_HEIGHT], [LABEL_BAND, 1234]]);
    expect(layoutCanvas(board([art("a", undefined, 600)]), { a: 90_000 }).artboards[0].height).toBe(4000);
  });

  it("bounds a selection, labels included", () => {
    expect(boundsOf([])).toBeUndefined();
    expect(boundsOf([{ id: "a", x: 10, y: 100, width: 50, height: 40 }, { id: "b", x: 100, y: 120, width: 20, height: 80 }]))
      .toEqual({ x: 10, y: 100 - LABEL_BAND, width: 110, height: 200 - (100 - LABEL_BAND) });
  });
});
