/**
 * Doodle Duck's saved run: the whole `DoodleState` in `localStorage`, versioned and normalised
 * on the way back out. A run autosaves while it is played (`DoodleDuck.tsx` throttles this to one
 * write per couple of seconds and forces one on every screen change), so stopping mid-climb and
 * quitting the app keeps the duck, its altitude and the pads around it.
 *
 * This is a convenience like `scores.ts`: unreadable storage just reads as no save, and a failing
 * write never interrupts a run. Nothing here talks to the network, and the envelope's `version`
 * lets a future build refuse a shape it doesn't understand instead of mis-reading it.
 */
import {
  MAX_BUGS, MAX_ITEMS, MAX_PARTICLES, MAX_PLATFORMS,
  type Banner, type Bug, type DoodleState, type Item, type Particle, type Platform, type Tone
} from "./doodle";

export const SAVE_KEY = "wackcode:doodleSave";
export const SAVE_VERSION = 1;
const THROTTLE_MS = 2000;

let lastWrite = 0;

/** Writes the run, at most once per `THROTTLE_MS` unless forced. Storage failures are silent. */
export function writeSave(state: DoodleState, force = false): void {
  const now = Date.now();
  if (!force && now - lastWrite < THROTTLE_MS) return;
  lastWrite = now;
  try {
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: SAVE_VERSION, state }));
  } catch {
    // Storage is a convenience; the in-memory run is the one being played.
  }
}

export function clearSave(): void {
  try {
    localStorage.removeItem(SAVE_KEY);
  } catch {
    // Nothing to do: with no readable storage there is no save either.
  }
}

/** The saved run, or null when there is nothing usable to continue. */
export function readSave(): DoodleState | null {
  let raw: unknown;
  try {
    raw = JSON.parse(localStorage.getItem(SAVE_KEY) ?? "null");
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object") return null;
  const envelope = raw as { version?: unknown; state?: unknown };
  if (envelope.version !== SAVE_VERSION) return null;
  return normalizeState(envelope.state);
}

export function hasSave(): boolean {
  return readSave() !== null;
}

// ── Normalising ──────────────────────────────────────
// Every field is rebuilt in `createDoodle`'s key order with clamped, finite values, so a resumed
// run is indistinguishable from one that never left memory — the determinism tests rely on it.

function num(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function list<T>(source: unknown, build: (raw: unknown) => T | null, cap: number): T[] {
  if (!Array.isArray(source)) return [];
  const out: T[] = [];
  for (const raw of source.slice(0, cap)) {
    const entry = build(raw);
    if (entry !== null) out.push(entry);
  }
  return out;
}

function normalizeState(raw: unknown): DoodleState | null {
  if (!raw || typeof raw !== "object") return null;
  const s = raw as Record<string, unknown>;
  if (s.phase !== "playing") return null;
  const duck = (s.duck && typeof s.duck === "object" ? s.duck : {}) as Record<string, unknown>;
  const view = (s.view && typeof s.view === "object" ? s.view : {}) as Record<string, unknown>;
  const time = Math.max(0, num(s.time, 0));
  const width = Math.max(120, num(view.w, 420));
  // A corrupt x is folded back into the view, as `moveDuck`'s wrap would do over time.
  const duckX = (((num(duck.x, 0) % width) + width) % width);

  const state: DoodleState = {
    rng: Math.max(1, Math.floor(num(s.rng, 1))),
    nextId: Math.max(1, Math.floor(num(s.nextId, 1))),
    time,
    phase: "playing",
    view: { w: width, h: Math.max(120, num(view.h, 640)) },
    duck: {
      x: duckX,
      y: num(duck.y, 0),
      vy: num(duck.vy, 0),
      facing: duck.facing === -1 ? -1 : 1,
      bubble: Math.max(0, num(duck.bubble, 0))
    },
    cameraY: num(s.cameraY, 0),
    peak: Math.max(0, num(s.peak, 0)),
    platforms: list(s.platforms, normalizePlatform, MAX_PLATFORMS),
    bugs: list(s.bugs, normalizeBug, MAX_BUGS),
    items: list(s.items, normalizeItem, MAX_ITEMS),
    particles: list(s.particles, normalizeParticle, MAX_PARTICLES),
    genY: num(s.genY, 0),
    genX: num(s.genX, 0),
    squash: Math.max(0, Math.floor(num(s.squash, 0))),
    zone: s.zone === 2 ? 2 : s.zone === 1 ? 1 : 0,
    banner: normalizeBanner(s.banner, time)
  };
  return state;
}

function normalizePlatform(raw: unknown): Platform | null {
  if (!raw || typeof raw !== "object") return null;
  const p = raw as Record<string, unknown>;
  const kind: Platform["kind"] = p.kind === "moving" || p.kind === "crumbling" ? p.kind : "static";
  return {
    id: Math.floor(num(p.id, 0)),
    kind,
    x: num(p.x, 0),
    y: num(p.y, 0),
    w: Math.min(400, Math.max(16, num(p.w, 64))),
    vx: kind === "moving" ? num(p.vx, 40) : 0,
    spring: p.spring === true,
    crumble: Math.max(-1, num(p.crumble, -1))
  };
}

function normalizeBug(raw: unknown): Bug | null {
  if (!raw || typeof raw !== "object") return null;
  const b = raw as Record<string, unknown>;
  const x = num(b.x, 0);
  return {
    id: Math.floor(num(b.id, 0)),
    x,
    y: num(b.y, 0),
    minX: num(b.minX, x - 40),
    maxX: num(b.maxX, x + 40),
    vx: num(b.vx, 40)
  };
}

function normalizeItem(raw: unknown): Item | null {
  if (!raw || typeof raw !== "object") return null;
  const i = raw as Record<string, unknown>;
  return { x: num(i.x, 0), y: num(i.y, 0) };
}

function normalizeParticle(raw: unknown): Particle | null {
  if (!raw || typeof raw !== "object") return null;
  const p = raw as Record<string, unknown>;
  const tone: Tone = p.tone === "accent" || p.tone === "danger" ? p.tone : "text";
  const max = Math.max(1, num(p.max, 260));
  return {
    x: num(p.x, 0), y: num(p.y, 0), vx: num(p.vx, 0), vy: num(p.vy, 0),
    life: Math.min(max, Math.max(0, num(p.life, 0))), max, size: Math.max(0.5, num(p.size, 2)), tone
  };
}

function normalizeBanner(raw: unknown, time: number): Banner | null {
  if (!raw || typeof raw !== "object") return null;
  const b = raw as Record<string, unknown>;
  if (typeof b.text !== "string" || !b.text) return null;
  const until = num(b.until, time + 2400);
  return until > time ? { text: b.text.slice(0, 60), until } : null;
}
