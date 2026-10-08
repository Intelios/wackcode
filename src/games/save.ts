/**
 * Quack Survivors' saved run: the whole `QuackState` in `localStorage`, versioned and normalised
 * on the way back out. A run autosaves while it is played (`QuackSurvivors.tsx` throttles this
 * to one write per couple of seconds and forces one on every screen change), so stopping mid-run
 * and quitting the app keeps the duck, its level, its weapons and the clock.
 *
 * This is a convenience like `scores.ts`: unreadable storage just reads as no save, and a failing
 * write never interrupts a run. Nothing here talks to the network, and the envelope's `version`
 * lets a future build refuse a shape it doesn't understand instead of mis-reading it.
 */
import {
  BOSS_TIMES, ENEMIES, MAX_ENEMIES, UPGRADE_IDS, WEAPONS, PASSIVES,
  type Bolt, type Enemy, type EnemyKind, type EliteKind, type Particle,
  type QuackState, type Ring, type Spit, type Token, type Tone, type UpgradeId, type WeaponId, type PassiveId, type Arc
} from "./quack";

export const SAVE_KEY = "wackcode:quackSave";
export const SAVE_VERSION = 1;
const THROTTLE_MS = 2000;

let lastWrite = 0;

/** Writes the run, at most once per `THROTTLE_MS` unless forced. Storage failures are silent. */
export function writeSave(state: QuackState, force = false): void {
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
export function readSave(): QuackState | null {
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
// Every field is rebuilt in `createQuack`'s key order with clamped, finite values, so a resumed
// run is indistinguishable from one that never left memory — the determinism tests rely on it.

function num(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function record(source: unknown, keys: readonly string[], fallback: Record<string, number>): Record<string, number> {
  const from = source && typeof source === "object" ? (source as Record<string, unknown>) : {};
  const out: Record<string, number> = {};
  for (const key of keys) out[key] = Math.max(0, Math.floor(num(from[key], fallback[key] ?? 0)));
  return out;
}

/** Like `record`, but keeps fractions: cooldowns are milliseconds in flight, not levels. */
function fractionalRecord(source: unknown, keys: readonly string[], fallback: Record<string, number>): Record<string, number> {
  const from = source && typeof source === "object" ? (source as Record<string, unknown>) : {};
  const out: Record<string, number> = {};
  for (const key of keys) out[key] = Math.max(0, num(from[key], fallback[key] ?? 0));
  return out;
}

function list(source: unknown, build: (raw: unknown) => unknown): unknown[] {
  return Array.isArray(source) ? source.map(build) : [];
}

function normalizeState(raw: unknown): QuackState | null {
  if (!raw || typeof raw !== "object") return null;
  const s = raw as Record<string, unknown>;
  if (s.phase !== "playing") return null;
  const player = (s.player && typeof s.player === "object" ? s.player : {}) as Record<string, unknown>;
  const facing = (player.facing && typeof player.facing === "object" ? player.facing : {}) as Record<string, unknown>;
  const view = (s.view && typeof s.view === "object" ? s.view : {}) as Record<string, unknown>;
  const weapons = record(s.weapons, WEAPONS, { quill: 1 }) as Record<WeaponId, number>;
  const passives = record(s.passives, PASSIVES, {}) as Record<PassiveId, number>;
  const evolved: Partial<Record<WeaponId, true>> = {};
  const evolvedRaw = s.evolved && typeof s.evolved === "object" ? (s.evolved as Record<string, unknown>) : {};
  for (const id of WEAPONS) if (evolvedRaw[id] === true) evolved[id] = true;
  const enemies = list(s.enemies, normalizeEnemy).filter((e): e is Enemy => e !== null);
  const bolts = list(s.bolts, normalizeBolt).filter((b): b is Bolt => b !== null);
  const spits = list(s.spits, normalizeSpit).filter((t): t is Spit => t !== null);
  const arcs = list(s.arcs, normalizeArc).filter((a): a is Arc => a !== null);
  const time = Math.max(0, num(s.time, 0));
  const hp = Math.max(1, num(player.hp, 100));

  const state: QuackState = {
    rng: Math.max(1, Math.floor(num(s.rng, 1))),
    nextId: Math.max(1, Math.floor(num(s.nextId, 1))),
    time,
    phase: "playing",
    view: { w: Math.max(120, num(view.w, 480)), h: Math.max(120, num(view.h, 640)) },
    player: {
      x: num(player.x, 0),
      y: num(player.y, 0),
      hp: Math.min(hp, 100 + 20 * passives.vitality),
      facing: { x: num(facing.x, 1), y: num(facing.y, 0) },
      invuln: Math.max(0, num(player.invuln, 0)),
      level: Math.max(1, Math.floor(num(player.level, 1))),
      xp: Math.max(0, num(player.xp, 0))
    },
    weapons,
    passives,
    cooldowns: fractionalRecord(s.cooldowns, WEAPONS, { quill: 400 }) as Record<WeaponId, number>,
    enemies: enemies.slice(0, MAX_ENEMIES),
    bolts,
    tokens: list(s.tokens, normalizeToken).filter((t): t is Token => t !== null),
    rings: list(s.rings, normalizeRing).filter((r): r is Ring => r !== null),
    particles: list(s.particles, normalizeParticle).filter((p): p is Particle => p !== null),
    kills: Math.max(0, Math.floor(num(s.kills, 0))),
    spawnDebt: Math.max(0, num(s.spawnDebt, 0)),
    bosses: Math.min(BOSS_TIMES.length, Math.max(0, Math.floor(num(s.bosses, 0)))),
    pendingLevels: Math.max(0, Math.floor(num(s.pendingLevels, 0))),
    choices: normalizeChoices(s.choices),
    banner: normalizeBanner(s.banner, time),
    shake: Math.max(0, num(s.shake, 0)),
    evolved,
    spits,
    arcs,
    nextChaos: Math.max(time, num(s.nextChaos, time + 30_000))
  };
  return state;
}

function normalizeEnemy(raw: unknown): Enemy | null {
  if (!raw || typeof raw !== "object") return null;
  const e = raw as Record<string, unknown>;
  const kind: EnemyKind = (ENEMIES as Record<string, unknown>)[e.kind as string] ? (e.kind as EnemyKind) : "bug";
  const scale = Math.min(3, Math.max(0.2, num(e.scale, 1)));
  const elite: EliteKind | null = e.elite === "swift" || e.elite === "armored" || e.elite === "explosive" ? e.elite : null;
  const armored = elite === "armored" ? 2.4 : 1;
  const maxHp = Math.max(1, num(e.maxHp, Math.round(ENEMIES[kind].hp * 1.35 * scale * armored)));
  return {
    id: Math.floor(num(e.id, 0)),
    kind,
    x: num(e.x, 0),
    y: num(e.y, 0),
    kx: num(e.kx, 0),
    ky: num(e.ky, 0),
    hp: Math.min(maxHp, Math.max(0, num(e.hp, maxHp))),
    maxHp,
    flash: Math.max(0, num(e.flash, 0)),
    orbitCd: Math.max(0, num(e.orbitCd, 0)),
    scale,
    elite,
    auraCd: Math.max(0, num(e.auraCd, 0)),
    spitCd: Math.max(0, num(e.spitCd, 1400))
  };
}

function normalizeBolt(raw: unknown): Bolt | null {
  if (!raw || typeof raw !== "object") return null;
  const b = raw as Record<string, unknown>;
  const kind = b.kind === "quill" || b.kind === "comet" || b.kind === "flock" ? b.kind : "quill";
  return {
    kind,
    x: num(b.x, 0),
    y: num(b.y, 0),
    vx: num(b.vx, 0),
    vy: num(b.vy, 0),
    damage: num(b.damage, 0),
    life: Math.max(0, num(b.life, 0)),
    hits: Array.isArray(b.hits) ? b.hits.filter((h): h is number => typeof h === "number") : [],
    limit: Math.max(1, Math.floor(num(b.limit, kind === "comet" ? 999 : 1)))
  };
}

function normalizeSpit(raw: unknown): Spit | null {
  if (!raw || typeof raw !== "object") return null;
  const t = raw as Record<string, unknown>;
  return { x: num(t.x, 0), y: num(t.y, 0), vx: num(t.vx, 0), vy: num(t.vy, 0), damage: num(t.damage, 9), life: Math.max(0, num(t.life, 0)) };
}

function normalizeArc(raw: unknown): Arc | null {
  if (!raw || typeof raw !== "object") return null;
  const a = raw as Record<string, unknown>;
  const points = Array.isArray(a.points) ? a.points.filter((p): p is number => typeof p === "number") : [];
  const max = Math.max(1, num(a.max, 220));
  return { points, life: Math.min(max, Math.max(0, num(a.life, 0))), max };
}

function normalizeToken(raw: unknown): Token | null {
  if (!raw || typeof raw !== "object") return null;
  const t = raw as Record<string, unknown>;
  return { x: num(t.x, 0), y: num(t.y, 0), value: Math.max(1, num(t.value, 1)), flying: t.flying === true };
}

function normalizeRing(raw: unknown): Ring | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  const max = Math.max(1, num(r.max, 420));
  return { x: num(r.x, 0), y: num(r.y, 0), radius: Math.max(0, num(r.radius, 0)), life: Math.min(max, Math.max(0, num(r.life, 0))), max };
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

function normalizeChoices(raw: unknown): UpgradeId[] | null {
  if (!Array.isArray(raw) || !raw.length) return null;
  const picks = raw.filter((id): id is UpgradeId => typeof id === "string" && (UPGRADE_IDS as readonly string[]).includes(id));
  return picks.length ? picks.slice(0, 3) : null;
}

function normalizeBanner(raw: unknown, time: number): { text: string; until: number } | null {
  if (!raw || typeof raw !== "object") return null;
  const b = raw as Record<string, unknown>;
  if (typeof b.text !== "string" || !b.text) return null;
  const until = num(b.until, time + 2600);
  return until > time ? { text: b.text.slice(0, 60), until } : null;
}
