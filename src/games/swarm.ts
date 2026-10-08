/**
 * Swarm: the Games panel's survivors-like. The duck holds out against a swarm of bugs in an
 * endless arena while its weapons fire on their own; defeated bugs drop tokens, and every level
 * offers a pick of three upgrades. Surviving `WIN_MS` wins the run.
 *
 * This module is the whole simulation and nothing else: plain mutable state, no DOM, no clock.
 * `SwarmGame.tsx` steps it at `STEP_MS` from a rAF accumulator and `swarm-render.ts` draws it.
 *
 * Invariants:
 * - Deterministic: all randomness comes from the seeded generator in `state.rng`, so the same
 *   seed and inputs replay the same run (the tests rely on it).
 * - `step` does nothing while a level-up choice is open (`state.choices`) or the run has ended.
 * - Lists are capped (`MAX_ENEMIES`, `MAX_TOKENS`, `MAX_PARTICLES`) so a long run stays cheap.
 * - The state is JSON-safe: `games/session.ts` keeps it across panel remounts.
 */

export const STEP_MS = 1000 / 60;
export const WIN_MS = 5 * 60_000;
export const MAX_LEVEL = 5;
export const MAX_ENEMIES = 320;
export const MAX_TOKENS = 260;
export const MAX_PARTICLES = 360;
export const PLAYER_RADIUS = 12;

export type WeaponId = "quill" | "orbit" | "comet" | "ping";
export type PassiveId = "speed" | "magnet" | "vitality" | "haste";
export type UpgradeId = WeaponId | PassiveId | "snack";
export type EnemyKind = "bug" | "gnat" | "beetle" | "segfault";
export type Tone = "accent" | "danger" | "text";

export const WEAPONS: readonly WeaponId[] = ["quill", "orbit", "comet", "ping"];
export const PASSIVES: readonly PassiveId[] = ["speed", "magnet", "vitality", "haste"];

export interface Input { x: number; y: number }

export interface Enemy {
  id: number;
  kind: EnemyKind;
  x: number;
  y: number;
  /** Knockback velocity, decaying each step; the chase itself is not stored. */
  kx: number;
  ky: number;
  hp: number;
  maxHp: number;
  /** ms left of the white hit flash. */
  flash: number;
  /** ms until the orbit beads may hit it again. */
  orbitCd: number;
}

export interface Bolt {
  kind: "quill" | "comet";
  x: number;
  y: number;
  vx: number;
  vy: number;
  damage: number;
  life: number;
  /** Enemies a piercing comet has already hit. */
  hits: number[];
}

export interface Token { x: number; y: number; value: number; flying: boolean }
export interface Ring { x: number; y: number; radius: number; life: number; max: number }
export interface Particle { x: number; y: number; vx: number; vy: number; life: number; max: number; size: number; tone: Tone }
export interface Banner { text: string; until: number }

export interface SwarmState {
  rng: number;
  nextId: number;
  /** Simulated ms since the run began. */
  time: number;
  phase: "playing" | "won" | "lost";
  view: { w: number; h: number };
  player: {
    x: number;
    y: number;
    hp: number;
    facing: Input;
    /** ms of invulnerability left after a hit. */
    invuln: number;
    level: number;
    xp: number;
  };
  weapons: Record<WeaponId, number>;
  passives: Record<PassiveId, number>;
  cooldowns: Record<WeaponId, number>;
  enemies: Enemy[];
  bolts: Bolt[];
  tokens: Token[];
  rings: Ring[];
  particles: Particle[];
  kills: number;
  spawnDebt: number;
  bosses: number;
  /** Level-ups banked while a choice is open. */
  pendingLevels: number;
  /** The open level-up choice, or null while playing. */
  choices: UpgradeId[] | null;
  banner: Banner | null;
  /** ms of screen shake left; the renderer ignores it under reduced motion. */
  shake: number;
}

interface EnemySpec { radius: number; hp: number; speed: number; damage: number; xp: number; mass: number }

export const ENEMIES: Record<EnemyKind, EnemySpec> = {
  bug: { radius: 9, hp: 14, speed: 80, damage: 6, xp: 1, mass: 1 },
  gnat: { radius: 6, hp: 6, speed: 135, damage: 4, xp: 1, mass: 0.7 },
  beetle: { radius: 14, hp: 60, speed: 54, damage: 14, xp: 4, mass: 3 },
  segfault: { radius: 30, hp: 900, speed: 72, damage: 25, xp: 40, mass: 12 }
};

/** The Segfault mini-boss arrives at these run times. */
export const BOSS_TIMES = [120_000, 240_000];

// ── Stats ────────────────────────────────────────────

export function speedOf(s: SwarmState): number { return 150 * (1 + 0.1 * s.passives.speed); }
export function magnetOf(s: SwarmState): number { return 80 * (1 + 0.4 * s.passives.magnet); }
export function maxHpOf(s: SwarmState): number { return 100 + 20 * s.passives.vitality; }
function regenOf(s: SwarmState): number { return 0.6 + 0.6 * s.passives.vitality; }
function hasteOf(s: SwarmState): number { return 1 - 0.08 * s.passives.haste; }

export function quillStats(level: number) {
  return { count: 1 + Math.floor((level - 1) / 2), damage: 12 + 4 * (level - 1), cooldown: 800 - 80 * (level - 1) };
}
export function orbitStats(level: number) {
  return { count: 1 + level, damage: 8 + 3 * level, radius: 46 + 4 * level };
}
export function cometStats(level: number) {
  return { directions: level >= 5 ? 4 : level >= 3 ? 2 : 1, damage: 30 + 10 * (level - 1), cooldown: 2200 - 200 * (level - 1) };
}
export function pingStats(level: number) {
  return { radius: 90 + 18 * (level - 1), damage: 10 + 6 * (level - 1), cooldown: 3000 - 250 * (level - 1) };
}

/** Tokens needed to leave `level`. */
export function xpForLevel(level: number): number { return 4 + 3 * level; }

export function score(s: SwarmState): number {
  const base = s.kills + 2 * Math.floor(s.time / 1000);
  return s.phase === "won" ? base * 2 : base;
}

// ── Upgrades ─────────────────────────────────────────

export const UPGRADE_NAMES: Record<UpgradeId, string> = {
  quill: "Quill", orbit: "Orbit", comet: "Comet", ping: "Ping",
  speed: "Quick feet", magnet: "Long context", vitality: "Rubber hide", haste: "Hot loop", snack: "Bread crumbs"
};

export function upgradeLevel(s: SwarmState, id: UpgradeId): number {
  if (id === "snack") return 0;
  return isWeapon(id) ? s.weapons[id] : s.passives[id];
}

function isWeapon(id: UpgradeId): id is WeaponId { return (WEAPONS as readonly string[]).includes(id); }

/** One line describing what picking `id` gives at its next level. */
export function upgradeDetail(s: SwarmState, id: UpgradeId): string {
  const next = upgradeLevel(s, id) + 1;
  switch (id) {
    case "quill": {
      const q = quillStats(next);
      return next === 1 ? "Ink bolts at the nearest bug." : `${q.count} ${q.count === 1 ? "bolt" : "bolts"} · ${q.damage} damage · faster`;
    }
    case "orbit": {
      const o = orbitStats(next);
      return next === 1 ? "Beads circle you and chew through bugs." : `${o.count} beads · ${o.damage} damage`;
    }
    case "comet": {
      const c = cometStats(next);
      return next === 1 ? "A piercing streak the way you face." : `${c.directions === 1 ? "Forward" : c.directions === 2 ? "Forward and back" : "Four ways"} · ${c.damage} damage`;
    }
    case "ping": {
      const p = pingStats(next);
      return next === 1 ? "A ring pulses out and knocks bugs back." : `Wider ring · ${p.damage} damage`;
    }
    case "speed": return "Move 10% faster.";
    case "magnet": return "Pull tokens in from further away.";
    case "vitality": return "+20 max HP and slow healing.";
    case "haste": return "Weapons recharge 8% faster.";
    case "snack": return "Restore 30 HP.";
  }
}

/** Up to three distinct upgrades that aren't maxed; a snack when everything is. */
export function upgradeChoices(s: SwarmState): UpgradeId[] {
  const pool: UpgradeId[] = [...WEAPONS, ...PASSIVES].filter((id) => upgradeLevel(s, id) < MAX_LEVEL);
  const picks: UpgradeId[] = [];
  while (picks.length < 3 && pool.length) picks.push(pool.splice(Math.floor(rand(s) * pool.length), 1)[0]);
  return picks.length ? picks : ["snack"];
}

export function applyUpgrade(s: SwarmState, id: UpgradeId): void {
  if (!s.choices?.includes(id)) return;
  if (id === "snack") s.player.hp = Math.min(maxHpOf(s), s.player.hp + 30);
  else if (isWeapon(id)) s.weapons[id] = Math.min(MAX_LEVEL, s.weapons[id] + 1);
  else {
    s.passives[id] = Math.min(MAX_LEVEL, s.passives[id] + 1);
    if (id === "vitality") s.player.hp = Math.min(maxHpOf(s), s.player.hp + 20);
  }
  s.pendingLevels -= 1;
  s.choices = s.pendingLevels > 0 ? upgradeChoices(s) : null;
}

// ── Setup ────────────────────────────────────────────

export function createSwarm(seed: number, view = { w: 480, h: 640 }): SwarmState {
  return {
    rng: seed >>> 0 || 1,
    nextId: 1,
    time: 0,
    phase: "playing",
    view: { ...view },
    player: { x: 0, y: 0, hp: 100, facing: { x: 1, y: 0 }, invuln: 0, level: 1, xp: 0 },
    weapons: { quill: 1, orbit: 0, comet: 0, ping: 0 },
    passives: { speed: 0, magnet: 0, vitality: 0, haste: 0 },
    cooldowns: { quill: 400, orbit: 0, comet: 0, ping: 0 },
    enemies: [],
    bolts: [],
    tokens: [],
    rings: [],
    particles: [],
    kills: 0,
    spawnDebt: 0,
    bosses: 0,
    pendingLevels: 0,
    choices: null,
    banner: { text: "Survive 5:00", until: 2600 },
    shake: 0
  };
}

export function setView(s: SwarmState, w: number, h: number): void {
  s.view.w = w;
  s.view.h = h;
}

/** mulberry32 over `state.rng`. */
export function rand(s: SwarmState): number {
  s.rng = (s.rng + 0x6d2b79f5) >>> 0;
  let t = s.rng;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

// ── Step ─────────────────────────────────────────────

export function step(s: SwarmState, input: Input, dt: number): void {
  if (s.phase !== "playing" || s.choices) return;
  const sec = dt / 1000;
  s.time += dt;
  s.shake = Math.max(0, s.shake - dt);
  if (s.banner && s.time > s.banner.until) s.banner = null;

  movePlayer(s, input, sec);
  spawn(s, sec);
  moveEnemies(s, sec, dt);
  fireWeapons(s, dt);
  moveBolts(s, sec, dt);
  orbitHits(s);
  reapEnemies(s);
  contact(s);
  collectTokens(s, sec);
  ageEffects(s, sec, dt);

  if (s.player.hp <= 0) {
    s.player.hp = 0;
    s.phase = "lost";
    burst(s, s.player.x, s.player.y, 18, "accent", 160);
  } else if (s.time >= WIN_MS) {
    s.phase = "won";
  }
}

function movePlayer(s: SwarmState, input: Input, sec: number) {
  const p = s.player;
  let { x, y } = input;
  const length = Math.hypot(x, y);
  if (length > 1) { x /= length; y /= length; }
  if (length > 0.05) p.facing = { x: input.x / length, y: input.y / length };
  const speed = speedOf(s);
  p.x += x * speed * sec;
  p.y += y * speed * sec;
  p.invuln = Math.max(0, p.invuln - sec * 1000);
  p.hp = Math.min(maxHpOf(s), p.hp + regenOf(s) * sec);
}

/** Bugs per second at a run time: a gentle start that keeps climbing. */
export function spawnRate(time: number): number {
  const minutes = time / 60_000;
  return 0.7 + 0.9 * minutes + 0.25 * minutes * minutes;
}

function spawn(s: SwarmState, sec: number) {
  if (s.bosses < BOSS_TIMES.length && s.time >= BOSS_TIMES[s.bosses]) {
    s.bosses += 1;
    addEnemy(s, "segfault");
    s.banner = { text: "A Segfault approaches", until: s.time + 2600 };
    s.shake = 400;
  }
  s.spawnDebt += spawnRate(s.time) * sec;
  while (s.spawnDebt >= 1) {
    s.spawnDebt -= 1;
    if (s.enemies.length >= MAX_ENEMIES) { s.spawnDebt = 0; break; }
    const roll = rand(s);
    const seconds = s.time / 1000;
    if (seconds > 60 && roll < Math.min(0.22, 0.08 + seconds / 2000)) addEnemy(s, "beetle");
    else if (seconds > 30 && roll > 0.8) {
      // Gnats arrive as a pack from one direction.
      const angle = rand(s) * Math.PI * 2;
      for (let i = 0; i < 5 && s.enemies.length < MAX_ENEMIES; i++) addEnemy(s, "gnat", angle + (rand(s) - 0.5) * 0.35);
    } else addEnemy(s, "bug");
  }
}

/** Just past the corner of the view: bugs arrive from off screen. */
function spawnDistance(s: SwarmState, kind: EnemyKind): number {
  return Math.hypot(s.view.w, s.view.h) / 2 + 30 + ENEMIES[kind].radius;
}

/** Bugs get a little quicker as the run goes on. */
function enemySpeed(s: SwarmState, kind: EnemyKind): number {
  return ENEMIES[kind].speed * (1 + 0.03 * (s.time / 60_000));
}

function addEnemy(s: SwarmState, kind: EnemyKind, angle = rand(s) * Math.PI * 2) {
  const spec = ENEMIES[kind];
  const distance = spawnDistance(s, kind);
  const scale = kind === "segfault" ? s.bosses : 1 + (s.time / 60_000) * 0.35;
  const hp = Math.round(spec.hp * scale);
  s.enemies.push({
    id: s.nextId++, kind,
    x: s.player.x + Math.cos(angle) * distance,
    y: s.player.y + Math.sin(angle) * distance,
    kx: 0, ky: 0, hp, maxHp: hp, flash: 0, orbitCd: 0
  });
}

const CELL = 48;

/** Enemies bucketed by grid cell, rebuilt each step: separation and hits look only nearby. */
function grid(enemies: Enemy[]): Map<string, number[]> {
  const cells = new Map<string, number[]>();
  for (let i = 0; i < enemies.length; i++) {
    const key = `${Math.floor(enemies[i].x / CELL)},${Math.floor(enemies[i].y / CELL)}`;
    const cell = cells.get(key);
    if (cell) cell.push(i); else cells.set(key, [i]);
  }
  return cells;
}

function nearby(cells: Map<string, number[]>, x: number, y: number, reach: number, visit: (index: number) => void) {
  const span = Math.ceil(reach / CELL);
  const cx = Math.floor(x / CELL);
  const cy = Math.floor(y / CELL);
  for (let gx = cx - span; gx <= cx + span; gx++) {
    for (let gy = cy - span; gy <= cy + span; gy++) {
      const cell = cells.get(`${gx},${gy}`);
      if (cell) for (const index of cell) visit(index);
    }
  }
}

function moveEnemies(s: SwarmState, sec: number, dt: number) {
  const { x: px, y: py } = s.player;
  const decay = Math.exp(-sec * 8);
  const heading = Math.atan2(s.player.facing.y, s.player.facing.x);
  for (const e of s.enemies) {
    const dx = px - e.x;
    const dy = py - e.y;
    const distance = Math.hypot(dx, dy) || 1;
    const far = spawnDistance(s, e.kind);
    if (distance > far * 1.6) {
      // A straggler left far behind re-enters ahead of the duck, so outrunning the swarm
      // never empties the screen.
      const angle = heading + (rand(s) - 0.5) * 1.6;
      e.x = px + Math.cos(angle) * far;
      e.y = py + Math.sin(angle) * far;
      continue;
    }
    const speed = enemySpeed(s, e.kind);
    e.x += (dx / distance) * speed * sec + e.kx * sec;
    e.y += (dy / distance) * speed * sec + e.ky * sec;
    e.kx *= decay;
    e.ky *= decay;
    e.flash = Math.max(0, e.flash - dt);
    e.orbitCd = Math.max(0, e.orbitCd - dt);
  }
  // Soft separation keeps the swarm a crowd instead of one stacked blob.
  const cells = grid(s.enemies);
  for (let i = 0; i < s.enemies.length; i++) {
    const a = s.enemies[i];
    const ra = ENEMIES[a.kind].radius;
    nearby(cells, a.x, a.y, ra + 30, (j) => {
      if (j <= i) return;
      const b = s.enemies[j];
      const min = ra + ENEMIES[b.kind].radius;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d2 = dx * dx + dy * dy;
      if (d2 >= min * min || d2 === 0) return;
      const d = Math.sqrt(d2);
      const push = (min - d) * 0.5;
      const ma = ENEMIES[a.kind].mass;
      const mb = ENEMIES[b.kind].mass;
      const share = mb / (ma + mb);
      a.x -= (dx / d) * push * share;
      a.y -= (dy / d) * push * share;
      b.x += (dx / d) * push * (1 - share);
      b.y += (dy / d) * push * (1 - share);
    });
  }
}

function hit(s: SwarmState, e: Enemy, damage: number, fromX: number, fromY: number, knock: number) {
  e.hp -= damage;
  e.flash = 90;
  const dx = e.x - fromX;
  const dy = e.y - fromY;
  const d = Math.hypot(dx, dy) || 1;
  const mass = ENEMIES[e.kind].mass;
  e.kx += (dx / d) * knock / mass;
  e.ky += (dy / d) * knock / mass;
}

function fireWeapons(s: SwarmState, dt: number) {
  const haste = hasteOf(s);
  const p = s.player;
  for (const id of WEAPONS) {
    const level = s.weapons[id];
    if (!level || id === "orbit") continue;
    s.cooldowns[id] -= dt;
    if (s.cooldowns[id] > 0) continue;
    if (id === "quill") {
      const q = quillStats(level);
      const targets = s.enemies
        .map((e) => ({ e, d: (e.x - p.x) ** 2 + (e.y - p.y) ** 2 }))
        .filter(({ d }) => d < 520 * 520)
        .sort((a, b) => a.d - b.d)
        .slice(0, q.count);
      if (!targets.length) { s.cooldowns.quill = 0; continue; }
      for (const { e } of targets) {
        const d = Math.hypot(e.x - p.x, e.y - p.y) || 1;
        s.bolts.push({ kind: "quill", x: p.x, y: p.y, vx: (e.x - p.x) / d * 430, vy: (e.y - p.y) / d * 430, damage: q.damage, life: 1000, hits: [] });
      }
      s.cooldowns.quill = q.cooldown * haste;
    } else if (id === "comet") {
      const c = cometStats(level);
      const { x: fx, y: fy } = p.facing;
      const directions = c.directions === 1 ? [[fx, fy]] : c.directions === 2 ? [[fx, fy], [-fx, -fy]] : [[fx, fy], [-fx, -fy], [-fy, fx], [fy, -fx]];
      for (const [dx, dy] of directions) {
        s.bolts.push({ kind: "comet", x: p.x, y: p.y, vx: dx * 540, vy: dy * 540, damage: c.damage, life: 1100, hits: [] });
      }
      s.cooldowns.comet = c.cooldown * haste;
    } else if (id === "ping") {
      const ping = pingStats(level);
      for (const e of s.enemies) {
        const reach = ping.radius + ENEMIES[e.kind].radius;
        if ((e.x - p.x) ** 2 + (e.y - p.y) ** 2 < reach * reach) hit(s, e, ping.damage, p.x, p.y, 260);
      }
      s.rings.push({ x: p.x, y: p.y, radius: ping.radius, life: 420, max: 420 });
      s.cooldowns.ping = ping.cooldown * haste;
    }
  }
}

function moveBolts(s: SwarmState, sec: number, dt: number) {
  const cells = grid(s.enemies);
  for (let i = s.bolts.length - 1; i >= 0; i--) {
    const b = s.bolts[i];
    b.x += b.vx * sec;
    b.y += b.vy * sec;
    b.life -= dt;
    const radius = b.kind === "comet" ? 8 : 4;
    let spent = false;
    nearby(cells, b.x, b.y, radius + 30, (j) => {
      if (spent) return;
      const e = s.enemies[j];
      if (e.hp <= 0 || b.hits.includes(e.id)) return;
      const reach = radius + ENEMIES[e.kind].radius;
      if ((e.x - b.x) ** 2 + (e.y - b.y) ** 2 >= reach * reach) return;
      hit(s, e, b.damage, b.x - b.vx, b.y - b.vy, b.kind === "comet" ? 120 : 60);
      if (b.kind === "comet") b.hits.push(e.id);
      else spent = true;
    });
    if (spent || b.life <= 0) {
      s.bolts[i] = s.bolts[s.bolts.length - 1];
      s.bolts.pop();
    }
  }
}

/** Where the orbit beads are right now, for hits and drawing. */
export function orbitBeads(s: SwarmState): Input[] {
  const level = s.weapons.orbit;
  if (!level) return [];
  const { count, radius } = orbitStats(level);
  const turn = (s.time / 1000) * 3.2;
  return Array.from({ length: count }, (_, i) => {
    const angle = turn + (i * Math.PI * 2) / count;
    return { x: s.player.x + Math.cos(angle) * radius, y: s.player.y + Math.sin(angle) * radius };
  });
}

function orbitHits(s: SwarmState) {
  const beads = orbitBeads(s);
  if (!beads.length) return;
  const { damage } = orbitStats(s.weapons.orbit);
  const cells = grid(s.enemies);
  for (const bead of beads) {
    nearby(cells, bead.x, bead.y, 40, (j) => {
      const e = s.enemies[j];
      if (e.orbitCd > 0 || e.hp <= 0) return;
      const reach = 6 + ENEMIES[e.kind].radius;
      if ((e.x - bead.x) ** 2 + (e.y - bead.y) ** 2 >= reach * reach) return;
      hit(s, e, damage, s.player.x, s.player.y, 90);
      e.orbitCd = 380;
    });
  }
}

function reapEnemies(s: SwarmState) {
  for (let i = s.enemies.length - 1; i >= 0; i--) {
    const e = s.enemies[i];
    if (e.hp > 0) continue;
    s.kills += 1;
    dropToken(s, e.x, e.y, ENEMIES[e.kind].xp);
    burst(s, e.x, e.y, e.kind === "segfault" ? 26 : 6, e.kind === "segfault" ? "danger" : "text", e.kind === "segfault" ? 200 : 90);
    if (e.kind === "segfault") s.shake = 380;
    s.enemies[i] = s.enemies[s.enemies.length - 1];
    s.enemies.pop();
  }
}

function dropToken(s: SwarmState, x: number, y: number, value: number) {
  if (s.tokens.length >= MAX_TOKENS) {
    // Past the cap, a drop tops up an existing token instead of adding one.
    s.tokens[Math.floor(rand(s) * s.tokens.length)].value += value;
    return;
  }
  s.tokens.push({ x, y, value, flying: false });
}

function contact(s: SwarmState) {
  const p = s.player;
  if (p.invuln > 0) return;
  for (const e of s.enemies) {
    const reach = PLAYER_RADIUS + ENEMIES[e.kind].radius - 2;
    if ((e.x - p.x) ** 2 + (e.y - p.y) ** 2 >= reach * reach) continue;
    p.hp -= ENEMIES[e.kind].damage;
    p.invuln = 800;
    s.shake = 180;
    burst(s, p.x, p.y, 8, "danger", 120);
    return;
  }
}

function collectTokens(s: SwarmState, sec: number) {
  const p = s.player;
  const magnet = magnetOf(s);
  for (let i = s.tokens.length - 1; i >= 0; i--) {
    const t = s.tokens[i];
    const dx = p.x - t.x;
    const dy = p.y - t.y;
    const d = Math.hypot(dx, dy) || 1;
    if (d < magnet) t.flying = true;
    if (t.flying) {
      const pull = Math.min(d, (380 + Math.max(0, magnet - d) * 4) * sec);
      t.x += (dx / d) * pull;
      t.y += (dy / d) * pull;
    }
    if (d < PLAYER_RADIUS + 6) {
      gainXp(s, t.value);
      s.tokens[i] = s.tokens[s.tokens.length - 1];
      s.tokens.pop();
    }
  }
}

export function gainXp(s: SwarmState, amount: number): void {
  const p = s.player;
  p.xp += amount;
  while (p.xp >= xpForLevel(p.level)) {
    p.xp -= xpForLevel(p.level);
    p.level += 1;
    s.pendingLevels += 1;
  }
  if (s.pendingLevels > 0 && !s.choices) s.choices = upgradeChoices(s);
}

function burst(s: SwarmState, x: number, y: number, count: number, tone: Tone, speed: number) {
  for (let i = 0; i < count && s.particles.length < MAX_PARTICLES; i++) {
    const angle = rand(s) * Math.PI * 2;
    const v = speed * (0.4 + rand(s) * 0.8);
    const life = 260 + rand(s) * 240;
    s.particles.push({ x, y, vx: Math.cos(angle) * v, vy: Math.sin(angle) * v, life, max: life, size: 1.5 + rand(s) * 2, tone });
  }
}

function ageEffects(s: SwarmState, sec: number, dt: number) {
  for (let i = s.particles.length - 1; i >= 0; i--) {
    const particle = s.particles[i];
    particle.x += particle.vx * sec;
    particle.y += particle.vy * sec;
    particle.vx *= 0.92;
    particle.vy *= 0.92;
    particle.life -= dt;
    if (particle.life <= 0) { s.particles[i] = s.particles[s.particles.length - 1]; s.particles.pop(); }
  }
  for (let i = s.rings.length - 1; i >= 0; i--) {
    s.rings[i].life -= dt;
    if (s.rings[i].life <= 0) { s.rings[i] = s.rings[s.rings.length - 1]; s.rings.pop(); }
  }
}
