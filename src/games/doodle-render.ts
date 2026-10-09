/**
 * Draws a Doodle Duck run (`doodle.ts`) onto a 2D canvas: the world's backdrop (the dot grid,
 * with cloud puffs in the sky and starlight above it), the platforms as lily pads, cloud ledges
 * and starlit pads, the bubble power-ups, the bugs, the duck, effects, then the HUD in screen
 * space. The camera keeps the bottom of the view at `state.cameraY`; world y grows upward, so
 * the renderer flips it. Pure drawing: it never mutates the state.
 *
 * The cast stays WackCode's: the duck is the `DuckMark` duck, bugs are danger red — the one
 * colour that means "this hurts" — and springs and bubbles wear the accent, which means "special
 * and good". Zones are drawn from world altitude alone, so the pond gives way to sky and then
 * stars as the duck climbs, and the two blend across a few hundred pixels.
 *
 * Reduced motion keeps the game playable but still: no bobbing, twinkle, crumble jitter or
 * particle bursts.
 */
import { DUCK_PATH } from "../gravity-well";
import type { GamePalette } from "./palette";
import {
  BANNER_MS, BUG_R, DUCK_R, ITEM_R, PLATFORM_H, SKY_Y, STARS_Y, score,
  type Bug, type DoodleState, type Platform, type Tone
} from "./doodle";

export interface RenderView { w: number; h: number; reduced: boolean }

const FONT = '-apple-system, "Inter", system-ui, sans-serif';
const MONO = '"SFMono-Regular", Consolas, monospace';
const GRID = 30;

let duckPath: Path2D | undefined;
function duck(): Path2D | undefined {
  if (typeof Path2D === "undefined") return undefined;
  return duckPath ?? (duckPath = new Path2D(DUCK_PATH));
}

export function formatClock(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function toneColor(palette: GamePalette, tone: Tone): string {
  return tone === "accent" ? palette.accent : tone === "danger" ? palette.danger : palette.textSoft;
}

/** A stable pseudo-random 0..1 from an integer, for backdrop glyphs that need no state. */
function hash(n: number): number {
  const x = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
}

const smooth = (t: number) => Math.max(0, Math.min(1, t));

export function render(ctx: CanvasRenderingContext2D, s: DoodleState | null, palette: GamePalette, view: RenderView): void {
  const { w, h, reduced } = view;
  ctx.globalAlpha = 1;
  ctx.fillStyle = palette.well;
  ctx.fillRect(0, 0, w, h);
  if (!s) return;

  // World y grows upward; the camera's bottom edge sits at cameraY.
  const toY = (wy: number) => h - (wy - s.cameraY);
  const pondWeight = smooth(1 - (s.cameraY + h / 2 - SKY_Y) / 500);
  const starsWeight = smooth((s.cameraY + h / 2 - STARS_Y) / 500);

  drawBackdrop(ctx, s, palette, view, toY, pondWeight, starsWeight);

  for (const p of s.platforms) drawPlatform(ctx, p, s, palette, view, toY, reduced);
  for (const item of s.items) drawItem(ctx, item.x, toY(item.y), s, palette, reduced);
  for (const bug of s.bugs) drawBug(ctx, bug, s, palette, toY, reduced);

  drawDuck(ctx, s, palette, view, toY, reduced);

  if (!reduced) {
    for (const p of s.particles) {
      const y = toY(p.y);
      if (y < -10 || y > h + 10) continue;
      ctx.globalAlpha = Math.max(0, p.life / p.max);
      ctx.fillStyle = toneColor(palette, p.tone);
      ctx.fillRect(p.x - p.size / 2, y - p.size / 2, p.size, p.size);
    }
    ctx.globalAlpha = 1;
  }

  hud(ctx, s, palette, w);
  if (s.banner) {
    ctx.globalAlpha = Math.max(0, Math.min(1, (s.banner.until - s.time) / 500, (s.time - (s.banner.until - BANNER_MS)) / 250));
    ctx.fillStyle = palette.text;
    ctx.font = `700 15px ${FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(s.banner.text, w / 2, h * 0.3);
    ctx.globalAlpha = 1;
  }
}

/** The scrolling backdrop: the dot grid everywhere, cloud puffs in the sky, stars up high. */
function drawBackdrop(
  ctx: CanvasRenderingContext2D, s: DoodleState, palette: GamePalette, view: RenderView,
  toY: (wy: number) => number, pond: number, stars: number
) {
  const { w, h } = view;

  // A dot grid everywhere, so the climb always reads as movement.
  ctx.fillStyle = palette.border;
  const firstY = Math.floor(s.cameraY / GRID) * GRID;
  for (let wy = firstY; wy < s.cameraY + h + GRID; wy += GRID) {
    const y = toY(wy);
    for (let x = GRID / 2; x < w; x += GRID) ctx.fillRect(x - 0.75, y - 0.75, 1.5, 1.5);
  }

  // Clouds live in the sky band; stars live above it. Both fade in across their boundary.
  const sky = Math.max(0, 1 - pond - stars);
  const band = 240;
  const firstBand = Math.floor((s.cameraY - 60) / band);
  for (let i = firstBand; i * band < s.cameraY + view.h + 60; i++) {
    const wy = i * band + hash(i * 5 + 2) * band;
    const y = toY(wy);
    if (y < -50 || y > view.h + 50) continue;
    if (sky > 0.01) {
      ctx.globalAlpha = 0.14 * sky;
      ctx.fillStyle = palette.textSoft;
      const cx = hash(i * 11 + 3) * w;
      const r = 16 + hash(i * 13 + 4) * 14;
      ctx.beginPath();
      ctx.ellipse(cx, y, r * 1.7, r * 0.62, 0, 0, Math.PI * 2);
      ctx.ellipse(cx - r * 0.8, y + r * 0.18, r * 0.8, r * 0.45, 0, 0, Math.PI * 2);
      ctx.ellipse(cx + r * 0.9, y + r * 0.12, r * 0.7, r * 0.4, 0, 0, Math.PI * 2);
      ctx.fill();
    }
    if (stars > 0.01) {
      const twinkle = view.reduced ? 1 : 0.5 + 0.5 * Math.sin(s.time * 0.004 + i * 2.1);
      ctx.globalAlpha = (0.35 + 0.65 * twinkle) * stars;
      ctx.fillStyle = palette.text;
      for (let k = 0; k < 3; k++) {
        const sx = hash(i * 29 + k * 31 + 5) * w;
        const r = 0.7 + hash(i + k * 3 + 6) * 1.1;
        ctx.beginPath();
        ctx.arc(sx, y + hash(i * 7 + k) * band * 0.5, r, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }
  ctx.globalAlpha = 1;
}

function drawPlatform(
  ctx: CanvasRenderingContext2D, p: Platform, s: DoodleState, palette: GamePalette,
  view: RenderView, toY: (wy: number) => number, reduced: boolean
) {
  const y = toY(p.y);
  if (y < -30 || y > view.h + 30) return;
  const crumbling = p.crumble >= 0;
  const jitter = crumbling && !reduced ? Math.sin(s.time * 0.09 + p.id) * 1.8 : 0;
  const cx = p.x + p.w / 2 + jitter;
  const zone = s.cameraY + view.h / 2 < SKY_Y ? "pond" : s.cameraY + view.h / 2 < STARS_Y ? "sky" : "stars";

  ctx.save();
  ctx.translate(cx, y);
  if (zone === "sky") {
    // A cloud ledge: three soft puffs with the pad's top as their floor.
    ctx.fillStyle = crumbling ? palette.surface : palette.textSoft;
    ctx.globalAlpha = crumbling ? 0.55 : 0.9;
    ctx.beginPath();
    ctx.ellipse(0, -PLATFORM_H * 0.5, p.w * 0.52, PLATFORM_H * 0.95, 0, 0, Math.PI * 2);
    ctx.ellipse(-p.w * 0.3, -PLATFORM_H * 0.7, p.w * 0.28, PLATFORM_H * 0.7, 0, 0, Math.PI * 2);
    ctx.ellipse(p.w * 0.31, -PLATFORM_H * 0.65, p.w * 0.26, PLATFORM_H * 0.65, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
  } else if (zone === "stars") {
    // A starlit ledge: a lit slab with an accent rim.
    ctx.fillStyle = palette.surface;
    ctx.strokeStyle = palette.accent;
    ctx.lineWidth = 1.5;
    ctx.globalAlpha = 0.9;
    ctx.beginPath();
    ctx.roundRect(-p.w / 2, -PLATFORM_H, p.w, PLATFORM_H, 4);
    ctx.fill();
    ctx.stroke();
    ctx.globalAlpha = 0.16;
    ctx.beginPath();
    ctx.roundRect(-p.w / 2 - 3, -PLATFORM_H - 3, p.w + 6, PLATFORM_H + 6, 6);
    ctx.fill();
    ctx.globalAlpha = 1;
  } else {
    // A lily pad: an ellipse with a notch cut toward its right, floating on the pond.
    ctx.fillStyle = palette.surface;
    ctx.strokeStyle = crumbling ? palette.textDim : palette.border;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.ellipse(0, -PLATFORM_H * 0.55, p.w / 2, PLATFORM_H * 0.85, 0, 0, Math.PI * 2);
    ctx.fill();
    if (crumbling) ctx.setLineDash([4, 3]);
    ctx.stroke();
    ctx.setLineDash([]);
    // The notch: a wedge of background colour, the pad's one recognisable feature.
    ctx.fillStyle = palette.well;
    ctx.beginPath();
    ctx.moveTo(p.w * 0.14, -PLATFORM_H * 0.55);
    ctx.lineTo(p.w / 2, -PLATFORM_H * 0.55 - 1.5);
    ctx.lineTo(p.w / 2, -PLATFORM_H * 0.55 + 1.5);
    ctx.closePath();
    ctx.fill();
  }
  ctx.restore();

  if (p.kind === "moving") {
    // Side chevrons mark the pad that will slide.
    ctx.strokeStyle = palette.textDim;
    ctx.lineWidth = 1.6;
    ctx.lineCap = "round";
    const gy = y - PLATFORM_H / 2;
    for (const side of [-1, 1]) {
      const x = cx + side * (p.w / 2 + 6);
      ctx.beginPath();
      ctx.moveTo(x - side * 2.5, gy - 3);
      ctx.lineTo(x + side * 2.5, gy);
      ctx.lineTo(x - side * 2.5, gy + 3);
      ctx.stroke();
    }
  }

  if (p.spring) {
    // The spring: two accent coils under a plate, waiting to throw.
    const sx = p.x + p.w / 2 + jitter;
    ctx.strokeStyle = palette.accent;
    ctx.lineWidth = 2;
    ctx.lineCap = "round";
    const top = y - PLATFORM_H - 12;
    ctx.beginPath();
    ctx.moveTo(sx - 6, y - PLATFORM_H);
    ctx.lineTo(sx + 3, y - PLATFORM_H - 4);
    ctx.lineTo(sx - 3, y - PLATFORM_H - 8);
    ctx.lineTo(sx + 3, top);
    ctx.stroke();
    ctx.strokeStyle = palette.accent;
    ctx.lineWidth = 2.2;
    ctx.beginPath();
    ctx.moveTo(sx - 5, top);
    ctx.lineTo(sx + 5, top);
    ctx.stroke();
  }
}

/** A floating power-up bubble: a clear accent sphere with a highlight, drifting in place. */
function drawItem(ctx: CanvasRenderingContext2D, x: number, y: number, s: DoodleState, palette: GamePalette, reduced: boolean) {
  const bob = reduced ? 0 : Math.sin(s.time * 0.003 + x * 0.05) * 3;
  const cy = y + bob;
  ctx.globalAlpha = 0.16;
  ctx.fillStyle = palette.accent;
  ctx.beginPath();
  ctx.arc(x, cy, ITEM_R + 5, 0, Math.PI * 2);
  ctx.fill();
  ctx.globalAlpha = 0.3;
  ctx.strokeStyle = palette.accent;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.arc(x, cy, ITEM_R, 0, Math.PI * 2);
  ctx.stroke();
  ctx.globalAlpha = 0.8;
  ctx.beginPath();
  ctx.arc(x - ITEM_R * 0.35, cy - ITEM_R * 0.35, ITEM_R * 0.28, -2.6, -0.9);
  ctx.stroke();
  ctx.globalAlpha = 1;
}

/** A patrolling bug, in the same dress as Quack Survivors': danger-red eyes on a dark shell. */
function drawBug(ctx: CanvasRenderingContext2D, bug: Bug, s: DoodleState, palette: GamePalette, toY: (wy: number) => number, reduced: boolean) {
  const y = toY(bug.y);
  const r = BUG_R;
  const facing = bug.vx < 0 ? -1 : 1;
  const wiggle = reduced ? 0 : Math.sin(s.time * 0.02 + bug.id * 1.7) * r * 0.2;
  ctx.save();
  ctx.translate(bug.x, y);
  ctx.scale(facing, 1);

  ctx.strokeStyle = palette.textDim;
  ctx.lineWidth = 1.4;
  ctx.lineCap = "round";
  ctx.beginPath();
  for (const i of [-1, 0, 1]) {
    const lx = i * r * 0.45;
    ctx.moveTo(lx, -r * 0.5);
    ctx.lineTo(lx + wiggle, -r * 1.1);
    ctx.moveTo(lx, r * 0.5);
    ctx.lineTo(lx - wiggle, r * 1.1);
  }
  ctx.stroke();

  ctx.fillStyle = palette.surface;
  ctx.strokeStyle = palette.danger;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.ellipse(0, 0, r * 1.02, r * 0.8, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(-r, 0);
  ctx.lineTo(r * 0.4, 0);
  ctx.stroke();

  ctx.fillStyle = palette.textDim;
  ctx.beginPath();
  ctx.arc(r * 0.85, 0, r * 0.42, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = palette.danger;
  const eye = Math.max(1.4, r * 0.18);
  ctx.beginPath();
  ctx.arc(r * 0.98, -r * 0.16, eye, 0, Math.PI * 2);
  ctx.arc(r * 0.98, r * 0.16, eye, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function drawDuck(
  ctx: CanvasRenderingContext2D, s: DoodleState, palette: GamePalette,
  view: RenderView, toY: (wy: number) => number, reduced: boolean
) {
  const d = s.duck;
  const y = toY(d.y);
  if (y < -60 || y > view.h + 60) return;
  const path = duck();
  const tilt = reduced ? 0 : Math.max(-0.3, Math.min(0.38, -d.vy / 2400));

  // Near a side, the wrapped copy draws too, so the duck never half-disappears.
  const copies = d.x < 48 ? [d.x, d.x + view.w] : d.x > view.w - 48 ? [d.x, d.x - view.w] : [d.x];
  for (const x of copies) {
    if (d.bubble > 0) {
      ctx.globalAlpha = 0.18;
      ctx.fillStyle = palette.accent;
      ctx.beginPath();
      ctx.arc(x, y, DUCK_R + 9, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 0.5;
      ctx.strokeStyle = palette.accent;
      ctx.lineWidth = 1.6;
      ctx.beginPath();
      ctx.arc(x, y, DUCK_R + 9, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(x - 5, y - 5, 4, -2.6, -0.9);
      ctx.stroke();
      ctx.globalAlpha = 1;
    } else {
      ctx.fillStyle = palette.accent;
      ctx.globalAlpha = 0.12;
      ctx.beginPath();
      ctx.arc(x, y + DUCK_R * 0.7, DUCK_R, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
    }
    if (path) {
      const k = 30 / 256;
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(tilt * d.facing);
      ctx.scale(d.facing < 0 ? -k : k, k);
      ctx.translate(-134, -129);
      ctx.fill(path, "evenodd");
      ctx.restore();
    } else {
      ctx.fillStyle = palette.accent;
      ctx.beginPath();
      ctx.arc(x, y, DUCK_R, 0, Math.PI * 2);
      ctx.fill();
    }
  }
}

function hud(ctx: CanvasRenderingContext2D, s: DoodleState, palette: GamePalette, w: number) {
  ctx.font = `650 13px ${MONO}`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = palette.text;
  ctx.fillText(`${score(s)} m`, w / 2, 18.5);
}
