/**
 * Quack Survivors: the Games panel's survivors-like. The duck holds out against a swarm of bugs
 * in an endless arena while its weapons fire on their own; defeated bugs drop tokens, chaos events
 * twist the run, and every level offers a pick of three upgrades — max a weapon beside its paired
 * passive and it evolves. Surviving `WIN_MS` wins the run.
 *
 * This module is the whole simulation and nothing else: plain mutable state, no DOM, no clock.
 * `QuackSurvivors.tsx` steps it at `STEP_MS` from a rAF accumulator and `quack-render.ts` draws
 * it. `save.ts` persists the state verbatim, so every field here must stay JSON-safe and any new
 * field needs a default in the save normaliser.
 *
 * Invariants:
 * - Deterministic: all randomness comes from the seeded generator in `state.rng`, so the same
 *   seed and inputs replay the same run (the tests rely on it).
 * - `step` does nothing while a level-up choice is open (`state.choices`) or the run has ended.
 * - Lists are capped (`MAX_ENEMIES`, `MAX_TOKENS`, `MAX_PARTICLES`, `MAX_SPITS`, `MAX_ARCS`) so a
 *   long run stays cheap.
 * - The state is JSON-safe: `games/session.ts` keeps it across panel remounts and `games/save.ts`
 *   across app launches.
 */

export const STEP_MS = 1000 / 60;
export const WIN_MS = 5 * 60_000;
export const MAX_LEVEL = 5;
export const MAX_ENEMIES = 320;
export const MAX_TOKENS = 260;
export const MAX_PARTICLES = 360;
export const MAX_SPITS = 90;
export const MAX_ARCS = 40;
export const PLAYER_RADIUS = 12;

export type WeaponId = "quill" | "orbit" | "comet" | "ping" | "flock" | "surge" | "ember";
export type PassiveId = "speed" | "magnet" | "vitality" | "haste" | "thorns" | "crop" | "shell";
export type EvolutionId = "volley" | "nebula" | "nova" | "shockwave" | "storm" | "overload" | "inferno";
export type UpgradeId = WeaponId | PassiveId | EvolutionId | "snack";
export type EnemyKind = "bug" | "gnat" | "beetle" | "segfault" | "splitter" | "mosquito";
export type EliteKind = "swift" | "armored" | "explosive";
export type Tone = "accent" | "danger" | "text";

export const WEAPONS: readonly WeaponId[] = ["quill", "orbit", "comet", "ping", "flock", "surge", "ember"];
export const PASSIVES: readonly PassiveId[] = ["speed", "magnet", "vitality", "haste", "thorns", "crop", "shell"];
export const EVOLUTIONS: readonly EvolutionId[] = ["volley", "nebula", "nova", "shockwave", "storm", "overload", "inferno"];
export const UPGRADE_IDS: readonly UpgradeId[] = [...WEAPONS, ...PASSIVES, ...EVOLUTIONS, "snack"];

/** Which weapon each evolution grows out of, and the passive that must be owned first. */
export const EVOLUTION_PAIRS: Record<EvolutionId, { weapon: WeaponId; passive: PassiveId }> = {
  volley: { weapon: "quill", passive: "haste" },
  nebula: { weapon: "orbit", passive: "magnet" },
  nova: { weapon: "comet", passive: "speed" },
  shockwave: { weapon: "ping", passive: "vitality" },
  storm: { weapon: "flock", passive: "crop" },
  overload: { weapon: "surge", passive: "shell" },
  inferno: { weapon: "ember", passive: "thorns" }
};

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
  /** Radius/HP/damage multiplier: splitter children are smaller copies of their parent. */
  scale: number;
  elite: EliteKind | null;
  /** ms until the ember aura may tick this enemy again. */
  auraCd: number;
  /** ms until the mosquito may spit again. */
  spitCd: number;
}

export interface Bolt {
  kind: "quill" | "comet" | "flock";
  x: number;
  y: number;
  vx: number;
  vy: number;
  damage: number;
  life: number;
  /** Enemies this bolt has already hit. */
  hits: number[];
  /** Enemies it may hit in total before it is spent (comets pierce freely). */
  limit: number;
}

/** A mosquito's slow ranged spit, aimed where the duck stood when it was loosed. */
export interface Spit { x: number; y: number; vx: number; vy: number; damage: number; life: number }

/** A fading jagged lightning polyline; `points` is a flat x,y list. */
export interface Arc { points: number[]; life: number; max: number }

export interface Token { x: number; y: number; value: number; flying: boolean }
export interface Ring { x: number; y: number; radius: number; life: number; max: number }
export interface Particle { x: number; y: number; vx: number; vy: number; life: number; max: number; size: number; tone: Tone }
export interface Banner { text: string; until: number }

export interface QuackState {
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
  /** Weapons that have evolved past MAX_LEVEL. */
  evolved: Partial<Record<WeaponId, true>>;
  spits: Spit[];
  arcs: Arc[];
  /** Run time of the next chaos event. */
  nextChaos: number;
}

interface EnemySpec { radius: number; hp: number; speed: number; damage: number; xp: number; mass: number }

export const ENEMIES: Record<EnemyKind, EnemySpec> = {
  bug: { radius: 9, hp: 14, speed: 80, damage: 6, xp: 1, mass: 1 },
  gnat: { radius: 6, hp: 6, speed: 135, damage: 4, xp: 1, mass: 0.7 },
  beetle: { radius: 14, hp: 60, speed: 54, damage: 14, xp: 4, mass: 3 },
  splitter: { radius: 13, hp: 46, speed: 64, damage: 11, xp: 3, mass: 1.7 },
  mosquito: { radius: 8, hp: 24, speed: 100, damage: 6, xp: 2, mass: 0.8 },
  segfault: { radius: 30, hp: 900, speed: 72, damage: 25, xp: 40, mass: 12 }
};

/** The Segfault mini-bosses arrive at these run times; the last one is a Kernel Panic. */
export const BOSS_TIMES = [120_000, 210_000, 280_000];

/** Radius of an enemy with its scale applied: splitter children are smaller. */
export function radiusOf(e: Enemy): number { return ENEMIES[e.kind].radius * e.scale; }
function massOf(e: Enemy): number { return ENEMIES[e.kind].mass * e.scale; }
function damageOf(e: Enemy): number { return ENEMIES[e.kind].damage * e.scale; }

// ── Stats ────────────────────────────────────────────

export function speedOf(s: QuackState): number { return 150 * (1 + 0.1 * s.passives.speed); }
export function magnetOf(s: QuackState): number { return 80 * (1 + 0.4 * s.passives.magnet); }
export function maxHpOf(s: QuackState): number { return 100 + 20 * s.passives.vitality; }
function regenOf(s: QuackState): number { return 0.6 + 0.6 * s.passives.vitality; }
function hasteOf(s: QuackState): number { return 1 - 0.08 * s.passives.haste; }

export function quillStats(level: number, evolved = false) {
  return {
    count: 1 + Math.floor((level - 1) / 2) + (evolved ? 2 : 0),
    damage: 12 + 4 * (level - 1),
    cooldown: (800 - 80 * (level - 1)) * (evolved ? 0.7 : 1),
    /** Volley bolts pierce three bugs instead of one. */
    limit: evolved ? 3 : 1
  };
}
export function orbitStats(level: number, evolved = false) {
  return { count: 1 + level + (evolved ? 3 : 0), damage: 8 + 3 * level, radius: Math.round((46 + 4 * level) * (evolved ? 1.3 : 1)) };
}
export function cometStats(level: number, evolved = false) {
  const directions = evolved ? 8 : level >= 5 ? 4 : level >= 3 ? 2 : 1;
  return { directions, damage: 30 + 10 * (level - 1), cooldown: (2200 - 200 * (level - 1)) * (evolved ? 0.6 : 1) };
}
export function pingStats(level: number, evolved = false) {
  return { radius: Math.round((90 + 18 * (level - 1)) * (evolved ? 1.8 : 1)), damage: 10 + 6 * (level - 1), cooldown: 3000 - 250 * (level - 1), heal: evolved ? 6 : 0 };
}
export function flockStats(level: number, evolved = false) {
  return {
    count: 2 + Math.floor(level / 2) + (evolved ? 4 : 0),
    damage: Math.round((9 + 4 * level) * (evolved ? 1.4 : 1)),
    cooldown: 1500 - 80 * level
  };
}
export function surgeStats(level: number, evolved = false) {
  return {
    links: 2 + level + (evolved ? 4 : 0),
    damage: 16 + 5 * level,
    /** Distance the arc may jump between enemies. */
    range: 110,
    cooldown: 2600 - 160 * level,
    /** Damage kept per hop; Overload keeps all of it. */
    decay: evolved ? 1 : 0.82
  };
}
export function emberStats(level: number, evolved = false) {
  return {
    radius: Math.round((40 + 6 * level) * (evolved ? 1.6 : 1)),
    damage: 6 + 3 * level,
    tick: evolved ? 130 : 220,
    knock: evolved ? 240 : 80
  };
}

/** Tokens needed to leave `level`. */
export function xpForLevel(level: number): number { return 4 + 3 * level; }

export function score(s: QuackState): number {
  const base = s.kills + 2 * Math.floor(s.time / 1000);
  return s.phase === "won" ? base * 2 : base;
}

// ── Upgrades ─────────────────────────────────────────

export const UPGRADE_NAMES: Record<UpgradeId, string> = {
  quill: "Quill", orbit: "Orbit", comet: "Comet", ping: "Ping", flock: "Ducklings", surge: "Surge", ember: "Ember",
  speed: "Quick feet", magnet: "Long context", vitality: "Rubber hide", haste: "Hot loop",
  thorns: "Barbed feathers", crop: "Bottomless crop", shell: "Hard shell",
  volley: "Quill Volley", nebula: "Orbit Nebula", nova: "Comet Nova", shockwave: "Shockwave",
  storm: "Duck Storm", overload: "Overload", inferno: "Inferno", snack: "Bread crumbs"
};

export function upgradeLevel(s: QuackState, id: UpgradeId): number {
  if (id === "snack" || isEvolution(id)) return 0;
  return isWeapon(id) ? s.weapons[id] : s.passives[id];
}

function isWeapon(id: UpgradeId): id is WeaponId { return (WEAPONS as readonly string[]).includes(id); }
function isEvolution(id: UpgradeId): id is EvolutionId { return (EVOLUTIONS as readonly string[]).includes(id); }

/** One line describing what picking `id` gives at its next level. */
export function upgradeDetail(s: QuackState, id: UpgradeId): string {
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
      return next === 1 ? "A piercing streak the way you face." : `${c.directions === 1 ? "Forward" : c.directions === 2 ? "Forward and back" : c.directions === 4 ? "Four ways" : "Eight ways"} · ${c.damage} damage`;
    }
    case "ping": {
      const p = pingStats(next);
      return next === 1 ? "A ring pulses out and knocks bugs back." : `Wider ring · ${p.damage} damage`;
    }
    case "flock": {
      const f = flockStats(next);
      return next === 1 ? "Ducklings launch and home in on bugs." : `${f.count} ducklings · ${f.damage} damage`;
    }
    case "surge": {
      const g = surgeStats(next);
      return next === 1 ? "Lightning arcs from bug to bug." : `${g.links} links · ${g.damage} damage`;
    }
    case "ember":
      return next === 1 ? "A ring of fire burns anything it touches." : `Wider ring · faster ticks`;
    case "speed": return "Move 10% faster.";
    case "magnet": return "Pull tokens in from further away.";
    case "vitality": return "+20 max HP and slow healing.";
    case "haste": return "Weapons recharge 8% faster.";
    case "thorns": return "Bugs that touch you take damage and get shoved.";
    case "crop": return "Tokens are worth 25% more XP.";
    case "shell": return "Take 2 less damage from every hit.";
    case "snack": return "Restore 30 HP.";
    case "volley": return "Quill evolves: piercing bolts, more of them, recharging faster.";
    case "nebula": return "Orbit evolves: more beads in a wider ring.";
    case "nova": return "Comet evolves: eight ways at once.";
    case "shockwave": return "Ping evolves: a huge ring that also heals you.";
    case "storm": return "Flock evolves: a formation of ducklings, hitting harder.";
    case "overload": return "Surge evolves: longer chains that keep their punch.";
    case "inferno": return "Ember evolves: a bigger blaze that shoves bugs away.";
  }
}

/** Up to three distinct upgrades that aren't maxed or already evolved; a snack when everything is. */
export function upgradeChoices(s: QuackState): UpgradeId[] {
  const pool: UpgradeId[] = [...WEAPONS, ...PASSIVES].filter((id) => upgradeLevel(s, id) < MAX_LEVEL);
  for (const evo of EVOLUTIONS) {
    const { weapon, passive } = EVOLUTION_PAIRS[evo];
    if (s.evolved[weapon]) continue;
    if (s.weapons[weapon] >= MAX_LEVEL && s.passives[passive] >= 1) pool.push(evo);
  }
  const picks: UpgradeId[] = [];
  while (picks.length < 3 && pool.length) picks.push(pool.splice(Math.floor(rand(s) * pool.length), 1)[0]);
  return picks.length ? picks : ["snack"];
}

export function applyUpgrade(s: QuackState, id: UpgradeId): void {
  if (!s.choices?.includes(id)) return;
  if (id === "snack") s.player.hp = Math.min(maxHpOf(s), s.player.hp + 30);
  else if (isEvolution(id)) s.evolved[EVOLUTION_PAIRS[id].weapon] = true;
  else if (isWeapon(id)) s.weapons[id] = Math.min(MAX_LEVEL, s.weapons[id] + 1);
  else {
    s.passives[id] = Math.min(MAX_LEVEL, s.passives[id] + 1);
    if (id === "vitality") s.player.hp = Math.min(maxHpOf(s), s.player.hp + 20);
  }
  s.pendingLevels -= 1;
  s.choices = s.pendingLevels > 0 ? upgradeChoices(s) : null;
}

// ── Setup ────────────────────────────────────────────

export function createQuack(seed: number, view = { w: 480, h: 640 }): QuackState {
  return {
    rng: seed >>> 0 || 1,
    nextId: 1,
    time: 0,
    phase: "playing",
    view: { ...view },
    player: { x: 0, y: 0, hp: 100, facing: { x: 1, y: 0 }, invuln: 0, level: 1, xp: 0 },
    weapons: { quill: 1, orbit: 0, comet: 0, ping: 0, flock: 0, surge: 0, ember: 0 },
    passives: { speed: 0, magnet: 0, vitality: 0, haste: 0, thorns: 0, crop: 0, shell: 0 },
    cooldowns: { quill: 400, orbit: 0, comet: 0, ping: 0, flock: 0, surge: 0, ember: 0 },
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
    shake: 0,
    evolved: {},
    spits: [],
    arcs: [],
    nextChaos: 60_000
  };
}

export function setView(s: QuackState, w: number, h: number): void {
  s.view.w = w;
  s.view.h = h;
}

/** mulberry32 over `state.rng`. */
export function rand(s: QuackState): number {
  s.rng = (s.rng + 0x6d2b79f5) >>> 0;
  let t = s.rng;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

// ── Step ─────────────────────────────────────────────

export function step(s: QuackState, input: Input, dt: number): void {
  if (s.phase !== "playing" || s.choices) return;
  const sec = dt / 1000;
  s.time += dt;
  s.shake = Math.max(0, s.shake - dt);
  if (s.banner && s.time > s.banner.until) s.banner = null;

  movePlayer(s, input, sec);
  spawn(s, sec);
  moveEnemies(s, sec, dt);
  fireWeapons(s, dt);
  emberHits(s);
  moveBolts(s, sec, dt);
  moveSpits(s, sec, dt);
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

function movePlayer(s: QuackState, input: Input, sec: number) {
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

function spawn(s: QuackState, sec: number) {
  // Chaos first, bosses second: when both land on the same step the boss announcement wins.
  if (s.time >= s.nextChaos) {
    chaos(s);
    s.nextChaos = s.time + 50_000 + Math.floor(rand(s) * 15_000);
  }
  if (s.bosses < BOSS_TIMES.length && s.time >= BOSS_TIMES[s.bosses]) {
    s.bosses += 1;
    const panic = s.bosses >= BOSS_TIMES.length;
    addEnemy(s, "segfault", undefined, { scale: panic ? 3 : 1 });
    s.banner = { text: panic ? "Kernel Panic approaches" : "A Segfault approaches", until: s.time + 2600 };
    s.shake = 400;
  }
  s.spawnDebt += spawnRate(s.time) * sec;
  while (s.spawnDebt >= 1) {
    s.spawnDebt -= 1;
    if (s.enemies.length >= MAX_ENEMIES) { s.spawnDebt = 0; break; }
    const roll = rand(s);
    const seconds = s.time / 1000;
    let kind: EnemyKind = "bug";
    let pack = 1;
    if (seconds > 45 && roll < Math.min(0.16, 0.05 + seconds / 4000)) kind = "splitter";
    else if (seconds > 25 && roll < Math.min(0.2, 0.07 + seconds / 3000)) kind = "mosquito";
    else if (seconds > 60 && roll < Math.min(0.26, 0.12 + seconds / 2000)) kind = "beetle";
    else if (seconds > 30 && roll > 0.78) { kind = "gnat"; pack = 5; }
    if (pack > 1) {
      // Gnats arrive as a pack from one direction.
      const angle = rand(s) * Math.PI * 2;
      for (let i = 0; i < pack && s.enemies.length < MAX_ENEMIES; i++) addEnemy(s, kind, angle + (rand(s) - 0.5) * 0.35, {});
    } else {
      addEnemy(s, kind, undefined, { elite: maybeElite(s, kind) });
    }
  }
}

/** Elites join the swarm late: a rising chance of one modifier, capped so they stay special. */
function maybeElite(s: QuackState, kind: EnemyKind): EliteKind | null {
  if (kind === "segfault") return null;
  const seconds = s.time / 1000;
  if (seconds <= 90 || rand(s) >= Math.min(0.1, 0.02 + seconds / 8000)) return null;
  let alive = 0;
  for (const e of s.enemies) if (e.elite) alive += 1;
  if (alive >= 8) return null;
  const roll = rand(s);
  return roll < 1 / 3 ? "swift" : roll < 2 / 3 ? "armored" : "explosive";
}

/** A mid-run twist, announced by the banner: free tokens, a ring of bugs, or a damaging pulse. */
function chaos(s: QuackState) {
  const p = s.player;
  const roll = rand(s);
  if (roll < 1 / 3) {
    s.banner = { text: "Token shower!", until: s.time + 2400 };
    for (let i = 0; i < 26; i++) {
      const angle = rand(s) * Math.PI * 2;
      const dist = 70 + rand(s) * 60;
      dropToken(s, p.x + Math.cos(angle) * dist, p.y + Math.sin(angle) * dist, 1);
    }
  } else if (roll < 2 / 3) {
    s.banner = { text: "Bug rush!", until: s.time + 2400 };
    for (let i = 0; i < 12 && s.enemies.length < MAX_ENEMIES; i++) addEnemy(s, "bug", (i / 12) * Math.PI * 2 + rand(s) * 0.2, {});
    for (let i = 0; i < 6 && s.enemies.length < MAX_ENEMIES; i++) addEnemy(s, "gnat", (i / 6) * Math.PI * 2 + rand(s) * 0.2, {});
  } else {
    s.banner = { text: "EMP!", until: s.time + 2400 };
    s.rings.push({ x: p.x, y: p.y, radius: 260, life: 420, max: 420 });
    const damage = 100 + 8 * p.level;
    for (const e of s.enemies) hit(s, e, damage, p.x, p.y, 300);
    s.shake = 260;
  }
}

/** Just past the corner of the view: bugs arrive from off screen. */
function spawnDistance(s: QuackState, kind: EnemyKind, scale = 1): number {
  return Math.hypot(s.view.w, s.view.h) / 2 + 30 + ENEMIES[kind].radius * scale;
}

/** Bugs get a little quicker as the run goes on. */
function enemySpeed(s: QuackState, kind: EnemyKind): number {
  return ENEMIES[kind].speed * (1 + 0.03 * (s.time / 60_000));
}

interface SpawnOpts { elite?: EliteKind | null; scale?: number; at?: { x: number; y: number } }

function addEnemy(s: QuackState, kind: EnemyKind, angle = rand(s) * Math.PI * 2, opts: SpawnOpts = {}) {
  const spec = ENEMIES[kind];
  const scale = opts.scale ?? 1;
  const distance = spawnDistance(s, kind, scale);
  const armored = opts.elite === "armored" ? 2.4 : 1;
  const hp = Math.round(spec.hp * (kind === "segfault" ? s.bosses : 1 + (s.time / 60_000) * 0.35) * scale * armored);
  const x = opts.at ? opts.at.x + Math.cos(angle) * 6 : s.player.x + Math.cos(angle) * distance;
  const y = opts.at ? opts.at.y + Math.sin(angle) * 6 : s.player.y + Math.sin(angle) * distance;
  s.enemies.push({
    id: s.nextId++, kind, x, y, kx: 0, ky: 0, hp, maxHp: hp, flash: 0, orbitCd: 0,
    scale, elite: opts.elite ?? null, auraCd: 0, spitCd: 1400
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

function moveEnemies(s: QuackState, sec: number, dt: number) {
  const { x: px, y: py } = s.player;
  const decay = Math.exp(-sec * 8);
  const heading = Math.atan2(s.player.facing.y, s.player.facing.x);
  for (const e of s.enemies) {
    const dx = px - e.x;
    const dy = py - e.y;
    const distance = Math.hypot(dx, dy) || 1;
    const far = spawnDistance(s, e.kind, e.scale);
    if (distance > far * 1.6) {
      // A straggler left far behind re-enters ahead of the duck, so out-running the swarm
      // never empties the screen.
      const angle = heading + (rand(s) - 0.5) * 1.6;
      e.x = px + Math.cos(angle) * far;
      e.y = py + Math.sin(angle) * far;
      continue;
    }
    const speed = enemySpeed(s, e.kind) * (e.elite === "swift" ? 1.45 : 1);
    if (e.kind === "mosquito") {
      // The ranged one: holds 150–210px, strafes, and spits.
      const side = e.id % 2 === 0 ? 1 : -1;
      let mx = 0;
      let my = 0;
      if (distance < 150) { mx = -dx / distance; my = -dy / distance; }
      else if (distance > 210) { mx = dx / distance; my = dy / distance; }
      else { mx = (-dy / distance) * side; my = (dx / distance) * side; }
      e.x += mx * speed * 0.8 * sec + e.kx * sec;
      e.y += my * speed * 0.8 * sec + e.ky * sec;
      e.spitCd -= dt;
      if (e.spitCd <= 0 && distance < 340 && s.spits.length < MAX_SPITS) {
        s.spits.push({ x: e.x, y: e.y, vx: (dx / distance) * 150, vy: (dy / distance) * 150, damage: 9, life: 2800 });
        e.spitCd = 2400;
      }
    } else {
      e.x += (dx / distance) * speed * sec + e.kx * sec;
      e.y += (dy / distance) * speed * sec + e.ky * sec;
    }
    e.kx *= decay;
    e.ky *= decay;
    e.flash = Math.max(0, e.flash - dt);
    e.orbitCd = Math.max(0, e.orbitCd - dt);
    e.auraCd = Math.max(0, e.auraCd - dt);
  }
  // Soft separation keeps the swarm a crowd instead of one stacked blob.
  const cells = grid(s.enemies);
  for (let i = 0; i < s.enemies.length; i++) {
    const a = s.enemies[i];
    const ra = radiusOf(a);
    nearby(cells, a.x, a.y, ra + 30, (j) => {
      if (j <= i) return;
      const b = s.enemies[j];
      const min = ra + radiusOf(b);
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d2 = dx * dx + dy * dy;
      if (d2 >= min * min || d2 === 0) return;
      const d = Math.sqrt(d2);
      const push = (min - d) * 0.5;
      const ma = massOf(a);
      const mb = massOf(b);
      const share = mb / (ma + mb);
      a.x -= (dx / d) * push * share;
      a.y -= (dy / d) * push * share;
      b.x += (dx / d) * push * (1 - share);
      b.y += (dy / d) * push * (1 - share);
    });
  }
}

function hit(s: QuackState, e: Enemy, damage: number, fromX: number, fromY: number, knock: number) {
  const dealt = e.elite === "armored" ? damage * 0.6 : damage;
  e.hp -= dealt;
  e.flash = 90;
  const dx = e.x - fromX;
  const dy = e.y - fromY;
  const d = Math.hypot(dx, dy) || 1;
  const mass = massOf(e);
  e.kx += (dx / d) * knock / mass;
  e.ky += (dy / d) * knock / mass;
}

function fireWeapons(s: QuackState, dt: number) {
  const haste = hasteOf(s);
  const p = s.player;
  for (const id of WEAPONS) {
    const level = s.weapons[id];
    if (!level || id === "orbit" || id === "ember") continue;
    s.cooldowns[id] -= dt;
    if (s.cooldowns[id] > 0) continue;
    if (id === "quill") {
      const q = quillStats(level, s.evolved.quill);
      const targets = s.enemies
        .map((e) => ({ e, d: (e.x - p.x) ** 2 + (e.y - p.y) ** 2 }))
        .filter(({ d }) => d < 520 * 520)
        .sort((a, b) => a.d - b.d)
        .slice(0, q.count);
      if (!targets.length) { s.cooldowns.quill = 0; continue; }
      for (const { e } of targets) {
        const d = Math.hypot(e.x - p.x, e.y - p.y) || 1;
        s.bolts.push({ kind: "quill", x: p.x, y: p.y, vx: (e.x - p.x) / d * 430, vy: (e.y - p.y) / d * 430, damage: q.damage, life: 1000, hits: [], limit: q.limit });
      }
      s.cooldowns.quill = q.cooldown * haste;
    } else if (id === "comet") {
      const c = cometStats(level, s.evolved.comet);
      const { x: fx, y: fy } = p.facing;
      // Evenly spaced rotations of the facing vector: 1, 2, 4 or (evolved) 8 ways.
      const directions: [number, number][] = [];
      for (let k = 0; k < c.directions; k++) {
        const a = (k * Math.PI * 2) / c.directions;
        directions.push([fx * Math.cos(a) - fy * Math.sin(a), fx * Math.sin(a) + fy * Math.cos(a)]);
      }
      for (const [dx, dy] of directions) {
        s.bolts.push({ kind: "comet", x: p.x, y: p.y, vx: dx * 540, vy: dy * 540, damage: c.damage, life: 1100, hits: [], limit: 999 });
      }
      s.cooldowns.comet = c.cooldown * haste;
    } else if (id === "ping") {
      const ping = pingStats(level, s.evolved.ping);
      for (const e of s.enemies) {
        const reach = ping.radius + radiusOf(e);
        if ((e.x - p.x) ** 2 + (e.y - p.y) ** 2 < reach * reach) hit(s, e, ping.damage, p.x, p.y, 260);
      }
      if (ping.heal) p.hp = Math.min(maxHpOf(s), p.hp + ping.heal);
      s.rings.push({ x: p.x, y: p.y, radius: ping.radius, life: 420, max: 420 });
      s.cooldowns.ping = ping.cooldown * haste;
    } else if (id === "flock") {
      const f = flockStats(level, s.evolved.flock);
      const targets = s.enemies
        .map((e) => ({ e, d: (e.x - p.x) ** 2 + (e.y - p.y) ** 2 }))
        .filter(({ d }) => d < 520 * 520)
        .sort((a, b) => a.d - b.d)
        .slice(0, f.count);
      if (!targets.length) { s.cooldowns.flock = 0; continue; }
      for (const { e } of targets) {
        const d = Math.hypot(e.x - p.x, e.y - p.y) || 1;
        s.bolts.push({ kind: "flock", x: p.x, y: p.y, vx: (e.x - p.x) / d * 240, vy: (e.y - p.y) / d * 240, damage: f.damage, life: 1400, hits: [], limit: 1 });
      }
      s.cooldowns.flock = f.cooldown * haste;
    } else if (id === "surge") {
      const g = surgeStats(level, s.evolved.surge);
      const targets = chainTargets(s, g.links, g.range);
      if (!targets.length) { s.cooldowns.surge = 0; continue; }
      const points = [p.x, p.y];
      let damage = g.damage;
      let fromX = p.x;
      let fromY = p.y;
      for (const e of targets) {
        hit(s, e, damage, fromX, fromY, 60);
        points.push(e.x, e.y);
        damage = Math.round(damage * g.decay);
        fromX = e.x;
        fromY = e.y;
      }
      if (s.arcs.length < MAX_ARCS) s.arcs.push({ points, life: 220, max: 220 });
      s.cooldowns.surge = g.cooldown * haste;
    }
  }
}

/** Nearest enemy, then the nearest unhit neighbour within `range`, until the chain runs out. */
function chainTargets(s: QuackState, links: number, range: number): Enemy[] {
  const out: Enemy[] = [];
  const taken = new Set<number>();
  let fromX = s.player.x;
  let fromY = s.player.y;
  while (out.length < links) {
    const reach = out.length === 0 ? 520 : range;
    let best: Enemy | null = null;
    let bestD = reach * reach;
    for (const e of s.enemies) {
      if (e.hp <= 0 || taken.has(e.id)) continue;
      const d2 = (e.x - fromX) ** 2 + (e.y - fromY) ** 2;
      if (d2 < bestD) { bestD = d2; best = e; }
    }
    if (!best) break;
    out.push(best);
    taken.add(best.id);
    fromX = best.x;
    fromY = best.y;
  }
  return out;
}

/** The ember aura: a standing ring of damage that ticks each enemy in reach. */
function emberHits(s: QuackState) {
  const level = s.weapons.ember;
  if (!level) return;
  const { radius, damage, tick, knock } = emberStats(level, s.evolved.ember);
  const p = s.player;
  for (const e of s.enemies) {
    if (e.auraCd > 0 || e.hp <= 0) continue;
    const reach = radius + radiusOf(e);
    if ((e.x - p.x) ** 2 + (e.y - p.y) ** 2 >= reach * reach) continue;
    hit(s, e, damage, p.x, p.y, knock);
    e.auraCd = tick;
  }
}

function moveBolts(s: QuackState, sec: number, dt: number) {
  const cells = grid(s.enemies);
  for (let i = s.bolts.length - 1; i >= 0; i--) {
    const b = s.bolts[i];
    if (b.kind === "flock") {
      // Ducklings home in on the nearest bug, turning gently.
      let best: Enemy | null = null;
      let bestD = 460 * 460;
      for (const e of s.enemies) {
        if (e.hp <= 0 || b.hits.includes(e.id)) continue;
        const d2 = (e.x - b.x) ** 2 + (e.y - b.y) ** 2;
        if (d2 < bestD) { bestD = d2; best = e; }
      }
      const speed = 300;
      if (best) {
        const want = Math.atan2(best.y - b.y, best.x - b.x);
        const have = Math.atan2(b.vy, b.vx);
        let diff = ((want - have + Math.PI * 3) % (Math.PI * 2)) - Math.PI;
        const turn = Math.max(-5 * sec, Math.min(5 * sec, diff));
        const angle = have + turn;
        b.vx = Math.cos(angle) * speed;
        b.vy = Math.sin(angle) * speed;
      } else {
        const have = Math.atan2(b.vy, b.vx);
        b.vx = Math.cos(have) * speed;
        b.vy = Math.sin(have) * speed;
      }
    }
    b.x += b.vx * sec;
    b.y += b.vy * sec;
    b.life -= dt;
    const radius = b.kind === "comet" ? 8 : b.kind === "flock" ? 5 : 4;
    let spent = false;
    nearby(cells, b.x, b.y, radius + 30, (j) => {
      if (spent) return;
      const e = s.enemies[j];
      if (e.hp <= 0 || b.hits.includes(e.id)) return;
      const reach = radius + radiusOf(e);
      if ((e.x - b.x) ** 2 + (e.y - b.y) ** 2 >= reach * reach) return;
      hit(s, e, b.damage, b.x - b.vx, b.y - b.vy, b.kind === "comet" ? 120 : b.kind === "flock" ? 40 : 60);
      b.hits.push(e.id);
      if (b.hits.length >= b.limit) spent = true;
    });
    if (spent || b.life <= 0) {
      s.bolts[i] = s.bolts[s.bolts.length - 1];
      s.bolts.pop();
    }
  }
}

function moveSpits(s: QuackState, sec: number, dt: number) {
  const p = s.player;
  for (let i = s.spits.length - 1; i >= 0; i--) {
    const spit = s.spits[i];
    spit.x += spit.vx * sec;
    spit.y += spit.vy * sec;
    spit.life -= dt;
    if (p.invuln <= 0 && (spit.x - p.x) ** 2 + (spit.y - p.y) ** 2 < (PLAYER_RADIUS + 7) ** 2) {
      p.hp -= spit.damage;
      p.invuln = 800;
      s.shake = Math.max(s.shake, 160);
      burst(s, p.x, p.y, 6, "danger", 110);
      spit.life = 0;
    }
    if (spit.life <= 0) {
      s.spits[i] = s.spits[s.spits.length - 1];
      s.spits.pop();
    }
  }
}

/** Where the orbit beads are right now, for hits and drawing. */
export function orbitBeads(s: QuackState): Input[] {
  const level = s.weapons.orbit;
  if (!level) return [];
  const { count, radius } = orbitStats(level, s.evolved.orbit);
  const turn = (s.time / 1000) * 3.2;
  return Array.from({ length: count }, (_, i) => {
    const angle = turn + (i * Math.PI * 2) / count;
    return { x: s.player.x + Math.cos(angle) * radius, y: s.player.y + Math.sin(angle) * radius };
  });
}

function orbitHits(s: QuackState) {
  const beads = orbitBeads(s);
  if (!beads.length) return;
  const { damage } = orbitStats(s.weapons.orbit, s.evolved.orbit);
  const cells = grid(s.enemies);
  for (const bead of beads) {
    nearby(cells, bead.x, bead.y, 40, (j) => {
      const e = s.enemies[j];
      if (e.orbitCd > 0 || e.hp <= 0) return;
      const reach = 6 + radiusOf(e);
      if ((e.x - bead.x) ** 2 + (e.y - bead.y) ** 2 >= reach * reach) return;
      hit(s, e, damage, s.player.x, s.player.y, 90);
      e.orbitCd = 380;
    });
  }
}

function reapEnemies(s: QuackState) {
  for (let i = s.enemies.length - 1; i >= 0; i--) {
    const e = s.enemies[i];
    if (e.hp > 0) continue;
    s.kills += 1;
    dropToken(s, e.x, e.y, Math.max(1, Math.round(ENEMIES[e.kind].xp * e.scale)));
    burst(s, e.x, e.y, e.kind === "segfault" ? 26 : 6, e.kind === "segfault" ? "danger" : "text", e.kind === "segfault" ? 200 : 90);
    if (e.kind === "segfault") s.shake = 380;
    if (e.kind === "splitter" && e.scale >= 0.9) {
      // Only full-size splitters divide, so the crowd can't fork without end.
      for (let c = 0; c < 2 && s.enemies.length < MAX_ENEMIES; c++) {
        addEnemy(s, "splitter", rand(s) * Math.PI * 2, { scale: e.scale * 0.55, at: { x: e.x, y: e.y } });
      }
    }
    if (e.elite === "explosive") {
      burst(s, e.x, e.y, 14, "danger", 200);
      s.shake = Math.max(s.shake, 220);
      const reach = 92;
      const p = s.player;
      if (p.invuln <= 0 && (e.x - p.x) ** 2 + (e.y - p.y) ** 2 < reach * reach) {
        p.hp -= 44;
        p.invuln = 800;
        burst(s, p.x, p.y, 8, "danger", 140);
      }
    }
    s.enemies[i] = s.enemies[s.enemies.length - 1];
    s.enemies.pop();
  }
}

function dropToken(s: QuackState, x: number, y: number, value: number) {
  if (s.tokens.length >= MAX_TOKENS) {
    // Past the cap, a drop tops up an existing token instead of adding one.
    s.tokens[Math.floor(rand(s) * s.tokens.length)].value += value;
    return;
  }
  s.tokens.push({ x, y, value, flying: false });
}

function contact(s: QuackState) {
  const p = s.player;
  if (p.invuln > 0) return;
  for (const e of s.enemies) {
    const reach = PLAYER_RADIUS + radiusOf(e) - 2;
    if ((e.x - p.x) ** 2 + (e.y - p.y) ** 2 >= reach * reach) continue;
    p.hp -= Math.max(1, damageOf(e) - 2 * s.passives.shell);
    p.invuln = 800;
    s.shake = 180;
    burst(s, p.x, p.y, 8, "danger", 120);
    if (s.passives.thorns > 0) hit(s, e, 8 + 6 * s.passives.thorns, p.x, p.y, 240);
    return;
  }
}

function collectTokens(s: QuackState, sec: number) {
  const p = s.player;
  const magnet = magnetOf(s);
  const boost = 1 + 0.25 * s.passives.crop;
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
      gainXp(s, Math.max(1, Math.round(t.value * boost)));
      s.tokens[i] = s.tokens[s.tokens.length - 1];
      s.tokens.pop();
    }
  }
}

export function gainXp(s: QuackState, amount: number): void {
  const p = s.player;
  p.xp += amount;
  while (p.xp >= xpForLevel(p.level)) {
    p.xp -= xpForLevel(p.level);
    p.level += 1;
    s.pendingLevels += 1;
  }
  if (s.pendingLevels > 0 && !s.choices) s.choices = upgradeChoices(s);
}

function burst(s: QuackState, x: number, y: number, count: number, tone: Tone, speed: number) {
  for (let i = 0; i < count && s.particles.length < MAX_PARTICLES; i++) {
    const angle = rand(s) * Math.PI * 2;
    const v = speed * (0.4 + rand(s) * 0.8);
    const life = 260 + rand(s) * 240;
    s.particles.push({ x, y, vx: Math.cos(angle) * v, vy: Math.sin(angle) * v, life, max: life, size: 1.5 + rand(s) * 2, tone });
  }
}

function ageEffects(s: QuackState, sec: number, dt: number) {
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
  for (let i = s.arcs.length - 1; i >= 0; i--) {
    s.arcs[i].life -= dt;
    if (s.arcs[i].life <= 0) { s.arcs[i] = s.arcs[s.arcs.length - 1]; s.arcs.pop(); }
  }
}
