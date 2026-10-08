import { describe, expect, it } from "vitest";
import {
  ENEMIES, MAX_ENEMIES, MAX_LEVEL, STEP_MS, WIN_MS, applyUpgrade, createSwarm, gainXp, score, speedOf, step, upgradeChoices,
  upgradeLevel, xpForLevel, type EnemyKind, type SwarmState
} from "./swarm";

function enemyAt(s: SwarmState, kind: EnemyKind, x: number, y: number, hp = ENEMIES[kind].hp) {
  s.enemies.push({ id: s.nextId++, kind, x, y, kx: 0, ky: 0, hp, maxHp: hp, flash: 0, orbitCd: 0 });
}

/** Plays `steps` frames, always taking the first upgrade offered. */
function play(s: SwarmState, steps: number, input = { x: 1, y: 0.3 }) {
  for (let i = 0; i < steps && s.phase === "playing"; i++) {
    if (s.choices) applyUpgrade(s, s.choices[0]);
    step(s, input, STEP_MS);
  }
}

describe("swarm simulation", () => {
  it("replays the same run from the same seed and inputs", () => {
    const a = createSwarm(42);
    const b = createSwarm(42);
    play(a, 3600);
    play(b, 3600);
    expect(a.kills).toBeGreaterThan(0);
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
    expect(JSON.stringify(createSwarm(43))).not.toBe(JSON.stringify(createSwarm(42)));
  });

  it("moves the duck at its speed however hard the input pushes", () => {
    const s = createSwarm(1);
    step(s, { x: 5, y: 0 }, 1000);
    expect(s.player.x).toBeCloseTo(speedOf(s));
    expect(s.player.y).toBe(0);
    expect(s.player.facing).toEqual({ x: 1, y: 0 });
    step(s, { x: 0, y: 0 }, 500);
    expect(s.player.facing).toEqual({ x: 1, y: 0 });
  });

  it("hurts the duck on contact, briefly shields it, and ends the run at zero", () => {
    const s = createSwarm(1);
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
    const s = createSwarm(1);
    enemyAt(s, "bug", 80, 0, 1);
    for (let i = 0; i < 60 && s.kills === 0; i++) step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.kills).toBe(1);
    expect(s.tokens.length).toBeGreaterThanOrEqual(1);
  });

  it("levels up from tokens and holds the run while three distinct upgrades are offered", () => {
    const s = createSwarm(7);
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
    const s = createSwarm(7);
    gainXp(s, xpForLevel(1) + xpForLevel(2));
    expect(s.pendingLevels).toBe(2);
    applyUpgrade(s, s.choices![0]);
    expect(s.choices).toHaveLength(3);
    applyUpgrade(s, s.choices![0]);
    expect(s.choices).toBeNull();
  });

  it("never offers a maxed upgrade, falls back to a snack, and ignores picks it didn't offer", () => {
    const s = createSwarm(3);
    s.weapons.orbit = MAX_LEVEL;
    for (let i = 0; i < 40; i++) expect(upgradeChoices(s)).not.toContain("orbit");
    s.weapons = { quill: MAX_LEVEL, orbit: MAX_LEVEL, comet: MAX_LEVEL, ping: MAX_LEVEL };
    s.passives = { speed: MAX_LEVEL, magnet: MAX_LEVEL, vitality: MAX_LEVEL, haste: MAX_LEVEL };
    expect(upgradeChoices(s)).toEqual(["snack"]);

    const fresh = createSwarm(3);
    applyUpgrade(fresh, "comet");
    expect(fresh.weapons.comet).toBe(0);
  });

  it("wins at five minutes and doubles the score", () => {
    const s = createSwarm(1);
    s.time = WIN_MS - STEP_MS;
    s.bosses = 2;
    s.kills = 10;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.phase).toBe("won");
    expect(score(s)).toBe((10 + 2 * 300) * 2);
  });

  it("sends a Segfault at two minutes", () => {
    const s = createSwarm(1);
    s.time = 120_000 - STEP_MS;
    step(s, { x: 0, y: 0 }, STEP_MS);
    expect(s.enemies.some((e) => e.kind === "segfault")).toBe(true);
    expect(s.banner?.text).toMatch(/Segfault/);
  });

  it("caps the swarm on a long run", () => {
    const s = createSwarm(5);
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
