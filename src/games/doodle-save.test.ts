import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SAVE_KEY, clearSave, hasSave, readSave, writeSave } from "./doodle-save";
import { SKY_Y, createDoodle } from "./doodle";

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

/** A run with every list and field exercised, built in the state's own key order. */
function richRun() {
  const s = createDoodle(7, { w: 420, h: 640 });
  s.platforms.push({ id: 1, kind: "moving", x: 30, y: 900, w: 64, vx: -55, spring: false, crumble: -1 });
  s.platforms.push({ id: 2, kind: "crumbling", x: 130, y: 960, w: 58, vx: 0, spring: false, crumble: 120 });
  s.bugs.push({ id: 3, x: 80, y: 930, minX: 40, maxX: 120, vx: 45 });
  s.items.push({ x: 100, y: 1000 });
  s.particles.push({ x: 1, y: 1, vx: 2, vy: 3, life: 100, max: 260, size: 2, tone: "danger" });
  s.duck.vy = 620;
  s.duck.facing = -1;
  s.duck.bubble = 900;
  s.cameraY = 500;
  s.peak = 1100;
  s.genY = 1200;
  s.genX = 210;
  s.squash = 4;
  s.zone = 1;
  s.time = 45_000;
  s.banner = { text: "The sky opens up", until: 46_000 };
  return s;
}

describe("saved doodle runs", () => {
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
    const legacy = JSON.parse(JSON.stringify(createDoodle(3)));
    delete legacy.zone;
    delete legacy.squash;
    delete legacy.items;
    legacy.platforms.push({ id: 99, kind: "static", x: 5, y: 5, w: 64 });
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1, state: legacy }));

    const loaded = readSave();
    expect(loaded).not.toBeNull();
    expect(loaded!.zone).toBe(0);
    expect(loaded!.squash).toBe(0);
    expect(loaded!.items).toEqual([]);
    expect(loaded!.platforms.find((p) => p.id === 99)).toMatchObject({ vx: 0, spring: false, crumble: -1 });
  });

  it("clamps nonsense values instead of trusting them", () => {
    const s = createDoodle(3);
    const junk = JSON.parse(JSON.stringify(s));
    junk.duck.hp = 50; // A field this game has never had.
    junk.duck.facing = 0;
    junk.duck.bubble = -400;
    junk.rng = "seventeen";
    junk.zone = 9;
    junk.platforms.push({ id: 5, kind: "werewolf", x: "near", y: null, w: 5000, spring: "yes", crumble: -9, vx: 12 });
    junk.bugs.push({ id: 6, x: 10, y: 10, minX: 90, maxX: 20, vx: "fast" });
    localStorage.setItem(SAVE_KEY, JSON.stringify({ version: 1, state: junk }));

    const loaded = readSave()!;
    expect(Number.isFinite(loaded.rng)).toBe(true);
    expect(loaded.duck.facing).toBe(1);
    expect(loaded.duck.bubble).toBe(0);
    expect(loaded.zone).toBe(0);
    const platform = loaded.platforms.find((p) => p.id === 5)!;
    expect(platform.kind).toBe("static");
    expect(platform.w).toBeLessThanOrEqual(400);
    expect(platform.spring).toBe(false);
    expect(platform.crumble).toBe(-1);
    expect(platform.vx).toBe(0);
    expect(Number.isFinite(platform.x)).toBe(true);
    expect(loaded.bugs.find((b) => b.id === 6)).toMatchObject({ vx: 40 });
  });

  it("keeps a zone the run already earned", () => {
    const s = createDoodle(3);
    s.duck.y = SKY_Y + 1000;
    s.zone = 1;
    writeSave(s, true);
    expect(readSave()!.zone).toBe(1);
  });

  it("throttles writes to one per couple of seconds unless forced", () => {
    vi.useFakeTimers();
    try {
      const s = createDoodle(1);
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
    expect(() => writeSave(createDoodle(1), true)).not.toThrow();
    set.mockRestore();

    const get = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("blocked"); });
    expect(readSave()).toBeNull();
    expect(() => clearSave()).not.toThrow();
  });

  it("clears the save", () => {
    writeSave(createDoodle(1), true);
    expect(hasSave()).toBe(true);
    clearSave();
    expect(hasSave()).toBe(false);
  });
});
