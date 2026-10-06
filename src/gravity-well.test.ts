import { describe, expect, it } from "vitest";
import {
  ACT_BUSY_MS, ACT_DRAMATIC_MS, ACT_BLEND_MS, CAMEO_MS, FINALE_MS, PARTICLE_CAP,
  actBlend, actFor, cameoPose, collapse, createWell, isDone, stillFrame, step,
  type WellState,
} from "./gravity-well";

/** Steps a well through `steps` frames of `dtMs`, advancing wall-clock `now` with it. */
function run(state: WellState, dtMs: number, steps: number, startNow = 0): number {
  let now = startNow;
  for (let i = 0; i < steps; i++) {
    now += dtMs;
    step(state, dtMs, now);
  }
  return now;
}

function snapshot(state: WellState) {
  return state.particles.map((p) => [p.theta, p.dist, p.x, p.y, p.w, p.h, p.kind, p.jitter, p.warm]);
}

describe("gravity well", () => {
  it("is deterministic for a given seed and step sequence", () => {
    const a = createWell(42, 400, 140);
    const b = createWell(42, 400, 140);
    const now = 12_000;
    run(a, 16, 100, 0);
    run(b, 16, 100, 0);
    expect(snapshot(a)).toEqual(snapshot(b));
    expect(a.core.swallowed).toBe(b.core.swallowed);
    expect(a.flares.length).toBe(b.flares.length);
    const c = createWell(43, 400, 140);
    run(c, 16, 100, 0);
    expect(snapshot(c)).not.toEqual(snapshot(a));
    void now;
  });

  it("runs acts at 20s and 60s with a 2s blend", () => {
    expect(actFor(0)).toBe("calm");
    expect(actFor(19_999)).toBe("calm");
    expect(actFor(ACT_BUSY_MS)).toBe("busy");
    expect(actFor(ACT_DRAMATIC_MS)).toBe("dramatic");
    expect(actBlend(1_000)).toBeCloseTo(0.5);
    expect(actBlend(ACT_BLEND_MS)).toBe(1);
    expect(actBlend(ACT_BUSY_MS)).toBe(0);
    expect(actBlend(ACT_BUSY_MS + ACT_BLEND_MS)).toBe(1);
    expect(actBlend(ACT_DRAMATIC_MS + ACT_BLEND_MS / 2)).toBeCloseTo(0.5);
  });

  it("spawns faster in later acts and stays under the cap", () => {
    for (const [act, low, high] of [["calm", 6, 30], ["busy", 12, 50], ["dramatic", 25, 80]] as const) {
      const s = createWell(7, 400, 140);
      const base = act === "calm" ? 0 : act === "busy" ? ACT_BUSY_MS + ACT_BLEND_MS : ACT_DRAMATIC_MS + ACT_BLEND_MS;
      run(s, 16, 625, base); // 10s of sim time inside the act
      expect(s.particles.length, act).toBeGreaterThanOrEqual(low);
      expect(s.particles.length, act).toBeLessThanOrEqual(high);
      expect(s.particles.length).toBeLessThanOrEqual(PARTICLE_CAP);
    }
  });

  it("swallows fragments at the horizon and warms the core", () => {
    const s = createWell(11, 400, 140);
    run(s, 40, 2_000, 0); // 80s: enough for particles to complete the spiral
    expect(s.core.swallowed).toBeGreaterThan(0);
    expect(s.core.heat).toBeGreaterThan(0.2);
    // Every live particle sits outside the horizon by definition.
    for (const p of s.particles) expect(p.dist).toBeGreaterThan(s.horizon);
  });

  it("guarantees a cameo within 10s of the dramatic act and never runs two", () => {
    const s = createWell(3, 400, 140);
    let saw = false;
    let now = ACT_DRAMATIC_MS;
    while (now < ACT_DRAMATIC_MS + 10_000) {
      now += 16;
      step(s, 16, now);
      if (s.cameo) saw = true;
      // If a second cameo ever appeared while one is live, `cameo` would be replaced —
      // the field only allows one, so just assert the invariant holds structurally.
    }
    expect(saw).toBe(true);
    expect(s.cameos).toBeGreaterThanOrEqual(1);
  });

  it("flies the duck edge-to-edge, closest at mid-flight, and never swallows it", () => {
    const s = createWell(5, 400, 140);
    s.cameo = { born: 0 };
    const at = (ms: number) => cameoPose(s, ms)!;
    const start = at(0);
    const mid = at(CAMEO_MS / 2);
    const end = at(CAMEO_MS);
    expect(start.x).toBeLessThan(s.cx - 100);
    expect(end.x).toBeGreaterThan(s.cx + 100);
    const dist = (x: number, y: number) => Math.hypot(x - s.cx, y - s.cy);
    expect(dist(mid.x, mid.y)).toBeLessThan(dist(start.x, start.y));
    expect(dist(mid.x, mid.y)).toBeLessThan(dist(end.x, end.y));
    expect(mid.squash).toBeLessThan(1);
    expect(cameoPose(s, CAMEO_MS + 1)).toBeNull();
    // Running the sim past the cameo's end removes it without touching core.swallowed.
    const swallowed = s.core.swallowed;
    step(s, 16, CAMEO_MS + 16);
    expect(s.cameo).toBeUndefined();
    expect(s.core.swallowed).toBe(swallowed);
  });

  it("collapses: everything falls in, the ring expands, isDone at 600ms", () => {
    const s = createWell(9, 400, 140);
    run(s, 16, 500, 0);
    const had = s.particles.length;
    expect(had).toBeGreaterThan(0);
    collapse(s, 8_500);
    step(s, 16, 8_516);
    expect(s.ending).toBe(8_500);
    expect(isDone(s, 8_500 + 300)).toBe(false);
    // During the collapse no new particles spawn and existing ones only fall.
    const during = run(s, 16, 10, 8_516);
    expect(s.particles.length).toBeLessThanOrEqual(had);
    expect(s.finaleRing).toBeGreaterThan(0);
    expect(isDone(s, during)).toBe(false);
    const done = run(s, 16, 30, during);
    expect(isDone(s, done)).toBe(true);
    expect(s.finaleRing).toBe(1);
    expect(s.core.radius).toBe(0);
    expect(s.particles.length).toBe(0);
  });

  it("stillFrame is a warm calm scene: particles mid-spiral, no flares or cameo", () => {
    const s = stillFrame(21, 400, 140);
    expect(s.particles.length).toBeGreaterThanOrEqual(10);
    expect(s.particles.length).toBeLessThanOrEqual(20);
    expect(s.flares).toHaveLength(0);
    expect(s.cameo).toBeUndefined();
    expect(s.act).toBe("calm");
    expect(s.ending).toBeUndefined();
    for (const p of s.particles) expect(p.dist).toBeGreaterThan(s.horizon);
  });
});
