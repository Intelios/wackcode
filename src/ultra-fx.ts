/**
 * Ultra's motion as pure functions: the pixel-wildcard mark's LED cycle (`UltraMark`), the
 * label's scramble (`UltraLabel`), and the reasoning slider's glyph field (`UltraGlyphField`).
 * The components only clock and paint these; the timings and shapes live here, where they
 * can be tested without a DOM.
 */

// ── The mark ──────────────────────────────────────────
// Cells are numbered row-major on the 3×3 grid. The ring walks the outer eight clockwise from
// the top-left; the centre (4) is the core that stays lit through the orbit.
export const ULTRA_RING = [0, 1, 2, 5, 8, 7, 6, 3] as const;
const PLUS = new Set([1, 3, 4, 5, 7]);
const CROSS = new Set([0, 2, 4, 6, 8]);

/** One LED step. The mark's ticker runs at this rate, so every frame lands on a step. */
export const ULTRA_STEP_MS = 110;
const ORBIT_MS = ULTRA_STEP_MS * ULTRA_RING.length * 2;
const PLUS_MS = 360;
const CROSS_MS = 360;
const HOLD_MS = 2400;
/** Two laps of the orbit, a `+`, an `×`, then the settled `✱` held long enough to read. */
export const ULTRA_CYCLE_MS = ORBIT_MS + PLUS_MS + CROSS_MS + HOLD_MS;

const DIM = 0.14;
/** The still wildcard: a bright `+` over dimmer corners. Static marks and reduced motion show it. */
export const ULTRA_SETTLED: readonly number[] = Array.from({ length: 9 }, (_, cell) => (cell === 4 ? 1 : PLUS.has(cell) ? 0.95 : 0.55));
// The orbit's head and the two beads trailing it, like `OrbitSpinner`'s comet broken into pixels.
const TRAIL = [1, 0.6, 0.3];

/** Cell opacities for a mark `elapsed` ms after it started (its mount is the entrance). */
export function ultraMarkFrame(elapsed: number): number[] {
  const t = ((elapsed % ULTRA_CYCLE_MS) + ULTRA_CYCLE_MS) % ULTRA_CYCLE_MS;
  if (t < ORBIT_MS) {
    const head = Math.floor(t / ULTRA_STEP_MS) % ULTRA_RING.length;
    return Array.from({ length: 9 }, (_, cell) => {
      if (cell === 4) return 0.9;
      const behind = (head - ULTRA_RING.indexOf(cell as (typeof ULTRA_RING)[number]) + ULTRA_RING.length) % ULTRA_RING.length;
      return TRAIL[behind] ?? DIM;
    });
  }
  if (t < ORBIT_MS + PLUS_MS) return Array.from({ length: 9 }, (_, cell) => (PLUS.has(cell) ? 1 : DIM));
  if (t < ORBIT_MS + PLUS_MS + CROSS_MS) return Array.from({ length: 9 }, (_, cell) => (CROSS.has(cell) ? 1 : DIM));
  return [...ULTRA_SETTLED];
}

// ── The label ─────────────────────────────────────────
export const ULTRA_NOISE = "*+x#%@&?<>/=";
export const ULTRA_DECODE_MS = 560;

/**
 * The label `k` (0–1) of the way through decoding: each letter resolves in turn from the left,
 * the rest show noise. Spaces never scramble, so the words keep their shape.
 */
export function scrambleText(text: string, k: number, random: () => number = Math.random): string {
  const letters = [...text];
  return letters
    .map((letter, index) => (letter === " " || k > (index / letters.length) * 0.7 + 0.22 ? letter : ULTRA_NOISE[Math.floor(random() * ULTRA_NOISE.length) % ULTRA_NOISE.length]))
    .join("");
}

// ── The glyph field ───────────────────────────────────
/** Sparse to dense. The space lets the field thin out at the low end. */
export const ULTRA_GLYPHS = " .:-=+*#%@";
/** What floods the fill while it boots, before each column resolves. */
export const ULTRA_FLOOD = "@#%&";

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

export function glyphFor(density: number): string {
  return ULTRA_GLYPHS[Math.round(clamp01(density) * (ULTRA_GLYPHS.length - 1))];
}

/**
 * How dense a cell is: sparse at the track's start and packed toward the thumb (`x`, 0–1), shaken
 * by the cell's own `noise` (0–1) and lifted by the scan head passing over it (`scan`, 0–1).
 */
export function fieldDensity(x: number, noise: number, scan: number): number {
  return clamp01(0.1 + 0.62 * x + (noise - 0.5) * 0.35 + scan * 0.6);
}

/** Where the scan head is, in columns: it sweeps past the last one, then rests off the track. */
export function scanHead(elapsed: number, columns: number, sweepMs = 1700): number {
  return ((Math.max(0, elapsed) / sweepMs) % 1.25) * columns;
}

/** How close a column is to the head (0–1): a soft bump a few columns wide. */
export function scanCloseness(column: number, head: number): number {
  return Math.exp(-((column - head) ** 2) / 5);
}

export type Rgb = [number, number, number];

export function mixRgb(from: Rgb, to: Rgb, amount: number): Rgb {
  return from.map((channel, index) => channel + (to[index] - channel) * amount) as Rgb;
}

/**
 * A colour along the spectrum. `t` runs 0→1 across the stops and keeps going, bouncing back at
 * each end, so a drifting offset never jumps from the last stop to the first.
 */
export function spectrumAt(stops: readonly Rgb[], t: number): Rgb {
  if (stops.length === 0) return [255, 255, 255];
  if (stops.length === 1) return stops[0];
  const wrapped = ((t % 2) + 2) % 2;
  const along = (wrapped <= 1 ? wrapped : 2 - wrapped) * (stops.length - 1);
  const index = Math.min(Math.floor(along), stops.length - 2);
  return mixRgb(stops[index], stops[index + 1], along - index);
}
