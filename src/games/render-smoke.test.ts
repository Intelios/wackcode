import { describe, expect, it } from "vitest";
import { render } from "./doodle-render";
import { createDoodle, step, SKY_Y, STARS_Y, STEP_MS } from "./doodle";

/** A canvas stand-in that records every call: enough shape for the renderer to run against. */
function stubCtx(): CanvasRenderingContext2D {
  const gradient = { addColorStop: () => {} };
  return new Proxy({}, {
    get: (_t, prop) => {
      if (prop === "createLinearGradient" || prop === "createRadialGradient") return () => gradient;
      if (prop === "measureText") return () => ({ width: 10 });
      if (typeof prop === "string" && prop !== "globalAlpha") return () => {};
      return undefined;
    },
    set: () => true
  }) as unknown as CanvasRenderingContext2D;
}

const palette = {
  accent: "#c2ee4a", onAccent: "#172000", text: "#e8ece4", textSoft: "#adb5a7", textDim: "#757e70",
  well: "#0c0e0b", surface: "#161914", border: "#2b3028", danger: "#f08080"
};

describe("render smoke", () => {
  it("draws a run in every zone without touching the state", () => {
    const ctx = stubCtx();
    expect(() => render(ctx, null, palette, { w: 400, h: 600, reduced: false })).not.toThrow();

    const s = createDoodle(11, { w: 400, h: 600 });
    expect(() => render(ctx, s, palette, { w: 400, h: 600, reduced: false })).not.toThrow();

    // Ride a bubble through sky and stars, rendering along the way with and without particles.
    s.duck.bubble = 60_000;
    for (let i = 0; i < 5000 && s.duck.bubble > 0; i++) {
      step(s, { x: Math.sin(i * 0.02) }, STEP_MS);
      if (i % 97 === 0) render(ctx, s, palette, { w: 400, h: 600, reduced: i % 194 === 0 });
    }
    expect(s.duck.y).toBeGreaterThan(STARS_Y + 500);
    expect(s.zone).toBe(2);
    expect(() => render(ctx, s, palette, { w: 400, h: 600, reduced: true })).not.toThrow();

    // A lost run and a pond-level run draw too.
    s.phase = "lost";
    expect(() => render(ctx, s, palette, { w: 400, h: 600, reduced: false })).not.toThrow();
    const pond = createDoodle(3);
    pond.duck.y = SKY_Y / 2;
    expect(() => render(ctx, pond, palette, { w: 300, h: 500, reduced: false })).not.toThrow();
    expect(pond.duck.y).toBe(SKY_Y / 2);
  });
});
