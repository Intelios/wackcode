/**
 * The compaction stage's gravity well (`CompactingStage.tsx`): text-like fragments drift in
 * from the edges and spiral into an accent-coloured core that warms as it swallows them —
 * the conversation being folded down to a summary.
 *
 * Invariants:
 * - The simulation is a pure, seeded function of (seed, dtMs, nowMs) steps — no DOM, no
 *   `Date.now()`. `render` is the only function that touches a canvas, and it is called
 *   only when a real 2D context exists (jsdom tests never draw).
 * - `step(state, dtMs, nowMs)`: `dtMs` is real frame delta (clamped ≤ 50ms by the caller);
 *   `nowMs` is wall-clock elapsed since mount, used for act boundaries and scheduling so a
 *   hidden tab resumes into the right act without a particle jump.
 * - Escalating acts: calm → busy at 20s → dramatic at 60s, eased over `ACT_BLEND_MS`.
 * - The duck cameo is never swallowed; it swings through and leaves.
 * - Drawing uses only cheap ops (transforms, globalAlpha); gradients are cached per state.
 */

export type Act = "calm" | "busy" | "dramatic";

/** Act boundaries and how long each transition eases. */
export const ACT_BUSY_MS = 20_000;
export const ACT_DRAMATIC_MS = 60_000;
export const ACT_BLEND_MS = 2_000;

/** How long the collapse finale runs before the stage may unmount. */
export const FINALE_MS = 600;
export const PARTICLE_CAP = 160;
/** One duck fly-by, edge to edge. */
export const CAMEO_MS = 4_000;

const TAU = Math.PI * 2;

interface ActTune {
  /** New fragments per second. */
  spawn: number;
  /** Mean gap between core flares. */
  flareGap: number;
  /** Orbital speed multiplier (Keplerian fall-off applied on top). */
  rotation: number;
  /** Radial pull multiplier; higher acts swallow faster. */
  pull: number;
  /** Baseline core heat the glow decays back to. */
  heat: number;
  /** Core radius multiplier. */
  core: number;
}
const ACTS: Record<Act, ActTune> = {
  calm:     { spawn: 2.2, flareGap: 8_000, rotation: 1.0, pull: 1.0, heat: 0.12, core: 1.0 },
  busy:     { spawn: 4.0, flareGap: 3_000, rotation: 1.4, pull: 1.4, heat: 0.30, core: 1.2 },
  dramatic: { spawn: 6.5, flareGap: 1_500, rotation: 1.8, pull: 1.8, heat: 0.55, core: 1.25 },
};

export function actFor(elapsedMs: number): Act {
  return elapsedMs >= ACT_DRAMATIC_MS ? "dramatic" : elapsedMs >= ACT_BUSY_MS ? "busy" : "calm";
}

/** 0→1 ease through `ACT_BLEND_MS` after the latest act boundary (1 in calm's run-in too). */
export function actBlend(nowMs: number): number {
  const start = nowMs >= ACT_DRAMATIC_MS ? ACT_DRAMATIC_MS : nowMs >= ACT_BUSY_MS ? ACT_BUSY_MS : 0;
  return Math.min(1, (nowMs - start) / ACT_BLEND_MS);
}

/** Interpolated tuning for the current moment, blending over the act boundary. */
function tune(nowMs: number): ActTune {
  const act = actFor(nowMs);
  const blend = actBlend(nowMs);
  if (blend >= 1 || act === "calm") return ACTS[act];
  const prev = act === "dramatic" ? ACTS.busy : ACTS.calm;
  const next = ACTS[act];
  const mix = (a: number, b: number) => a + (b - a) * blend;
  return {
    spawn: mix(prev.spawn, next.spawn), flareGap: mix(prev.flareGap, next.flareGap),
    rotation: mix(prev.rotation, next.rotation), pull: mix(prev.pull, next.pull),
    heat: mix(prev.heat, next.heat), core: mix(prev.core, next.core),
  };
}

export interface Palette {
  accent: string;
  text: string;
  textDim: string;
  surface: string;
}

export interface Particle {
  /** Polar coordinates around the core; x/y are the render position. */
  theta: number;
  dist: number;
  x: number;
  y: number;
  /** Radial velocity px/s, grows as it falls; tangential is `omega * dist`. */
  fall: number;
  /** Mini file glyph: page w×h whose contents follow `kind`. */
  w: number;
  h: number;
  kind: "doc" | "code" | "image";
  /** Per-particle pull variance so fragments fall on visibly different schedules. */
  jitter: number;
  /** 0 (cool, textDim) → 1 (accent) as it nears the horizon; written each step. */
  warm: number;
}

export interface Flare { born: number; dur: number; maxR: number }
export interface Cameo { born: number }

export interface WellState {
  rand: () => number;
  w: number;
  h: number;
  cx: number;
  cy: number;
  /** Simulation time (sum of dtMs) and wall-clock time of the last step. */
  simMs: number;
  act: Act;
  actBlend: number;
  particles: Particle[];
  flares: Flare[];
  core: { radius: number; heat: number; swallowed: number; pulse: number };
  horizon: number;
  /** Seeded [angle, radius-jitter] pairs for the accretion disk's motes — kept on the state
      so `render` never consumes the simulation's RNG (steps stay deterministic). */
  disk: number[];
  maxDist: number;
  accretionTheta: number;
  spawnCarry: number;
  nextFlareAt: number;
  cameo?: Cameo;
  nextCameoAt: number;
  cameos: number;
  /** Wall-clock ms when collapse started; undefined while live. */
  ending?: number;
  finaleRing: number;
  /** Rare dramatic-act lensing pulse; 1 while wobbling. */
  wobble: number;
}

/** mulberry32: tiny deterministic PRNG so a seed reproduces a whole compaction. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const HORIZON = 11;
const CORE_R = 10;
/** Keplerian-ish constant: omega = K·rotation / dist^1.5 — tight, fast whips near the core. */
const ORBIT_K = 1_400;
/** Drift-in acceleration px/s², scaled by act pull, distance (falls faster as it nears) and
    per-particle jitter — tuned so a calm fragment takes ~10–15s from the edge to the horizon. */
const DRIFT = 3.4;
const END_DRIFT = 90;
const MAX_FALL = 240;

export function createWell(seed: number, w: number, h: number): WellState {
  const rand = mulberry32(seed);
  const state: WellState = {
    rand, w, h, cx: w / 2, cy: h / 2,
    simMs: 0, act: "calm", actBlend: 1,
    particles: [], flares: [],
    core: { radius: CORE_R, heat: ACTS.calm.heat, swallowed: 0, pulse: 0 },
    disk: Array.from({ length: 120 }, () => rand()),
    horizon: HORIZON,
    maxDist: Math.hypot(w, h) / 2 + 30,
    accretionTheta: 0,
    spawnCarry: 0,
    nextFlareAt: 2_500 + rand() * 2_500,
    nextCameoAt: 6_000 + rand() * 40_000,
    cameos: 0,
    finaleRing: 0,
    wobble: 0,
  };
  return state;
}

function spawn(state: WellState, t: ActTune) {
  const theta = state.rand() * TAU;
  const dist = state.maxDist - 40 + state.rand() * 40;
  state.particles.push({
    theta, dist,
    x: state.cx + Math.cos(theta) * dist,
    y: state.cy + Math.sin(theta) * dist,
    fall: 0,
    w: 9 + state.rand() * 6,
    h: 11 + state.rand() * 6,
    kind: (() => { const r = state.rand(); return r < 0.5 ? "doc" : r < 0.78 ? "code" : "image"; })(),
    jitter: 0.6 + state.rand() * 0.8,
    warm: 0,
  });
}

/**
 * Advances the simulation. `dtMs` is the frame's real delta (caller clamps to ≤ 50ms);
 * `nowMs` is wall-clock time since the stage mounted, used for act boundaries, flare and
 * cameo schedules, and the finale — so a tab that was hidden resumes into the right act
 * without a physics jump. Deterministic for a given seed and (dtMs, nowMs) sequence.
 */
export function step(state: WellState, dtMs: number, nowMs: number) {
  const t = tune(nowMs);
  const dt = dtMs / 1_000;
  state.simMs += dtMs;
  state.act = actFor(nowMs);
  state.actBlend = actBlend(nowMs);
  const ending = state.ending !== undefined;

  // Spawning stops the moment the collapse begins — the well swallows what it has.
  if (!ending) {
    state.spawnCarry += t.spawn * dt;
    while (state.spawnCarry >= 1 && state.particles.length < PARTICLE_CAP) {
      state.spawnCarry -= 1;
      spawn(state, t);
    }
    state.spawnCarry = Math.min(state.spawnCarry, 4);
  }

  for (let i = state.particles.length - 1; i >= 0; i--) {
    const p = state.particles[i];
    const omega = (ORBIT_K * t.rotation) / Math.pow(Math.max(p.dist, 14), 1.5);
    p.theta += omega * dt;
    const accel = (ending ? END_DRIFT : DRIFT) * t.pull * (1 + 120 / Math.max(p.dist, 20)) * p.jitter;
    p.fall = Math.min(ending ? MAX_FALL * 4 : MAX_FALL, p.fall + accel * dt);
    if (ending) {
      // Guarantee arrival exactly as the finale ends: floor the fall speed to whatever
      // covers the remaining distance, so the well is empty when the ring fires.
      const remainSec = Math.max(0.03, (FINALE_MS - (nowMs - state.ending!)) / 1_000);
      p.fall = Math.min(MAX_FALL * 4, Math.max(p.fall, p.dist / remainSec));
    }
    p.dist -= p.fall * dt;
    p.x = state.cx + Math.cos(p.theta) * p.dist;
    p.y = state.cy + Math.sin(p.theta) * p.dist;
    p.warm = Math.max(0, Math.min(1, 1 - (p.dist - state.horizon) / 60));
    if (p.dist <= state.horizon) {
      state.particles.splice(i, 1);
      state.core.swallowed += 1;
      state.core.heat = Math.min(1, state.core.heat + 0.09);
      // A little gulp: the core swells with every file it eats, then relaxes.
      state.core.pulse = Math.min(3, state.core.pulse + 0.9);
    }
  }

  // Core: breathing pulse in the dramatic act, shrinking to nothing during the finale.
  state.core.heat += (t.heat - state.core.heat) * Math.min(1, 0.8 * dt);
  state.core.pulse = Math.max(0, state.core.pulse - 5 * dt);
  const breathe = state.act === "dramatic" ? 1 + 0.15 * Math.sin(nowMs / 3_100 * TAU) : 1;
  // The core visibly fattens on what it has eaten (capped so it can't fill the stage).
  const grow = Math.min(14, state.core.swallowed * 0.45) + Math.min(5, state.core.pulse * 1.6);
  let radius = (CORE_R + grow) * t.core * breathe;
  if (ending) {
    const p = Math.min(1, (nowMs - state.ending!) / FINALE_MS);
    radius *= 1 - p;
    state.finaleRing = p;
  }
  state.core.radius = radius;
  state.horizon = HORIZON * t.core;

  // Flares: expanding rings fired off the core at each act's cadence.
  if (!ending && nowMs >= state.nextFlareAt) {
    state.flares.push({ born: nowMs, dur: 700, maxR: 55 + state.rand() * 35 });
    state.nextFlareAt = nowMs + t.flareGap * (0.7 + state.rand() * 0.6);
  }
  state.flares = state.flares.filter((f) => nowMs - f.born < f.dur);

  // Accretion disk: exists only inside the dramatic act's blend; spins up 4× in the finale.
  state.accretionTheta += 2.2 * dt * (ending ? 4 : state.act === "dramatic" ? state.actBlend : 0);

  // Lensing wobble: rare 1.4s pulses while dramatic — a subtle global scale shiver.
  state.wobble = state.act === "dramatic" && Math.sin(nowMs / 9_000 * TAU) > 0.86
    ? Math.sin(nowMs / 200 * TAU) * 0.02
    : 0;

  // The duck cameo: never swallowed, sweeps through on a hyperbolic orbit and leaves.
  // Entering the dramatic act guarantees one within its first 10s.
  if (!ending && state.act === "dramatic") {
    state.nextCameoAt = Math.min(state.nextCameoAt, nowMs + 8_000);
  }
  if (!ending && !state.cameo && nowMs >= state.nextCameoAt) {
    state.cameo = { born: nowMs };
    state.cameos += 1;
  }
  if (state.cameo && (ending || nowMs - state.cameo.born >= CAMEO_MS)) {
    state.cameo = undefined;
    state.nextCameoAt = nowMs + (state.act === "dramatic" ? 30_000 : 45_000) * (0.8 + state.rand() * 0.4);
  }
}

/** Where the duck is along its fly-by at `elapsed` ms after `born`. Returns null when done. */
export function cameoPose(state: WellState, elapsed: number): { x: number; y: number; rot: number; squash: number } | null {
  if (elapsed < 0 || elapsed > CAMEO_MS) return null;
  // Enters left at ~171°, swings under the core (periapsis mid-flight), exits upper-right.
  const t = elapsed / CAMEO_MS;
  // Fixed periapsis: the core may be shrinking through a finale while a cameo is mid-flight.
  const peri = 26;
  const dist = peri + (state.maxDist - peri) * (4 * (t - 0.5) * (t - 0.5));
  const theta = Math.PI * 0.95 - 1.10 * Math.PI * t;
  const x = state.cx + Math.cos(theta) * dist;
  const y = state.cy + Math.sin(theta) * dist;
  // Tangent direction from a tiny lookahead so the duck flies beak-first.
  const t2 = Math.min(1, t + 0.02);
  const dist2 = peri + (state.maxDist - peri) * (4 * (t2 - 0.5) * (t2 - 0.5));
  const theta2 = Math.PI * 0.95 - 1.10 * Math.PI * t2;
  const rot = Math.atan2(state.cy + Math.sin(theta2) * dist2 - y, state.cx + Math.cos(theta2) * dist2 - x);
  const closeness = 1 - (dist - peri) / (state.maxDist - peri);
  return { x, y, rot, squash: 1 - 0.15 * Math.max(0, closeness) };
}

/** Begins the collapse finale: pull everything in 4×, shrink the core, fire the ring. */
export function collapse(state: WellState, nowMs: number) {
  if (state.ending === undefined) state.ending = nowMs;
}

export function isDone(state: WellState, nowMs: number): boolean {
  return state.ending !== undefined && nowMs - state.ending >= FINALE_MS;
}

/**
 * A representative frozen frame for reduced motion: a warmed-up calm well with enough
 * fragments mid-spiral to read as a scene, no flares, no cameo.
 */
export function stillFrame(seed: number, w: number, h: number): WellState {
  const state = createWell(seed, w, h);
  let now = 0;
  while (state.particles.length < 14 && now < 20_000) {
    step(state, 40, now);
    now += 40;
  }
  state.flares = [];
  state.cameo = undefined;
  return state;
}

// ---------------------------------------------------------------------------
// Rendering. Everything below only runs when a 2D context exists; callers guard
// `canvas.getContext("2d")` for jsdom. Cheap ops only: transforms + globalAlpha.
// ---------------------------------------------------------------------------

/**
 * WackCode's rubber duck silhouette — the same shape as the app icon (`src-tauri/icons/icon.svg`).
 * `DuckMark.tsx` renders it as an SVG path; the well's cameo fills it as a `Path2D`.
 */
export const DUCK_PATH =
  "M34 102C50 120 66 134 88 136C108 137 107.91 128.49 99.91 100.49A56 56 0 0 1 205.92 65.02" +
  "C218 70 236 72 245 80C253 88 250 101 238 103C228 106 217 110 209 112C201 114 205 132 217 142" +
  "C231 154 238 172 234 188C228 216 190 228 128 228C70 228 32 216 24 188C18 166 16 126 22 106" +
  "C24 99 30 98 34 102ZM159 72A11 11 0 1 0 181 72A11 11 0 1 0 159 72Z";

let duckPath: Path2D | undefined;
function duck(): Path2D | undefined {
  if (typeof Path2D === "undefined") return undefined;
  return duckPath ?? (duckPath = new Path2D(DUCK_PATH));
}

/** A tiny file page centred on the origin: outline with a folded corner, then its contents
    — text lines for documents, `</>` for code, a sun over hills for images. */
function fileGlyph(ctx: CanvasRenderingContext2D, w: number, h: number, kind: "doc" | "code" | "image") {
  const fold = Math.min(3.2, w * 0.28);
  ctx.beginPath();
  ctx.moveTo(-w / 2, -h / 2);
  ctx.lineTo(w / 2 - fold, -h / 2);
  ctx.lineTo(w / 2, -h / 2 + fold);
  ctx.lineTo(w / 2, h / 2);
  ctx.lineTo(-w / 2, h / 2);
  ctx.closePath();
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(w / 2 - fold, -h / 2);
  ctx.lineTo(w / 2 - fold, -h / 2 + fold);
  ctx.lineTo(w / 2, -h / 2 + fold);
  ctx.stroke();
  const lw = ctx.lineWidth;
  ctx.lineWidth = Math.max(0.7, lw * 0.8);
  if (kind === "doc") {
    const lineW = w - 6;
    const lines = h > 14 ? 3 : 2;
    for (let i = 0; i < lines; i++) {
      const y = -h / 2 + 4.5 + i * 3;
      if (y > h / 2 - 2.5) break;
      ctx.beginPath();
      ctx.moveTo(-w / 2 + 3, y);
      ctx.lineTo(-w / 2 + 3 + (i === lines - 1 ? lineW * 0.6 : lineW), y);
      ctx.stroke();
    }
  } else if (kind === "code") {
    // `</>` tucked low on the page.
    const cy0 = h / 2 - 5.5;
    for (const s of [-1, 1]) {
      ctx.beginPath();
      ctx.moveTo(s * 1.2 - s * 3.2, cy0 - 2.4);
      ctx.lineTo(s * 1.2, cy0);
      ctx.lineTo(s * 1.2 - s * 3.2, cy0 + 2.4);
      ctx.stroke();
    }
    ctx.beginPath();
    ctx.moveTo(-0.6, cy0 - 3.4);
    ctx.lineTo(0.6, cy0 + 3.4);
    ctx.stroke();
  } else {
    // image: a sun disc over two hills.
    const cy0 = h / 2 - 6.5;
    ctx.beginPath();
    ctx.arc(-w / 2 + 4, cy0 - 1.6, 1.4, 0, TAU);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(-w / 2 + 2, cy0 + 3.4);
    ctx.lineTo(-w / 2 + 4.8, cy0 + 0.6);
    ctx.lineTo(-w / 2 + 6.6, cy0 + 2.6);
    ctx.lineTo(-w / 2 + 8.4, cy0 + 0.8);
    ctx.lineTo(w / 2 - 2, cy0 + 3.4);
    ctx.stroke();
  }
  ctx.lineWidth = lw;
}

export function render(ctx: CanvasRenderingContext2D, state: WellState, palette: Palette, nowMs: number) {
  const { w, h, cx, cy } = state;
  ctx.clearRect(0, 0, w, h);
  ctx.save();
  // The finale implodes the whole field toward the core before the ring pings out.
  if (state.ending !== undefined) {
    const squash = 1 - 0.22 * Math.min(1, state.finaleRing);
    ctx.translate(cx, cy);
    ctx.scale(squash, squash);
    ctx.translate(-cx, -cy);
  }
  if (state.wobble) {
    ctx.translate(cx, cy);
    ctx.scale(1 + state.wobble, 1 + state.wobble);
    ctx.translate(-cx, -cy);
  }

  // Core halo: heat scales the glow; the finale shrinks it with the core.
  const heat = state.core.heat;
  if (state.core.radius > 0.1 || state.ending === undefined) {
    const glowR = Math.max(1, state.core.radius * (3 + heat * 2.5));
    const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, glowR);
    grad.addColorStop(0, palette.accent);
    grad.addColorStop(1, "transparent");
    ctx.globalAlpha = 0.35 + heat * 0.45;
    ctx.fillStyle = grad;
    ctx.fillRect(cx - glowR, cy - glowR, glowR * 2, glowR * 2);
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.arc(cx, cy, Math.max(0.1, state.core.radius), 0, TAU);
    ctx.fillStyle = palette.accent;
    ctx.fill();
  }

  // Flares: expanding accent rings.
  for (const f of state.flares) {
    const p = (nowMs - f.born) / f.dur;
    ctx.globalAlpha = (1 - p) * 0.5;
    ctx.strokeStyle = palette.accent;
    ctx.lineWidth = 2 * (1 - p) + 0.4;
    ctx.beginPath();
    ctx.arc(cx, cy, p * f.maxR, 0, TAU);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  // Accretion disk (dramatic only): a tilted luminous band of fine motes spinning round the
  // core, faded in by the act blend. Positions come from `state.disk` seeds, so rendering
  // never consumes the sim's RNG.
  if (state.act === "dramatic" && state.actBlend > 0) {
    const tilt = 0.62;
    const ringR = state.core.radius + 22;
    const blend = state.actBlend;
    // Glow band: two soft strokes in flattened space.
    ctx.save();
    ctx.translate(cx, cy);
    ctx.scale(1, tilt);
    ctx.strokeStyle = palette.accent;
    ctx.globalAlpha = 0.09 * blend;
    ctx.lineWidth = 20;
    ctx.beginPath(); ctx.arc(0, 0, ringR, 0, TAU); ctx.stroke();
    ctx.globalAlpha = 0.16 * blend;
    ctx.lineWidth = 7;
    ctx.beginPath(); ctx.arc(0, 0, ringR, 0, TAU); ctx.stroke();
    ctx.restore();
    // Motes: round dots projected onto the tilted orbit, inner ones whipping faster.
    const n = Math.floor(state.disk.length / 2);
    ctx.fillStyle = palette.accent;
    for (let i = 0; i < n; i++) {
      const aSeed = state.disk[i * 2];
      const rSeed = state.disk[i * 2 + 1];
      const r = ringR + (rSeed - 0.5) * 18;
      const speed = 2.4 * Math.pow(ringR / r, 1.5);
      const a = aSeed * TAU + state.accretionTheta * speed * 0.45;
      ctx.globalAlpha = (0.2 + 0.35 * ((i * 7) % 3) / 2) * blend;
      ctx.beginPath();
      ctx.arc(cx + Math.cos(a) * r, cy + Math.sin(a) * r * tilt, 0.7 + rSeed * 0.9, 0, TAU);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }

  // Fragments: little files tumbling along their spiral, warming toward accent at the horizon.
  for (const p of state.particles) {
    const alpha = Math.max(0.4, Math.min(0.95, 0.5 + 0.45 * (1 - p.dist / state.maxDist)));
    const ang = Math.atan2(p.y - cy, p.x - cx) + Math.PI / 2 + (1 - p.dist / state.maxDist) * 0.45
      + Math.sin(p.theta * 2.1 + p.w) * 0.35;
    ctx.save();
    ctx.translate(p.x, p.y);
    ctx.rotate(ang);
    ctx.globalAlpha = alpha * (1 - p.warm * 0.55);
    ctx.strokeStyle = palette.textDim;
    ctx.lineWidth = 1.1;
    fileGlyph(ctx, p.w, p.h, p.kind);
    if (p.warm > 0.05) {
      ctx.globalAlpha = alpha * p.warm * 0.9;
      ctx.strokeStyle = palette.accent;
      fileGlyph(ctx, p.w, p.h, p.kind);
    }
    ctx.restore();
  }
  ctx.globalAlpha = 1;

  // Duck cameo.
  if (state.cameo) {
    const pose = cameoPose(state, Math.min(CAMEO_MS, nowMs - state.cameo.born));
    const path = duck();
    if (pose && path) {
      // ~26px silhouette, squashed vertically as it passes closest to the core.
      const s = 26 / 256;
      ctx.save();
      ctx.translate(pose.x, pose.y);
      ctx.rotate(pose.rot);
      ctx.scale(s, s * pose.squash);
      ctx.translate(-128, -128);
      ctx.globalAlpha = 0.85;
      ctx.fillStyle = palette.text;
      ctx.fill(path);
      ctx.restore();
      ctx.globalAlpha = 1;
    }
  }

  // Finale ring: one accent ping outward as the core vanishes.
  if (state.finaleRing > 0 && state.finaleRing < 1) {
    ctx.globalAlpha = (1 - state.finaleRing) * 0.9;
    ctx.strokeStyle = palette.accent;
    ctx.lineWidth = 3 * (1 - state.finaleRing) + 0.5;
    ctx.beginPath();
    ctx.arc(cx, cy, state.finaleRing * h * 1.2, 0, TAU);
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
  ctx.restore();
}
