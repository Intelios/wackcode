import { describe, expect, it } from "vitest";
import type { CanvasTouch } from "./canvas-state";
import { canvasAnchor, canvasOwners } from "./canvas-placement";

const touch = (canvas: string): CanvasTouch => ({ canvas, title: canvas, toolCallId: canvas, added: [], updated: [], removed: [] });
const turn = (userKey: string, ...canvases: string[]) => ({ userKey, canvases: canvases.map(touch) });

describe("canvas placement", () => {
  it("places each board at its last modifying turn, independently of other canvases", () => {
    expect([...canvasOwners([turn("first", "a", "b"), turn("second", "a"), turn("third")])]).toEqual([["a", "second"], ["b", "first"]]);
  });

  it("holds interaction at its surviving occurrence and relocates once unlocked", () => {
    const previous = new Map([["a", "first"]]);
    const turns = [turn("first", "a"), turn("second", "a")];
    expect(canvasOwners(turns, previous, () => true).get("a")).toBe("first");
    expect(canvasOwners(turns, previous, () => false).get("a")).toBe("second");
  });

  it("recomputes ownership on rewinds and never holds a retired branch occurrence", () => {
    const previous = new Map([["a", "second"], ["gone", "old"]]);
    expect([...canvasOwners([turn("first", "a")], previous, () => true)]).toEqual([["a", "first"]]);
    expect([...canvasOwners([], previous, () => true)]).toEqual([]);
  });

  it("gives each turn a stable, unambiguous occurrence anchor", () => {
    expect(canvasAnchor("first", "a")).toBe(canvasAnchor("first", "a"));
    expect(canvasAnchor("first", "a")).not.toBe(canvasAnchor("second", "a"));
    expect(canvasAnchor("first:a", "b")).not.toBe(canvasAnchor("first", "a:b"));
  });
});
