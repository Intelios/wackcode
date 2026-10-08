import { describe, expect, it } from "vitest";
import { readSave, writeSave } from "./save";
import {
  BOSS_TIMES, ENEMIES, MAX_ENEMIES, MAX_LEVEL, MAX_SPITS, STEP_MS, WIN_MS, applyUpgrade, createQuack, flockStats, gainXp,
  quillStats, score, speedOf, step, surgeStats, upgradeChoices, upgradeLevel, xpForLevel,
  type Enemy, type EnemyKind, type EliteKind, type QuackState
} from "./quack";

function enemyAt(s: QuackState, kind: EnemyKind, x: number, y: number, hp = ENEMIES[kind].hp, elite: EliteKind | null = null): Enemy {
  const enemy: Enemy = { id: s.nextId++, kind, x, y, kx: 0, ky: 0, hp, maxHp: hp, flash: 0, orbitCd: 0, scale: 1, elite, auraCd: 0, spitCd: 1400 };
  s.enemies.push(enemy);
  return enemy;
}

/** Only `weapon` fires: the starting quill would otherwise steal the test's targets. */
function armed(s: QuackState, weapon: keyof QuackState["weapons"], level = 1) {
  for (const id of Object.keys(s.weapons) as (keyof QuackState["weapons"])[]) s.weapons[id] = id === weapon ? level : 0;
}

/** Plays `steps` frames, always taking the first upgrade offered. */
function play(s: QuackState, steps: number, input = { x: 1, y: 0.3 }) {
  for (let i = 0; i < steps && s.phase === "playing"; i++) {
    if (s.choices) applyUpgrade(s, s.choices[0]);
    step(s, input, STEP_MS);
  }
}

describe("quack simulation", () => {
  it("replays the same run from the same seed and inputs", () => {
    const a = createQuack(42);
    const b = createQuack(42);
    // Long enough for every join: mosquitos (25s), chaos (60s), elites (90s), splitters (45s).
    play(a, 6000);
    play(b, 6000);
    expect(a.kills).toBeGreaterThan(0);
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
    expect(JSON.stringify(createQuack(43))).not.toBe(JSON.stringify(createQuack(42)));
  });

  it("moves the duck at its speed however hard the input pushes", () => {
    const s = createQuack(1);
    step(s, { x: 5, y: 0 }, 1000);
    expect(s.player.x).toBeCloseTo(speedOf(s));
    expect(s.player.y).toBe(0);
    expect(s.player.facing).toEqual({ x: 1, y: 0 });
    step(s, { x: 0, y: 0 }, 500);
    expect(s.player.facing).toEqual({ x: 1, y: 0 });
  });

  it("hurts the duck on contact, briefly shields it, and ends the run at zero", () => {
    const s = createQuack(1);
    enemyAt(s, "bug", 4, 0);
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.player.hp).toBe(100 - ENEMIES.bug.damage);
    expect(s.player.invuln).toBeGreaterThan(0);
    step(s, { x: 0, y: 0 }, STEP_MS);
    // Shielded: only the slow regen moves it.
    expect(s.player.hp).toBeCloseTo(100 - ENEMIES.bug.damage, 1);

    s.player.invuln = 0;
    s.player.hp = 1;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.phase).toBe("lost");
    const time = s.time;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.time).toBe(time);
  });

  it("kills bugs with the starting quill and drops a token", () => {
    const s = createQuack(1);
    enemyAt(s, "bug", 80, 0, 1);
    for (let i = 0; i < 60 && s.kills === 0; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.kills).toBe(1);
    expect(s.tokens.length).toBeGreaterThanOrEqual(1);
  });

  it("levels up from tokens and holds the run while three distinct upgrades are offered", () => {
    const s = createQuack(7);
    gainXp(s, xpForLevel(1));
    expect(s.player.level).toBe(2);
    expect(s.choices).toHaveLength(3);
    expect(new Set(s.choices).size).toBe(3);
    const time = s.time;
    step(s, { x: 1, y: 0 }, STEP_MS);
    expect(s.time).toBe(time);

    const pick = s.choices![0];
    const before = upgradeLevel(s, pick);
    applyUpgrade(s, pick);
    expect(upgradeLevel(s, pick)).toBe(before + 1);
    expect(s.choices).toBeNull();
  });

  it("banks several level-ups and offers one choice after another", () => {
    const s = createQuack(7);
    gainXp(s, xpForLevel(1) + xpForLevel(2));
    expect(s.pendingLevels).toBe(2);
    applyUpgrade(s, s.choices![0]);
    expect(s.choices).toHaveLength(3);
    applyUpgrade(s, s.choices![0]);
    expect(s.choices).toBeNull();
  });

  it("never offers a maxed upgrade, falls back to a snack, and ignores picks it didn't offer", () => {
    const s = createQuack(3);
    s.weapons.orbit = MAX_LEVEL;
    for (let i = 0; i < 40; i++) expect(upgradeChoices(s)).not.toContain("orbit");
    // Everything maxed and every evolution either taken or unqualified: only the snack is left.
    s.weapons = { quill: MAX_LEVEL, orbit: MAX_LEVEL, comet: MAX_LEVEL, ping: MAX_LEVEL, flock: MAX_LEVEL, surge: MAX_LEVEL, ember: MAX_LEVEL };
    s.passives = { speed: MAX_LEVEL, magnet: MAX_LEVEL, vitality: MAX_LEVEL, haste: MAX_LEVEL, thorns: MAX_LEVEL, crop: MAX_LEVEL, shell: MAX_LEVEL };
    s.evolved = { quill: true, orbit: true, comet: true, ping: true, flock: true, surge: true, ember: true };
    expect(upgradeChoices(s)).toEqual(["snack"]);

    const fresh = createQuack(3);
    applyUpgrade(fresh, "comet");
    expect(fresh.weapons.comet).toBe(0);
  });

  it("wins at five minutes and doubles the score", () => {
    const s = createQuack(1);
    s.time = WIN_MS - STEP_MS;
    s.bosses = 2;
    s.kills = 10;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.phase).toBe("won");
    expect(score(s)).toBe((10 + 2 * 300) * 2);
  });

  it("sends a Segfault at two minutes and the Kernel Panic at the last one", () => {
    const s = createQuack(1);
    s.time = 120_000 - STEP_MS;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.enemies.some((e) => e.kind === "segfault")).toBe(true);
    expect(s.banner?.text).toMatch(/Segfault/);

    const t = createQuack(1);
    t.time = BOSS_TIMES[BOSS_TIMES.length - 1] - STEP_MS;
    t.bosses = 2;
    step(t, { x: 0, y: 0 }, STEP_MS);
    expect(t.bosses).toBe(3);
    expect(t.banner?.text).toMatch(/Kernel Panic/);
  });

  it("caps the swarm on a long run", () => {
    const s = createQuack(5);
    s.weapons.quill = 0;
    s.time = 270_000;
    s.bosses = 2;
    for (let i = 0; i < 1800; i++) {
      s.player.hp = 100;
      step(s, { x: 1, y: 0 }, STEP_MS);
      if (s.phase !== "playing") break;
    }
    expect(s.enemies.length).toBeLessThanOrEqual(MAX_ENEMIES);
    expect(s.enemies.length).toBeGreaterThan(100);
  });
});

describe("new weapons", () => {
  it("sends homing ducklings with Flock", () => {
    const s = createQuack(1);
    armed(s, "flock");
    enemyAt(s, "bug", 120, 0);
    for (let i = 0; i < 240 && s.kills === 0; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.kills).toBeGreaterThan(0);
  });

  it("arcs Surge from bug to bug in one chain", () => {
    const s = createQuack(1);
    armed(s, "surge");
    enemyAt(s, "bug", 60, 0);
    enemyAt(s, "bug", 140, 30); // within the 110px hop of the first
    for (let i = 0; i < 60 && s.kills < 2; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.kills).toBe(2);
  });

  it("burns whatever Ember's ring touches, one tick at a time", () => {
    const s = createQuack(1);
    armed(s, "ember");
    const enemy = enemyAt(s, "bug", 20, 0, 100);
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(enemy.hp).toBeLessThan(100);
    const afterFirst = enemy.hp;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(enemy.hp).toBe(afterFirst); // the per-enemy tick cooldown holds
  });
});

describe("new passives", () => {
  it("thorns hurt the bug that touches the duck", () => {
    const s = createQuack(1);
    s.passives.thorns = 1;
    const enemy = enemyAt(s, "bug", 4, 0, 100);
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.player.hp).toBeLessThan(100);
    expect(enemy.hp).toBeLessThan(100);
  });

  it("crop makes tokens worth more XP", () => {
    const s = createQuack(1);
    s.passives.crop = 1;
    s.tokens.push({ x: s.player.x, y: s.player.y, value: 4, flying: true });
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.player.xp).toBe(5); // 4 × 1.25, rounded
  });

  it("shell soaks damage from every hit", () => {
    const s = createQuack(1);
    s.passives.shell = 3;
    enemyAt(s, "bug", 4, 0);
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.player.hp).toBe(99); // 6 damage, soaked to 1
  });
});

describe("evolutions", () => {
  it("offers an evolution only when its weapon is maxed beside its passive", () => {
    const s = createQuack(9);
    s.weapons.quill = MAX_LEVEL;
    // Three of a large pool are drawn per roll, so a qualified evolution shows up across rolls.
    for (let i = 0; i < 60; i++) expect(upgradeChoices(s)).not.toContain("volley");
    s.passives.haste = 1;
    const offered = new Set<string>();
    for (let i = 0; i < 60; i++) for (const id of upgradeChoices(s)) offered.add(id);
    expect(offered).toContain("volley");
  });

  it("never offers the same evolution twice", () => {
    const s = createQuack(9);
    s.weapons.quill = MAX_LEVEL;
    s.passives.haste = 1;
    const offered = new Set<string>();
    for (let i = 0; i < 60; i++) for (const id of upgradeChoices(s)) offered.add(id);
    expect(offered).toContain("volley");
    s.evolved.quill = true;
    for (let i = 0; i < 60; i++) expect(upgradeChoices(s)).not.toContain("volley");
  });

  it("makes the evolved weapon strictly stronger", () => {
    const base = quillStats(MAX_LEVEL);
    const evolved = quillStats(MAX_LEVEL, true);
    expect(evolved.count).toBeGreaterThan(base.count);
    expect(evolved.cooldown).toBeLessThan(base.cooldown);
    expect(evolved.limit).toBeGreaterThan(base.limit);
    expect(flockStats(3, true).count).toBeGreaterThan(flockStats(3).count);
    expect(surgeStats(3, true).links).toBeGreaterThan(surgeStats(3).links);
  });

  it("applies an evolution when picked, and only when offered", () => {
    const s = createQuack(9);
    applyUpgrade(s, "volley");
    expect(s.evolved.quill).toBeUndefined();
    s.weapons.quill = MAX_LEVEL;
    s.passives.haste = 1;
    s.choices = ["volley"];
    applyUpgrade(s, "volley");
    expect(s.evolved.quill).toBe(true);
  });
});

describe("new enemies", () => {
  it("splits a splitter into two smaller children that do not split again", () => {
    const s = createQuack(1);
    enemyAt(s, "splitter", 80, 0, 1);
    for (let i = 0; i < 60 && s.kills === 0; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    const children = s.enemies.filter((e) => e.kind === "splitter" && e.scale < 0.9);
    expect(children).toHaveLength(2);

    children[0].hp = 0;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.enemies.filter((e) => e.kind === "splitter" && e.scale < 0.9)).toHaveLength(1);
  });

  it("keeps the mosquito at range and lets its spit hurt the duck", () => {
    const s = createQuack(1);
    enemyAt(s, "mosquito", 200, 0);
    let sawSpit = false;
    let closest = Infinity;
    for (let i = 0; i < 600; i++) {
      s.enemies = s.enemies.filter((e) => e.kind === "mosquito");
      const mosquito = s.enemies[0];
      if (mosquito) closest = Math.min(closest, Math.hypot(mosquito.x - s.player.x, mosquito.y - s.player.y));
      if (s.spits.length) sawSpit = true;
      step(s, { x: 0, y: 0 }, STEP_MS);
      if (s.player.hp < 100) break;
    }
    expect(sawSpit).toBe(true);
    expect(s.player.hp).toBeLessThan(100);
    expect(closest).toBeGreaterThan(100); // it never closes in to touch the duck
  });

  it("caps mosquito spit", () => {
    const s = createQuack(1);
    for (let i = 0; i < MAX_SPITS; i++) s.spits.push({ x: 5000 + i, y: 5000, vx: 0, vy: -150, damage: 9, life: 2800 });
    enemyAt(s, "mosquito", 200, 0);
    for (let i = 0; i < 600; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.spits.length).toBeLessThanOrEqual(MAX_SPITS);
  });
});

describe("elites", () => {
  it("armored bugs soak damage and die later than the same bug without it", () => {
    const s = createQuack(1);
    const plain = enemyAt(s, "bug", 40, 0, 20);
    const armored = enemyAt(s, "bug", 60, 20, 20, "armored");
    const deaths: number[] = [];
    for (let i = 0; i < 900 && deaths.length < 2; i++) {
      step(s, { x: 0, y: 0 }, STEP_MS);
      if (plain.hp <= 0 && !deaths.includes(plain.id)) deaths.push(plain.id);
      if (armored.hp <= 0 && !deaths.includes(armored.id)) deaths.push(armored.id);
    }
    expect(deaths).toEqual([plain.id, armored.id]);
  });

  it("explosive bugs burst when they die", () => {
    const s = createQuack(1);
    // Out of contact range (24px for a beetle), so the burst is the only thing that can hurt.
    const elite = enemyAt(s, "beetle", 60, 0, 1, "explosive");
    for (let i = 0; i < 600 && s.player.hp === 100; i++) {
      if (elite.hp <= 0) break;
      step(s, { x: 0, y: 0 }, STEP_MS);
    }
    expect(elite.hp).toBeLessThanOrEqual(0);
    expect(s.player.hp).toBeLessThan(60); // the 44-damage burst, contact never engaged
  });
});

describe("chaos events", () => {
  it("fires one on schedule, announced by the banner", () => {
    const s = createQuack(4);
    s.time = 59_000;
    s.nextChaos = 60_000;
    s.banner = null; // the opening banner, already expired
    for (let i = 0; i < 200 && !s.banner && s.phase === "playing"; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    const banner = s.banner as { text: string } | null;
    expect(banner?.text).toMatch(/Token shower|Bug rush|EMP/);
    expect(s.nextChaos).toBeGreaterThan(60_000);
  });

  it("the EMP damages everything on screen", () => {
    let emped: QuackState | null = null;
    for (let seed = 1; seed <= 60 && !emped; seed++) {
      const s = createQuack(seed);
      s.time = 59_000;
      s.nextChaos = 60_000;
      s.banner = null;
      enemyAt(s, "beetle", 100, 0, 500);
      enemyAt(s, "bug", -100, 60, 500);
      const before = s.enemies.map((e) => e.hp);
      for (let i = 0; i < 150 && !s.banner; i++) step(s, { x: 0, y: 0 }, STEP_MS);
      const banner = s.banner as { text: string } | null;
      // One seed in the range rolls the EMP; its 100+ damage swamps the quill's ~36 in the
      // same two seconds, so a big drop proves the pulse and not the weapon.
      if (banner?.text === "EMP!") {
        expect(s.enemies.some((e, index) => index < before.length && before[index] - e.hp > 80)).toBe(true);
        emped = s;
      }
    }
    expect(emped).not.toBeNull();
  });
});

describe("saves", () => {
  it("round-trips a played run exactly and continues it identically", () => {
    const a = createQuack(21);
    const b = createQuack(21);
    play(a, 900);
    play(b, 900);
    writeSave(a, true);
    const loaded = readSave();
    expect(loaded).not.toBeNull();
    expect(JSON.stringify(loaded)).toBe(JSON.stringify(a));
    play(a, 600);
    play(loaded!, 600);
    expect(JSON.stringify(loaded)).toBe(JSON.stringify(a));
    expect(JSON.stringify(loaded)).not.toBe(JSON.stringify(b));
  });
});
