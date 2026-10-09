import { describe, expect, it } from "vitest";
import {
  BUBBLE_MS, BUBBLE_POP_V, BUBBLE_V, CRUMBLE_MS, DUCK_R, GRAVITY, JUMP_V, MAX_BUGS, MAX_FALL, MAX_ITEMS,
  MAX_PARTICLES, MAX_PLATFORMS, SKY_Y, SPRING_V, STARS_Y, START_Y, STEP_MS, createDoodle, difficultyAt,
  maxGapAt, rand, score, step, type Banner, type Bug, type DoodleState, type Platform
} from "./doodle";

/** A world with nothing in it and nothing being generated: tests place their own fixtures. */
function bare(seed = 1, view = { w: 400, h: 600 }): DoodleState {
  const s = createDoodle(seed, view);
  s.platforms.length = 0;
  s.bugs.length = 0;
  s.items.length = 0;
  s.genY = 10_000;
  s.genX = view.w / 2;
  return s;
}

function platformAt(s: DoodleState, x: number, y: number, w = 100, kind: Platform["kind"] = "static", spring = false): Platform {
  const p: Platform = { id: s.nextId++, kind, x, y, w, vx: kind === "moving" ? 50 : 0, spring, crumble: -1 };
  s.platforms.push(p);
  return p;
}

function bugAt(s: DoodleState, x: number, y: number): Bug {
  const bug: Bug = { id: s.nextId++, x, y, minX: x - 40, maxX: x + 40, vx: 40 };
  s.bugs.push(bug);
  return bug;
}

/** Drops the duck from `above` onto whatever is below, stepping until it bounces (or gives up). */
function fallUntilBounce(s: DoodleState, above: number, maxSteps = 240): boolean {
  s.duck.y = above;
  s.duck.vy = 0;
  for (let i = 0; i < maxSteps; i++) {
    step(s, { x: 0 }, STEP_MS);
    if (s.duck.vy > 0) return true;
  }
  return false;
}

describe("doodle simulation", () => {
  it("replays the same run from the same seed and inputs", () => {
    // A simple bot steers toward the next pad up, so the run climbs for a long while.
    const steer = (s: ReturnType<typeof createDoodle>) => {
      let target: number | null = null;
      let best = Infinity;
      for (const p of s.platforms) {
        if (p.y <= s.duck.y + DUCK_R) continue;
        if (p.y - s.duck.y < best) { best = p.y - s.duck.y; target = p.x + p.w / 2; }
      }
      if (target === null) return { x: 0 };
      let dx = target - s.duck.x;
      if (Math.abs(dx) > s.view.w / 2) dx -= Math.sign(dx) * s.view.w;
      return { x: Math.max(-1, Math.min(1, dx / 20)) };
    };
    const run = (seed: number) => {
      const s = createDoodle(seed, { w: 400, h: 600 });
      for (let i = 0; i < 4000 && s.phase === "playing"; i++) {
        step(s, steer(s), STEP_MS);
      }
      return s;
    };
    const a = run(42);
    const b = run(42);
    expect(a.peak).toBeGreaterThan(START_Y + 500);
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
    expect(JSON.stringify(run(43))).not.toBe(JSON.stringify(a));
  });

  it("bounces off a pad at the jump speed, feet on its surface", () => {
    const s = bare();
    platformAt(s, 150, 200);
    expect(fallUntilBounce(s, 200 + DUCK_R + 60)).toBe(true);
    expect(s.duck.vy).toBe(JUMP_V);
    expect(s.duck.y).toBeCloseTo(200 + DUCK_R, 5);
  });

  it("bounces while rising past nothing, and only while falling onto a pad", () => {
    const s = bare();
    platformAt(s, 150, 200);
    // Rising from below: the pad must not catch the duck.
    s.duck.x = 200;
    s.duck.y = 180;
    s.duck.vy = JUMP_V;
    step(s, { x: 0 }, STEP_MS);
    expect(s.duck.y).toBeGreaterThan(180);
    expect(s.duck.vy).toBeLessThan(JUMP_V);
  });

  it("wraps horizontally around the view", () => {
    const s = bare();
    s.duck.bubble = 60_000; // Rise out of danger: this test is only about x.
    s.duck.x = 395;
    step(s, { x: 1 }, 1000);
    expect(s.duck.x).toBeCloseTo(295, 5);
    s.duck.x = 5;
    step(s, { x: -1 }, 1000);
    expect(s.duck.x).toBeCloseTo(105, 5);
  });

  it("springs give a super-jump", () => {
    const s = bare();
    platformAt(s, 150, 200, 100, "static", true);
    expect(fallUntilBounce(s, 200 + DUCK_R + 60)).toBe(true);
    expect(s.duck.vy).toBe(SPRING_V);
  });

  it("crumbling pads hold exactly one bounce, then go", () => {
    const s = bare();
    const pad = platformAt(s, 150, 200, 100, "crumbling");
    expect(fallUntilBounce(s, 200 + DUCK_R + 60)).toBe(true);
    expect(pad.crumble).toBeGreaterThanOrEqual(0);
    expect(pad.crumble).toBeLessThanOrEqual(CRUMBLE_MS);
    for (let i = 0; i < Math.ceil(CRUMBLE_MS / STEP_MS) + 2; i++) step(s, { x: 0 }, STEP_MS);
    expect(s.platforms).not.toContain(pad);
  });

  it("moving pads slide and turn at the view's sides", () => {
    const s = bare();
    const pad = platformAt(s, 380, 200, 60, "moving");
    pad.vx = 80;
    for (let i = 0; i < 60; i++) step(s, { x: 0 }, STEP_MS);
    expect(pad.x).toBeLessThanOrEqual(400 - 6 - pad.w);
    expect(pad.vx).toBeLessThan(0);
  });

  it("stomps a bug from above and bounces off it", () => {
    const s = bare();
    bugAt(s, 200, 250);
    expect(fallUntilBounce(s, 250 + DUCK_R + 40)).toBe(true);
    expect(s.bugs).toHaveLength(0);
    expect(s.squash).toBe(1);
    expect(s.duck.vy).toBe(JUMP_V);
  });

  it("a bug touched from the side ends the run", () => {
    const s = bare();
    bugAt(s, 200, 250);
    s.duck.x = 200;
    s.duck.y = 258;
    s.duck.vy = 40;
    step(s, { x: 0 }, STEP_MS);
    expect(s.phase).toBe("lost");
    expect(s.squash).toBe(0);
  });

  it("the bubble lifts the duck, sweeping bugs aside, then pops", () => {
    const s = bare();
    s.items.push({ x: s.duck.x, y: s.duck.y });
    step(s, { x: 0 }, STEP_MS);
    expect(s.duck.bubble).toBeGreaterThan(0);
    const rise = s.duck.y;
    step(s, { x: 0 }, STEP_MS);
    expect(s.duck.vy).toBe(BUBBLE_V);
    expect(s.duck.y).toBeGreaterThan(rise);

    bugAt(s, s.duck.x, s.duck.y + 16);
    step(s, { x: 0 }, STEP_MS);
    expect(s.phase).toBe("playing");
    expect(s.squash).toBe(1);

    const before = s.duck.y;
    for (let i = 0; i < Math.ceil(BUBBLE_MS / STEP_MS) + 2; i++) step(s, { x: 0 }, STEP_MS);
    expect(s.duck.bubble).toBe(0);
    expect(s.duck.y).toBeGreaterThan(before + 1000);
    expect(s.duck.vy).toBeLessThanOrEqual(BUBBLE_POP_V);
  });

  it("scores the metres climbed and only the metres climbed", () => {
    const s = bare();
    s.duck.y = START_Y + 12 * 10;
    s.peak = s.duck.y;
    expect(score(s)).toBe(10);
    // Falling never unwinds the score.
    s.duck.y = START_Y;
    step(s, { x: 0 }, STEP_MS);
    expect(score(s)).toBe(10);
    expect(score({ ...s, peak: START_Y })).toBe(0);
  });

  it("ends the run when the duck falls off the bottom, and freezes time after", () => {
    const s = bare();
    s.duck.y = 100;
    s.cameraY = 300;
    step(s, { x: 0 }, STEP_MS);
    expect(s.phase).toBe("lost");
    const time = s.time;
    step(s, { x: 0 }, STEP_MS);
    expect(s.time).toBe(time);
  });

  it("raises the camera under a climbing duck and never lets it fall", () => {
    const s = bare();
    s.duck.bubble = BUBBLE_MS;
    const startY = s.duck.y;
    for (let i = 0; i < 120; i++) step(s, { x: 0 }, STEP_MS);
    expect(s.duck.y).toBeGreaterThan(startY + 900);
    expect(s.cameraY).toBeGreaterThan(600);
    const camera = s.cameraY;
    s.duck.bubble = 0;
    s.duck.vy = -MAX_FALL;
    step(s, { x: 0 }, STEP_MS);
    expect(s.cameraY).toBe(camera);
  });

  it("generates gaps that stay reachable at every altitude", () => {
    for (const altitude of [0, 2_000, 10_000, 18_000, 60_000]) {
      expect(maxGapAt(altitude)).toBeLessThan((JUMP_V * JUMP_V) / (2 * GRAVITY) - 20);
    }
    // A long generated ladder: ride a bubble up and inspect what the generator built.
    const s = createDoodle(9, { w: 400, h: 600 });
    s.duck.bubble = 60_000;
    for (let i = 0; i < 4000 && s.duck.bubble > 0; i++) step(s, { x: 0 }, STEP_MS);
    const sorted = [...s.platforms].sort((a, b) => a.y - b.y);
    expect(sorted.length).toBeGreaterThan(6);
    for (let i = 1; i < sorted.length; i++) {
      expect(sorted[i].y - sorted[i - 1].y).toBeLessThanOrEqual(maxGapAt(sorted[i - 1].y) + 0.001);
      expect(sorted[i].w).toBeGreaterThanOrEqual(30);
    }
    expect(difficultyAt(60_000)).toBe(1);
  });

  it("announces each zone once, on first crossing", () => {
    const s = bare();
    const banner = (): Banner | null => s.banner;
    s.duck.y = SKY_Y + 400;
    s.duck.vy = 0;
    step(s, { x: 0 }, STEP_MS);
    expect(s.zone).toBe(1);
    expect(banner()?.text).toBe("The sky opens up");
    s.banner = null;
    // Still in the sky, having never left: no second announcement.
    s.duck.y = SKY_Y + 400;
    step(s, { x: 0 }, STEP_MS);
    expect(s.zone).toBe(1);
    expect(banner()).toBeNull();
    s.duck.y = STARS_Y + 400;
    step(s, { x: 0 }, STEP_MS);
    expect(s.zone).toBe(2);
    expect(banner()?.text).toBe("The stars come out");
  });

  it("keeps every list inside its cap on a long run", () => {
    const s = createDoodle(5, { w: 400, h: 600 });
    for (let i = 0; i < 6000 && s.phase === "playing"; i++) step(s, { x: Math.sin(i * 0.007) }, STEP_MS);
    expect(s.platforms.length).toBeLessThanOrEqual(MAX_PLATFORMS);
    expect(s.bugs.length).toBeLessThanOrEqual(MAX_BUGS);
    expect(s.items.length).toBeLessThanOrEqual(MAX_ITEMS);
    expect(s.particles.length).toBeLessThanOrEqual(MAX_PARTICLES);
  });

  it("prunes what fell below the camera", () => {
    const s = bare();
    platformAt(s, 150, 200);
    bugAt(s, 200, 220);
    s.cameraY = 600;
    s.duck.y = 900;
    s.duck.vy = 0;
    step(s, { x: 0 }, STEP_MS);
    expect(s.platforms).toHaveLength(0);
    expect(s.bugs).toHaveLength(0);
  });

  it("collides across the wrap seam, where the duck is drawn on both sides", () => {
    const s = bare();
    // A pad hugging the left edge; the duck falls at the right edge, its wrapped copy over it.
    platformAt(s, 6, 200, 60);
    s.duck.x = 399;
    s.duck.y = 200 + DUCK_R + 40;
    s.duck.vy = 0;
    let bounced = false;
    for (let i = 0; i < 120 && !bounced; i++) {
      step(s, { x: 0 }, STEP_MS);
      bounced = s.duck.vy > 0;
    }
    expect(bounced).toBe(true);
    expect(s.duck.x).toBeGreaterThan(390);

    // And a bug beside the seam kills the wrapped copy just the same.
    const t = bare();
    bugAt(t, 8, 150);
    t.duck.x = 398;
    t.duck.y = 150;
    t.duck.vy = 100;
    step(t, { x: 0 }, STEP_MS);
    expect(t.phase).toBe("lost");
  });

  it("keeps the state JSON-safe and the rng working", () => {
    const s = createDoodle(3);
    for (let i = 0; i < 600; i++) step(s, { x: 0.3 }, STEP_MS);
    expect(() => JSON.stringify(s)).not.toThrow();
    const revived: DoodleState = JSON.parse(JSON.stringify(s));
    expect(revived.duck.facing).toBe(s.duck.facing);
    const value = rand(s);
    expect(value).toBeGreaterThanOrEqual(0);
    expect(value).toBeLessThan(1);
    expect(() => JSON.stringify(bugAt(bare(), 10, 10))).not.toThrow();
  });
});
