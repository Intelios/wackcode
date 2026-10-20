import { describe, expect, it } from "vitest";
import {
  fieldDensity, glyphFor, scanCloseness, scanHead, scrambleText, spectrumAt, ultraMarkFrame,
  ULTRA_CYCLE_MS, ULTRA_GLYPHS, ULTRA_NOISE, ULTRA_RING, ULTRA_SETTLED, ULTRA_STEP_MS, type Rgb
} from "./ultra-fx";

const brightest = (frame: number[]) => frame.reduce((best, value, cell) => (value > frame[best] ? cell : best), 0);
const lit = (frame: number[]) => frame.flatMap((value, cell) => (value === 1 ? [cell] : []));

describe("ultraMarkFrame", () => {
  it("walks the orbit's head round the ring in order, with the core lit", () => {
    for (let step = 0; step < ULTRA_RING.length * 2; step++) {
      const frame = ultraMarkFrame(step * ULTRA_STEP_MS + 1);
      expect(brightest(frame)).toBe(ULTRA_RING[step % ULTRA_RING.length]);
      expect(frame[4]).toBeGreaterThan(0.8);
    }
  });

  it("trails the head with two fading beads", () => {
    const frame = ultraMarkFrame(3 * ULTRA_STEP_MS + 1);
    expect(frame[ULTRA_RING[3]]).toBe(1);
    expect(frame[ULTRA_RING[2]]).toBeLessThan(1);
    expect(frame[ULTRA_RING[1]]).toBeLessThan(frame[ULTRA_RING[2]]);
    expect(frame[ULTRA_RING[0]]).toBeLessThan(frame[ULTRA_RING[1]]);
  });

  it("flashes a plus, flips to a cross, then settles into the wildcard", () => {
    const orbit = ULTRA_RING.length * 2 * ULTRA_STEP_MS;
    expect(lit(ultraMarkFrame(orbit + 10))).toEqual([1, 3, 4, 5, 7]);
    expect(lit(ultraMarkFrame(orbit + 370))).toEqual([0, 2, 4, 6, 8]);
    expect(ultraMarkFrame(orbit + 800)).toEqual([...ULTRA_SETTLED]);
  });

  it("repeats the cycle, so a mark that has run a while is back in the orbit", () => {
    expect(ultraMarkFrame(ULTRA_CYCLE_MS + 1)).toEqual(ultraMarkFrame(1));
    expect(ultraMarkFrame(-1)).toEqual([...ULTRA_SETTLED]);
  });

  it("settles on a bright plus over dimmer corners", () => {
    expect(ULTRA_SETTLED[4]).toBe(1);
    for (const edge of [1, 3, 5, 7]) expect(ULTRA_SETTLED[edge]).toBeGreaterThan(ULTRA_SETTLED[0]);
  });
});

describe("scrambleText", () => {
  it("shows noise before it starts and the label once done, keeping spaces", () => {
    const start = scrambleText("Ultra Plan", 0, () => 0);
    expect(start).toHaveLength("Ultra Plan".length);
    expect(start[5]).toBe(" ");
    expect([...start.replace(" ", "")].every((glyph) => ULTRA_NOISE.includes(glyph))).toBe(true);
    expect(scrambleText("Ultra Plan", 1)).toBe("Ultra Plan");
  });

  it("resolves from the left", () => {
    const halfway = scrambleText("Ultra", 0.5, () => 0);
    expect(halfway.startsWith("Ul")).toBe(true);
    expect(halfway.endsWith(ULTRA_NOISE[0])).toBe(true);
  });
});

describe("the glyph field", () => {
  it("maps density onto the ramp and clamps it", () => {
    expect(glyphFor(-1)).toBe(ULTRA_GLYPHS[0]);
    expect(glyphFor(0)).toBe(" ");
    expect(glyphFor(1)).toBe("@");
    expect(glyphFor(4)).toBe("@");
  });

  it("thickens toward the thumb and under the scan head", () => {
    expect(fieldDensity(0.9, 0.5, 0)).toBeGreaterThan(fieldDensity(0.1, 0.5, 0));
    expect(fieldDensity(0.4, 0.5, 1)).toBeGreaterThan(fieldDensity(0.4, 0.5, 0));
  });

  it("sweeps the head across the track and rests past its end", () => {
    expect(scanHead(0, 40)).toBe(0);
    expect(scanHead(1700 * 0.5, 40)).toBeCloseTo(20);
    expect(scanHead(1700 * 1.1, 40)).toBeGreaterThan(40);
    expect(scanCloseness(10, 10)).toBe(1);
    expect(scanCloseness(20, 10)).toBeLessThan(0.01);
  });

  it("runs the spectrum end to end and bounces back instead of jumping", () => {
    const stops: Rgb[] = [[0, 0, 0], [100, 100, 100], [200, 200, 200]];
    expect(spectrumAt(stops, 0)).toEqual([0, 0, 0]);
    expect(spectrumAt(stops, 0.5)).toEqual([100, 100, 100]);
    expect(spectrumAt(stops, 1)).toEqual([200, 200, 200]);
    expect(spectrumAt(stops, 1.5)).toEqual([100, 100, 100]);
    expect(spectrumAt(stops, 2)).toEqual([0, 0, 0]);
    expect(spectrumAt([], 0.3)).toEqual([255, 255, 255]);
  });
});
