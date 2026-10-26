/** A bounded glyph field, painted from one cached atlas. Masks are sampled only on resize;
 * no prompts, generated images, network assets, per-glyph DOM nodes or per-frame text layout. */
import { glyphGrid } from "../dynamic-background";

export interface Ripple { x: number; y: number; age: number }
const GLYPHS = ".:;+*ox#%@0123456789abcdef";
const ATLAS_W = 12;
const ATLAS_H = 18;
const ellipse = (x: number, y: number, cx: number, cy: number, rx: number, ry: number) =>
  Math.exp(-2 * (((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2));

export function createGlyphField(canvas: HTMLCanvasElement, colours: string[]) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return undefined;
  const atlas = document.createElement("canvas");
  atlas.width = GLYPHS.length * ATLAS_W * 2;
  atlas.height = ATLAS_H * 3 * 2;
  const ink = atlas.getContext("2d");
  if (!ink) return undefined;
  ink.scale(2, 2);
  ink.font = '12px "SFMono-Regular", Consolas, monospace';
  ink.textBaseline = "top";
  colours.forEach((colour, row) => {
    ink.fillStyle = colour;
    [...GLYPHS].forEach((glyph, col) => ink.fillText(glyph, col * ATLAS_W, row * ATLAS_H));
  });
  let width = 1, height = 1;
  let grid = glyphGrid(1, 1);
  let masks = new Float32Array(0);
  function resize(w: number, h: number, ratio: number) {
    width = w; height = h;
    canvas.width = Math.round(w * ratio); canvas.height = Math.round(h * ratio);
    ctx!.setTransform(ratio, 0, 0, ratio, 0, 0);
    grid = glyphGrid(w, h);
    masks = new Float32Array(grid.columns * grid.rows * 3);
    const scale = Math.min(w, h) * 0.34;
    for (let row = 0; row < grid.rows; row++) for (let col = 0; col < grid.columns; col++) {
      const i = row * grid.columns + col;
      const x = (col * grid.cell - w * 0.55) / scale;
      const y = (row * grid.cell * 1.6 - h * 0.46) / scale;
      // Duck's body, head and bill emerge from the same field as the waves.
      masks[i * 3] = Math.min(1, ellipse(x, y, -.12, .2, .8, .45)
        + ellipse(x, y, .4, -.3, .34, .4) + ellipse(x, y, .77, -.21, .25, .12));
      masks[i * 3 + 1] = Math.exp(-80 * (Math.hypot(x, y * 1.6) - .85) ** 2);
      masks[i * 3 + 2] = Math.min(1, [[-.8, -.4], [-.35, .15], [.12, -.25], [.65, .2], [.4, .65]]
        .reduce((sum, [sx, sy]) => sum + ellipse(x, y, sx, sy, .14, .14), 0));
    }
  }
  // `motion` scales how far glyphs stray from their cells; the caller keeps it 1 for the still frame.
  function draw(time: number, energy: number, direction: number, ripples: Ripple[], swell: number, motion = 1) {
    if (!ctx) return;
    ctx.clearRect(0, 0, width, height);
    const { columns, rows, cell } = grid;
    const cycle = time % 28;
    const shapeMix = Math.max(0, Math.sin((cycle - 15) / 10 * Math.PI)) * (cycle >= 15 && cycle <= 25 ? .85 : 0);
    const shape = Math.floor(time / 28) % 3;
    for (let row = 0; row < rows; row++) for (let col = 0; col < columns; col++) {
      const i = row * columns + col;
      const px = col * cell, py = row * cell * 1.6;
      const x = px / width, y = py / height;
      const radius = Math.hypot(x - .55, y - .48);
      const angle = Math.atan2(y - .48, x - .55);
      const wave = (Math.sin(x * 12 + y * 5 + time * .38)
        + Math.sin(radius * 19 - angle * 2 + time * (.6 + energy))
        + Math.cos(y * 13 - time * .46)) / 3;
      let light = (.1 + Math.max(0, wave) * .82) * (1 - shapeMix) + masks[i * 3 + shape] * shapeMix;
      // Bounded drift plus a slow diagonal wander: glyphs visibly travel, never escape their cell.
      let dx = (Math.sin(y * 7 + time * .65) * cell * 1.05
        + Math.sin(time * .21 + y * 1.3) * cell * .6
        + Math.cos(time * .14 - x * 1.7) * cell * .4) * motion;
      let dy = direction * Math.sin(radius * 10 - time) * cell * energy
        + (Math.cos(x * 9 + time * .55) * cell * .5
        + Math.sin(time * .17 + x * 2.1) * cell * .35) * motion;
      for (const ripple of ripples) {
        const rx = px - ripple.x * width, ry = py - ripple.y * height;
        const distance = Math.hypot(rx, ry);
        const ring = Math.exp(-(((distance - ripple.age * 260) / 48) ** 2)) * Math.max(0, 1 - ripple.age / 2.4);
        light += ring * .8;
        dx += rx / Math.max(1, distance) * ring * 17;
        dy += ry / Math.max(1, distance) * ring * 17;
      }
      light = Math.min(1, light + swell * Math.exp(-(((radius - (1 - swell) * .7) / .12) ** 2)) * .4);
      if (light < .055) continue;
      const hash = ((i * 1597 + 5171) >>> 0);
      const glyph = (hash + Math.floor(time * .6 + x * 4)) % GLYPHS.length;
      const colour = Math.floor((x + y + time * .015) * 2) % 3;
      ctx.globalAlpha = light * (.64 + energy * .2);
      ctx.drawImage(atlas, glyph * ATLAS_W * 2, colour * ATLAS_H * 2, ATLAS_W * 2, ATLAS_H * 2,
        px + dx, py + dy, cell, cell * 1.5);
    }
    ctx.globalAlpha = 1;
  }
  return { resize, draw };
}
