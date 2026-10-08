import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SAVE_KEY, clearSave, hasSave, readSave, writeSave } from "./save";
import { createQuack } from "./quack";

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

/** A run with every list and field exercised, built in the state's own key order. */
function richRun() {
  const s = createQuack(7);
  s.enemies.push({ id: 1, kind: "beetle", x: 30, y: 0, kx: 1, ky: -1, hp: 20, maxHp: 60, flash: 10, orbitCd: 0, scale: 1, elite: "armored", auraCd: 5, spitCd: 900 });
  s.bolts.push({ kind: "comet", x: 1, y: 1, vx: 5, vy: 5, damage: 40, life: 500, hits: [3], limit: 999 });
  s.tokens.push({ x: 3, y: 4, value: 2, flying: true });
  s.rings.push({ x: 1, y: 2, radius: 80, life: 300, max: 420 });
  s.particles.push({ x: 1, y: 1, vx: 2, vy: 3, life: 100, max: 260, size: 2, tone: "danger" });
  s.spits.push({ x: 5, y: 5, vx: 10, vy: -10, damage: 9, life: 1200 });
  s.arcs.push({ points: [0, 0, 10, 10, 20, 5], life: 100, max: 220 });
  s.weapons.comet = 2;
  s.evolved.quill = true;
  s.time = 45_000;
  s.kills = 12;
  s.spawnDebt = 0.5;
  s.bosses = 1;
  s.player.invuln = 100;
  s.pendingLevels = 1;
  s.choices = ["orbit", "snack"];
  s.banner = { text: "Bug rush!", until: 46_000 };
  s.shake = 120;
  s.nextChaos = 110_000;
  return s;
}

describe("saved runs", () => {
  it("round-trips a rich run exactly", () => {
    const s = richRun();
    writeSave(s, true);
    expect(JSON.stringify(readSave())).toBe(JSON.stringify(s));
    expect(hasSave()).toBe(true);
  });

  it("reads anything unreadable as no save", () => {
    expect(readSave()).toBeNull();
    localStorage.setItem(SAVE_KEY, "{not json");
    expect(readSave()).toBeNull();
    localStorage.setItem(SAVE_KEY, JSON.stringify([1, 2, 3]));
    expect(readSave()).toBeNull();
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 2, state: {} }));
    expect(readSave()).toBeNull();
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1 }));
    expect(readSave()).toBeNull();
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1, state: { phase: "lost" } }));
    expect(readSave()).toBeNull();
    expect(hasSave()).toBe(false);
  });

  it("fills in the fields a save from an older build would lack", () => {
    const legacy = JSON.parse(JSON.stringify(createQuack(3)));
    delete legacy.evolved;
    delete legacy.spits;
    delete legacy.arcs;
    delete legacy.nextChaos;
    legacy.enemies.push({ id: 99, kind: "bug", x: 5, y: 5, kx: 0, ky: 0, hp: 14, maxHp: 14, flash: 0, orbitCd: 0 });
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1, state: legacy }));

    const loaded = readSave();
    expect(loaded).not.toBeNull();
    expect(loaded!.evolved).toEqual({});
    expect(loaded!.spits).toEqual([]);
    expect(loaded!.arcs).toEqual([]);
    expect(loaded!.nextChaos).toBeGreaterThan(loaded!.time);
    expect(loaded!.enemies.find((e) => e.id === 99)).toMatchObject({ scale: 1, elite: null, auraCd: 0, spitCd: 1400 });
  });

  it("clamps nonsense values instead of trusting them", () => {
    const s = createQuack(3);
    const junk = JSON.parse(JSON.stringify(s));
    junk.player.hp = -50;
    junk.player.level = 0;
    junk.rng = "seventeen";
    junk.enemies.push({ id: 5, kind: "werewolf", x: "near", y: null, kx: 0, ky: 0, hp: 999, maxHp: 5, flash: -2, orbitCd: 0, scale: 99, elite: "god", auraCd: -1, spitCd: "soon" });
    junk.choices = ["orbit", "not-an-upgrade"];
    junk.bosses = 9;
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1, state: junk }));

    const loaded = readSave()!;
    expect(loaded.player.hp).toBeGreaterThanOrEqual(1);
    expect(loaded.player.level).toBe(1);
    expect(Number.isFinite(loaded.rng)).toBe(true);
    expect(loaded.bosses).toBe(3);
    expect(loaded.choices).toEqual(["orbit"]);
    const enemy = loaded.enemies.find((e) => e.id === 5)!;
    expect(enemy.kind).toBe("bug");
    expect(enemy.elite).toBeNull();
    expect(enemy.scale).toBeLessThanOrEqual(3);
    expect(Number.isFinite(enemy.x)).toBe(true);
  });

  it("throttles writes to one per couple of seconds unless forced", () => {
    vi.useFakeTimers();
    try {
      const s = createQuack(1);
      writeSave(s, true);
      const spy = vi.spyOn(Storage.prototype, "setItem");
      writeSave(s);
      writeSave(s);
      expect(spy).not.toHaveBeenCalled();
      vi.advanceTimersByTime(2100);
      writeSave(s);
      expect(spy).toHaveBeenCalledOnce();
    } finally {
      vi.useRealTimers();
    }
  });

  it("stays silent when storage fails", () => {
    const set = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("quota"); });
    expect(() => writeSave(createQuack(1), true)).not.toThrow();
    set.mockRestore();

    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
    expect(readSave()).toBeNull();
    expect(() => clearSave()).not.toThrow();
  });

  it("clears the save", () => {
    writeSave(createQuack(1), true);
    expect(hasSave()).toBe(true);
    clearSave();
    expect(hasSave()).toBe(false);
  });
});
