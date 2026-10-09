/**
 * Doodle Duck: the Games panel's vertical jumper, a Doodle Jump in the duck's world. The duck
 * bounces off lily pads, clouds and starlit ledges, ever upward through three altitude zones;
 * steering is horizontal only, and falling off the bottom of the view ends the run. Springs give
 * a super-jump, crumbling pads hold exactly one bounce, bugs must be stomped from above, and a
 * bubble power-up carries the duck through three screens of sky, untouchable. The score is
 * height: metres climbed, nothing else.
 *
 * This module is the whole simulation and nothing else: plain mutable state, no DOM, no clock.
 * `DoodleDuck.tsx` steps it at `STEP_MS` from a rAF accumulator and `doodle-render.ts` draws it.
 * `doodle-save.ts` persists the state verbatim, so every field here must stay JSON-safe and any
 * new field needs a default in the save normaliser.
 *
 * Coordinates: world units with y growing upward (altitude) and x wrapping across the view's
 * width, Doodle Jump-style — leaving one side enters at the other. `cameraY` is the altitude at
 * the bottom edge of the view; the renderer flips y to the canvas.
 *
 * Invariants:
 * - Deterministic: all randomness comes from the seeded generator in `state.rng`, so the same
 *   seed and inputs replay the same run (the tests rely on it).
 * - Every generated gap is reachable: `maxGapAt` stays under the jump's apex with room to spare.
 * - `step` does nothing once the run has ended.
 * - Lists are capped (`MAX_PLATFORMS`, `MAX_BUGS`, `MAX_ITEMS`, `MAX_PARTICLES`) and pruned
 *   below the camera, so a long climb stays cheap.
 * - The state is JSON-safe: `games/session.ts` keeps it across panel remounts and
 *   `games/doodle-save.ts` across app launches.
 */

export const STEP_MS = 1000 / 60;

// ── Physics ─────────────────────────────────────────
export const GRAVITY = 1500;
/** Launch speed of an ordinary bounce: an apex of about 128 px. */
export const JUMP_V = 620;
/** Spring launch: about two thirds of a screen. */
export const SPRING_V = 1180;
/** The bubble's steady rise, and how long it lasts: roughly three screens. */
export const BUBBLE_V = 560;
export const BUBBLE_MS = 3400;
/** The little hop the duck keeps when the bubble pops. */
export const BUBBLE_POP_V = 340;
export const MOVE_SPEED = 300;
export const MAX_FALL = 1000;

export const DUCK_R = 13;
export const BUG_R = 10;
export const ITEM_R = 11;
export const PLATFORM_H = 9;
export const CRUMBLE_MS = 320;

export const MAX_PLATFORMS = 26;
export const MAX_BUGS = 8;
export const MAX_ITEMS = 3;
export const MAX_PARTICLES = 220;

/** One metre of altitude in pixels; the score counts these. */
export const METRE_PX = 12;
/** The altitude the duck starts at, so a fresh run scores zero. */
export const START_Y = 96;
/** How long a banner shows. */
export const BANNER_MS = 2400;

/** Where the world changes: the pond ends, then the stars begin. */
export const SKY_Y = 2600;
export const STARS_Y = 9000;
export type Zone = "pond" | "sky" | "stars";

export function zoneOf(y: number): Zone {
  return y < SKY_Y ? "pond" : y < STARS_Y ? "sky" : "stars";
}

export type PlatformKind = "static" | "moving" | "crumbling";
export type Tone = "accent" | "danger" | "text";

export interface Platform {
  id: number;
  kind: PlatformKind;
  /** Left edge; the top surface is `y`. */
  x: number;
  y: number;
  w: number;
  /** `moving` only, px/s; bounces at the view's sides. */
  vx: number;
  spring: boolean;
  /** ms of crumble left, or -1 while intact (one bounce starts it). */
  crumble: number;
}

export interface Bug {
  id: number;
  x: number;
  y: number;
  minX: number;
  maxX: number;
  vx: number;
}

/** A floating bubble power-up, waiting to be touched. */
export interface Item { x: number; y: number }

export interface Particle { x: number; y: number; vx: number; vy: number; life: number; max: number; size: number; tone: Tone }
export interface Banner { text: string; until: number }

export interface Input { x: number }

export interface DoodleState {
  rng: number;
  nextId: number;
  /** Simulated ms since the run began. */
  time: number;
  phase: "playing" | "lost";
  view: { w: number; h: number };
  duck: {
    x: number;
    y: number;
    vy: number;
    facing: 1 | -1;
    /** ms of bubble lift left; 0 while plain physics rule. */
    bubble: number;
  };
  /** Altitude at the bottom edge of the view; only ever rises. */
  cameraY: number;
  /** The highest altitude the duck's centre has reached. */
  peak: number;
  platforms: Platform[];
  bugs: Bug[];
  items: Item[];
  particles: Particle[];
  /** Surface altitude of the highest generated platform. */
  genY: number;
  /** Centre x of the last generated platform, so the ladder keeps a shape. */
  genX: number;
  /** Bugs stomped or bubbled through. */
  squash: number;
  /** The highest zone reached: banners mark each first crossing. */
  zone: 0 | 1 | 2;
  banner: Banner | null;
}

// ── Difficulty ──────────────────────────────────────
// Everything hard ramps with altitude and tops out near 1500 m, so a long run stops getting
// harder and settles into its top speed.

/** 0 at the pond's surface to 1 near the top of the ramp. */
export function difficultyAt(altitude: number): number {
  return Math.min(1, Math.max(0, altitude / 18_000));
}

/** The largest gap generated at an altitude: always well under the 128 px jump apex. */
export function maxGapAt(altitude: number): number {
  return 56 + 48 * difficultyAt(altitude);
}

export function score(s: DoodleState): number {
  return Math.max(0, Math.floor((s.peak - START_Y) / METRE_PX));
}

// ── Setup ───────────────────────────────────────────

export function createDoodle(seed: number, view = { w: 420, h: 640 }): DoodleState {
  const s: DoodleState = {
    rng: seed >>> 0 || 1,
    nextId: 1,
    time: 0,
    phase: "playing",
    view: { ...view },
    duck: { x: view.w / 2, y: START_Y, vy: 0, facing: 1, bubble: 0 },
    cameraY: 0,
    peak: START_Y,
    platforms: [],
    bugs: [],
    items: [],
    particles: [],
    genY: START_Y - DUCK_R,
    genX: view.w / 2,
    squash: 0,
    zone: 0,
    banner: null
  };
  // The starting pad: wide, centred, certain. The duck begins standing on it.
  s.platforms.push({ id: s.nextId++, kind: "static", x: view.w / 2 - 46, y: s.genY, w: 92, vx: 0, spring: false, crumble: -1 });
  return s;
}

export function setView(s: DoodleState, w: number, h: number): void {
  s.view.w = w;
  s.view.h = h;
}

/** mulberry32 over `state.rng`. */
export function rand(s: DoodleState): number {
  s.rng = (s.rng + 0x6d2b79f5) >>> 0;
  let t = s.rng;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

// ── Step ────────────────────────────────────────────

export function step(s: DoodleState, input: Input, dt: number): void {
  if (s.phase !== "playing") return;
  const sec = dt / 1000;
  s.time += dt;
  if (s.banner && s.time > s.banner.until) s.banner = null;

  moveDuck(s, input.x, sec, dt);
  movePlatforms(s, sec, dt);
  moveBugs(s, sec);
  collide(s);
  generate(s);
  raiseCamera(s);
  prune(s);
  ageParticles(s, sec, dt);
  checkZone(s);
  checkFall(s);
}

function moveDuck(s: DoodleState, steer: number, sec: number, dt: number) {
  const d = s.duck;
  const x = Math.max(-1, Math.min(1, steer));
  if (x !== 0) d.facing = x < 0 ? -1 : 1;
  d.x += x * MOVE_SPEED * sec;
  if (d.x < 0) d.x += s.view.w;
  if (d.x >= s.view.w) d.x -= s.view.w;

  if (d.bubble > 0) {
    d.bubble -= dt;
    d.vy = BUBBLE_V;
    if (d.bubble <= 0) {
      d.bubble = 0;
      d.vy = BUBBLE_POP_V;
      burst(s, d.x, d.y, 10, "accent", 150);
    }
  } else {
    d.vy = Math.max(-MAX_FALL, d.vy - GRAVITY * sec);
  }
  d.y += d.vy * sec;
  s.peak = Math.max(s.peak, d.y);
}

function movePlatforms(s: DoodleState, sec: number, dt: number) {
  for (let i = s.platforms.length - 1; i >= 0; i--) {
    const p = s.platforms[i];
    if (p.kind === "moving") {
      p.x += p.vx * sec;
      if (p.x < 6) { p.x = 6; p.vx = Math.abs(p.vx); }
      if (p.x + p.w > s.view.w - 6) { p.x = s.view.w - 6 - p.w; p.vx = -Math.abs(p.vx); }
    }
    if (p.crumble >= 0) {
      p.crumble -= dt;
      if (p.crumble <= 0) {
        burst(s, p.x + p.w / 2, p.y, 5, "text", 70);
        s.platforms[i] = s.platforms[s.platforms.length - 1];
        s.platforms.pop();
      }
    }
  }
}

function moveBugs(s: DoodleState, sec: number) {
  for (const bug of s.bugs) {
    bug.x += bug.vx * sec;
    if (bug.x < bug.minX) { bug.x = bug.minX; bug.vx = Math.abs(bug.vx); }
    if (bug.x > bug.maxX) { bug.x = bug.maxX; bug.vx = -Math.abs(bug.vx); }
  }
}

function collide(s: DoodleState) {
  const d = s.duck;
  // Everything collides across the wrap seam, because the duck is drawn on both sides of it:
  // the shortest wrapped x-distance is the one that matches what the player sees.
  const width = s.view.w;
  const wrapDx = (x: number) => {
    let dx = (((x - d.x) % width) + width) % width;
    if (dx > width / 2) dx -= width;
    return dx;
  };

  if (d.bubble > 0) {
    // Riding the bubble: pads and bugs alike are swept aside, nothing can hurt.
    for (let i = s.bugs.length - 1; i >= 0; i--) {
      const bug = s.bugs[i];
      const dx = wrapDx(bug.x);
      if (dx * dx + (bug.y - d.y) ** 2 < (DUCK_R + BUG_R) ** 2) squashBug(s, i);
    }
    return;
  }

  if (d.vy <= 0) {
    const feet = d.y - DUCK_R;
    for (const p of s.platforms) {
      if (feet > p.y + 2 || feet < p.y - 20) continue;
      if (Math.abs(wrapDx(p.x + p.w / 2)) > p.w / 2 + DUCK_R * 0.75) continue;
      d.y = p.y + DUCK_R;
      if (p.spring) {
        d.vy = SPRING_V;
        burst(s, d.x, p.y, 8, "accent", 130);
      } else {
        d.vy = JUMP_V;
      }
      if (p.kind === "crumbling" && p.crumble < 0) p.crumble = CRUMBLE_MS;
      break;
    }
  }

  for (let i = s.bugs.length - 1; i >= 0; i--) {
    const bug = s.bugs[i];
    const dx = wrapDx(bug.x);
    if (dx * dx + (bug.y - d.y) ** 2 >= (DUCK_R + BUG_R) ** 2) continue;
    if (d.vy < 0 && d.y - 4 > bug.y) {
      squashBug(s, i);
      d.y = Math.max(d.y, bug.y + BUG_R + DUCK_R);
      d.vy = JUMP_V;
    } else {
      s.phase = "lost";
      burst(s, d.x, d.y, 16, "danger", 170);
      return;
    }
  }

  for (let i = s.items.length - 1; i >= 0; i--) {
    const item = s.items[i];
    const dx = wrapDx(item.x);
    if (dx * dx + (item.y - d.y) ** 2 >= (DUCK_R + ITEM_R) ** 2) continue;
    d.bubble = BUBBLE_MS;
    s.items.splice(i, 1);
    burst(s, item.x, item.y, 12, "accent", 160);
  }
}

function squashBug(s: DoodleState, index: number) {
  const bug = s.bugs[index];
  s.squash += 1;
  burst(s, bug.x, bug.y, 7, "danger", 120);
  s.bugs.splice(index, 1);
}

/** Keeps platforms stacked above the view's top edge, with bugs and bubbles among them. */
function generate(s: DoodleState) {
  const ceiling = s.cameraY + s.view.h + 260;
  while (s.genY < ceiling && s.platforms.length < MAX_PLATFORMS) {
    const altitude = s.genY;
    const d = difficultyAt(altitude);
    const gap = 44 + rand(s) * (maxGapAt(altitude) - 44);
    const w = Math.round(64 - 20 * d + rand(s) * 10);
    const y = s.genY + gap;
    // The ladder drifts at most ±150 px a step, so it always reads as a path, never a scatter.
    const x = Math.max(6, Math.min(s.view.w - w - 6, s.genX + (rand(s) - 0.5) * 300 - w / 2));

    const roll = rand(s);
    let kind: PlatformKind = "static";
    if (roll < 0.05 + 0.35 * d) kind = "moving";
    else if (roll < 0.05 + 0.35 * d + 0.04 + 0.3 * d) kind = "crumbling";

    const spring = kind === "static" && rand(s) < 0.07;
    s.platforms.push({
      id: s.nextId++, kind, x, y, w,
      vx: kind === "moving" ? (30 + rand(s) * 55) * (rand(s) < 0.5 ? -1 : 1) : 0,
      spring,
      crumble: -1
    });
    s.genY = y;
    s.genX = x + w / 2;

    if (s.bugs.length < MAX_BUGS && d >= 0.06 && rand(s) < 0.05 + 0.22 * d) {
      // The patrol sits beside the pad, never straight above its centre: a clean landing on the
      // pad beneath a bug's beat must never be an unavoidable death.
      const offset = (60 + rand(s) * 50) * (rand(s) < 0.5 ? -1 : 1);
      const bx = Math.max(20, Math.min(s.view.w - 20, x + w / 2 + offset));
      const span = 40 + rand(s) * 30;
      s.bugs.push({
        id: s.nextId++,
        x: bx,
        y: y - 50,
        minX: Math.max(6, bx - span),
        maxX: Math.min(s.view.w - 6, bx + span),
        vx: (40 + rand(s) * 45) * (rand(s) < 0.5 ? -1 : 1)
      });
    }
    if (s.items.length < MAX_ITEMS && rand(s) < 0.012) {
      s.items.push({ x: x + w / 2, y: y - 36 });
    }
  }
}

/** The camera never falls: the duck is held at most 58% up the view while it climbs. */
function raiseCamera(s: DoodleState) {
  const target = s.duck.y - s.view.h * 0.58;
  if (target > s.cameraY) s.cameraY = target;
}

function prune(s: DoodleState) {
  const floor = s.cameraY - 80;
  for (let i = s.platforms.length - 1; i >= 0; i--) {
    if (s.platforms[i].y < floor) {
      s.platforms[i] = s.platforms[s.platforms.length - 1];
      s.platforms.pop();
    }
  }
  for (let i = s.bugs.length - 1; i >= 0; i--) {
    if (s.bugs[i].y < floor) s.bugs.splice(i, 1);
  }
  for (let i = s.items.length - 1; i >= 0; i--) {
    if (s.items[i].y < floor) s.items.splice(i, 1);
  }
}

function checkZone(s: DoodleState) {
  const zone = zoneOf(s.duck.y);
  if (zone === "sky" && s.zone < 1) {
    s.zone = 1;
    s.banner = { text: "The sky opens up", until: s.time + BANNER_MS };
  } else if (zone === "stars" && s.zone < 2) {
    s.zone = 2;
    s.banner = { text: "The stars come out", until: s.time + BANNER_MS };
  }
}

function checkFall(s: DoodleState) {
  if (s.duck.y + DUCK_R < s.cameraY - 6) {
    s.phase = "lost";
    burst(s, s.duck.x, s.cameraY, 14, "accent", 150);
  }
}

function burst(s: DoodleState, x: number, y: number, count: number, tone: Tone, speed: number) {
  for (let i = 0; i < count && s.particles.length < MAX_PARTICLES; i++) {
    const angle = rand(s) * Math.PI * 2;
    const v = speed * (0.4 + rand(s) * 0.8);
    const life = 260 + rand(s) * 240;
    s.particles.push({ x, y, vx: Math.cos(angle) * v, vy: Math.sin(angle) * v, life, max: life, size: 1.5 + rand(s) * 2, tone });
  }
}

/** Ages the effect particles; the renderer skips them entirely under reduced motion. */
function ageParticles(s: DoodleState, sec: number, dt: number) {
  for (let i = s.particles.length - 1; i >= 0; i--) {
    const p = s.particles[i];
    p.x += p.vx * sec;
    p.y += p.vy * sec;
    p.vx *= 0.92;
    p.vy *= 0.92;
    p.life -= dt;
    if (p.life <= 0) {
      s.particles[i] = s.particles[s.particles.length - 1];
      s.particles.pop();
    }
  }
}
