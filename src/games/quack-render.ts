/**
 * Draws a Quack Survivors run (`quack.ts`) onto a 2D canvas: the arena's dot grid, tokens, spit,
 * the bugs, the duck's weapons, the duck, effects, then the HUD in screen space. The camera keeps
 * the duck centred. Pure drawing: it never mutates the state.
 *
 * The weapons borrow the app's own marks: Orbit's beads are `OrbitSpinner`'s, Comet is the
 * send comet, Flock fires the `DuckMark` silhouette, and the player is the duck itself.
 *
 * Reduced motion keeps the game playable but still: no screen shake, no particle bursts, no
 * bobbing or invulnerability blink.
 */
import { DUCK_PATH } from "../gravity-well";
import type { GamePalette } from "./palette";
import {
  ENEMIES, PLAYER_RADIUS, WIN_MS, emberStats, maxHpOf, orbitBeads, radiusOf, xpForLevel,
  type Enemy, type QuackState, type Tone
} from "./quack";

export interface RenderView { w: number; h: number; reduced: boolean }

const FONT = '-apple-system, "Inter", system-ui, sans-serif';
const MONO = '"SFMono-Regular", Consolas, monospace';
const GRID = 28;

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

export function render(ctx: CanvasRenderingContext2D, s: QuackState | null, palette: GamePalette, view: RenderView): void {
  const { w, h, reduced } = view;
  ctx.globalAlpha = 1;
  ctx.fillStyle = palette.well;
  ctx.fillRect(0, 0, w, h);

  let ox = w / 2 - (s?.player.x ?? 0);
  let oy = h / 2 - (s?.player.y ?? 0);
  if (s && s.shake > 0 && !reduced) {
    const magnitude = Math.min(6, s.shake / 40);
    ox += Math.sin(s.time * 0.11) * magnitude;
    oy += Math.cos(s.time * 0.13) * magnitude;
  }

  // The arena: a dot grid that scrolls under the duck, so movement reads even in empty space.
  ctx.fillStyle = palette.border;
  const startX = ((ox % GRID) + GRID) % GRID;
  const startY = ((oy % GRID) + GRID) % GRID;
  for (let x = startX; x < w; x += GRID) for (let y = startY; y < h; y += GRID) ctx.fillRect(x - 0.75, y - 0.75, 1.5, 1.5);
  if (!s) return;

  const left = s.player.x - w / 2 - 40;
  const right = s.player.x + w / 2 + 40;
  const top = s.player.y - h / 2 - 40;
  const bottom = s.player.y + h / 2 + 40;
  const onScreen = (x: number, y: number, r = 0) => x + r > left && x - r < right && y + r > top && y - r < bottom;

  ctx.save();
  ctx.translate(ox, oy);

  // Ping rings ease outward and fade.
  ctx.strokeStyle = palette.accent;
  for (const ring of s.rings) {
    const progress = 1 - ring.life / ring.max;
    ctx.globalAlpha = Math.max(0, 1 - progress) * 0.85;
    ctx.lineWidth = 2.5 - progress * 1.5;
    ctx.beginPath();
    ctx.arc(ring.x, ring.y, ring.radius * (1 - (1 - progress) ** 3), 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  // Tokens: small accent diamonds; a merged token grows.
  ctx.fillStyle = palette.accent;
  for (const token of s.tokens) {
    if (!onScreen(token.x, token.y)) continue;
    const size = Math.min(6.5, 3 + Math.sqrt(token.value) * 0.8);
    ctx.globalAlpha = 0.92;
    ctx.beginPath();
    ctx.moveTo(token.x, token.y - size);
    ctx.lineTo(token.x + size * 0.72, token.y);
    ctx.lineTo(token.x, token.y + size);
    ctx.lineTo(token.x - size * 0.72, token.y);
    ctx.closePath();
    ctx.fill();
  }
  ctx.globalAlpha = 1;

  // Mosquito spit: a danger-red dart with a short tail.
  for (const spit of s.spits) {
    if (!onScreen(spit.x, spit.y, 20)) continue;
    const speed = Math.hypot(spit.vx, spit.vy) || 1;
    const ux = spit.vx / speed;
    const uy = spit.vy / speed;
    ctx.strokeStyle = palette.danger;
    ctx.globalAlpha = 0.5;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(spit.x - ux * 14, spit.y - uy * 14);
    ctx.lineTo(spit.x, spit.y);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.fillStyle = palette.danger;
    ctx.beginPath();
    ctx.moveTo(spit.x + ux * 5, spit.y + uy * 5);
    ctx.lineTo(spit.x - uy * 3.5, spit.y + ux * 3.5);
    ctx.lineTo(spit.x + uy * 3.5, spit.y - ux * 3.5);
    ctx.closePath();
    ctx.fill();
  }

  for (const enemy of s.enemies) {
    if (onScreen(enemy.x, enemy.y, ENEMIES[enemy.kind].radius * enemy.scale)) drawEnemy(ctx, enemy, s, palette, reduced);
  }

  drawEmber(ctx, s, palette, reduced);

  // Orbit beads with a soft halo, like the working mark's lead bead.
  for (const bead of orbitBeads(s)) {
    ctx.fillStyle = palette.accent;
    ctx.globalAlpha = 0.22;
    ctx.beginPath();
    ctx.arc(bead.x, bead.y, 10, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.beginPath();
    ctx.arc(bead.x, bead.y, 5.5, 0, Math.PI * 2);
    ctx.fill();
  }

  for (const bolt of s.bolts) {
    if (!onScreen(bolt.x, bolt.y, 40)) continue;
    if (bolt.kind === "flock") {
      // A duckling, banking through the swarm.
      const path = duck();
      const angle = Math.atan2(bolt.vy, bolt.vx);
      if (path) {
        const k = 11 / 256;
        ctx.save();
        ctx.translate(bolt.x, bolt.y);
        ctx.rotate(angle);
        ctx.scale(k, k);
        ctx.translate(-134, -129);
        ctx.fill(path, "evenodd");
        ctx.restore();
      } else {
        ctx.fillStyle = palette.accent;
        ctx.beginPath();
        ctx.arc(bolt.x, bolt.y, 5, 0, Math.PI * 2);
        ctx.fill();
      }
      continue;
    }
    const speed = Math.hypot(bolt.vx, bolt.vy) || 1;
    const ux = bolt.vx / speed;
    const uy = bolt.vy / speed;
    if (bolt.kind === "quill") {
      ctx.strokeStyle = palette.accent;
      ctx.lineCap = "round";
      ctx.lineWidth = 3;
      ctx.beginPath();
      ctx.moveTo(bolt.x - ux * 12, bolt.y - uy * 12);
      ctx.lineTo(bolt.x, bolt.y);
      ctx.stroke();
    } else {
      // The send comet: a bright head trailing an accent tail that fades out.
      const tail = ctx.createLinearGradient(bolt.x - ux * 46, bolt.y - uy * 46, bolt.x, bolt.y);
      tail.addColorStop(0, palette.well);
      tail.addColorStop(1, palette.accent);
      ctx.strokeStyle = tail;
      ctx.lineCap = "round";
      ctx.lineWidth = 7;
      ctx.globalAlpha = 0.85;
      ctx.beginPath();
      ctx.moveTo(bolt.x - ux * 46, bolt.y - uy * 46);
      ctx.lineTo(bolt.x, bolt.y);
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.fillStyle = palette.text;
      ctx.beginPath();
      ctx.arc(bolt.x, bolt.y, 4.5, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // Surge arcs: a jagged polyline that flashes and fades.
  for (const arc of s.arcs) {
    ctx.globalAlpha = Math.max(0, arc.life / arc.max);
    ctx.strokeStyle = palette.accent;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = 2.5;
    ctx.beginPath();
    ctx.moveTo(arc.points[0], arc.points[1]);
    for (let i = 2; i < arc.points.length; i += 2) {
      // A midpoint jog keeps the line reading as lightning rather than a ruler line.
      const mx = (arc.points[i - 2] + arc.points[i]) / 2 + Math.sin(s.time * 0.05 + i) * 4;
      const my = (arc.points[i - 1] + arc.points[i + 1]) / 2 + Math.cos(s.time * 0.05 + i) * 4;
      ctx.lineTo(mx, my);
      ctx.lineTo(arc.points[i], arc.points[i + 1]);
    }
    ctx.stroke();
    ctx.globalAlpha = 1;
  }

  drawDuck(ctx, s, palette, reduced);

  if (!reduced) {
    for (const particle of s.particles) {
      if (!onScreen(particle.x, particle.y)) continue;
      ctx.globalAlpha = Math.max(0, particle.life / particle.max);
      ctx.fillStyle = toneColor(palette, particle.tone);
      ctx.fillRect(particle.x - particle.size / 2, particle.y - particle.size / 2, particle.size, particle.size);
    }
    ctx.globalAlpha = 1;
  }
  ctx.restore();

  bossPointers(ctx, s, palette, w, h, ox, oy);
  hud(ctx, s, palette, w);
  if (s.banner) {
    ctx.globalAlpha = Math.max(0, Math.min(1, (s.banner.until - s.time) / 500, (s.time - (s.banner.until - 2600)) / 250));
    const urgent = /Segfault|Kernel Panic|rush/.test(s.banner.text);
    ctx.fillStyle = urgent ? palette.danger : palette.text;
    ctx.font = `700 15px ${FONT}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(s.banner.text, w / 2, h * 0.3);
    ctx.globalAlpha = 1;
  }
}

function drawEnemy(ctx: CanvasRenderingContext2D, e: Enemy, s: QuackState, palette: GamePalette, reduced: boolean) {
  const r = radiusOf(e);
  const angle = Math.atan2(s.player.y - e.y, s.player.x - e.x);
  const wiggle = reduced ? 0 : Math.sin(s.time * 0.022 + e.id * 1.7) * r * 0.22;
  const flash = e.flash > 0;
  ctx.save();
  ctx.translate(e.x, e.y);
  if (e.kind === "segfault" && !reduced) {
    // A glitch: the whole bug jumps sideways now and then.
    const glitch = Math.sin(s.time * 0.05 + e.id) > 0.93 ? Math.sin(s.time) * 5 : 0;
    ctx.translate(glitch, 0);
  }
  ctx.rotate(angle);

  // Legs, three pairs, scuttling out of phase.
  ctx.strokeStyle = e.kind === "segfault" ? palette.danger : palette.textDim;
  ctx.lineWidth = e.kind === "segfault" ? 2.4 : 1.4;
  ctx.lineCap = "round";
  ctx.beginPath();
  for (let i = -1; i <= 1; i++) {
    const x = i * r * 0.45;
    const swing = i === 0 ? -wiggle : wiggle;
    ctx.moveTo(x, -r * 0.6);
    ctx.lineTo(x + swing, -r * 1.25);
    ctx.moveTo(x, r * 0.6);
    ctx.lineTo(x - swing, r * 1.25);
  }
  ctx.stroke();

  if (e.kind === "gnat" || e.kind === "mosquito") {
    // Wings: two translucent flickering ovals.
    ctx.fillStyle = palette.textSoft;
    ctx.globalAlpha = 0.3;
    const flap = reduced ? 1 : 0.6 + Math.abs(Math.sin(s.time * (e.kind === "mosquito" ? 0.05 : 0.06) + e.id)) * 0.5;
    ctx.beginPath();
    ctx.ellipse(-r * 0.3, -r * 0.9, r * (e.kind === "mosquito" ? 1.5 : 0.9) * flap, r * 0.45, -0.4, 0, Math.PI * 2);
    ctx.ellipse(-r * 0.3, r * 0.9, r * (e.kind === "mosquito" ? 1.5 : 0.9) * flap, r * 0.45, 0.4, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;
  }

  ctx.fillStyle = flash ? palette.text : e.kind === "segfault" ? palette.well : palette.surface;
  ctx.strokeStyle = e.kind === "segfault" ? palette.danger : palette.textSoft;
  ctx.lineWidth = e.kind === "segfault" ? 2.5 : 1.5;
  ctx.beginPath();
  if (e.kind === "mosquito") {
    // A thin dart of a body with a long stinger.
    ctx.ellipse(-r * 0.2, 0, r * 1.5, r * 0.42, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(-r * 1.6, 0);
    ctx.lineTo(-r * 2.4, 0);
    ctx.stroke();
  } else if (e.kind === "splitter") {
    // Two lobes: the one that divides.
    ctx.ellipse(-r * 0.42, 0, r * 0.68, r * 0.82, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
    ctx.beginPath();
    ctx.ellipse(r * 0.45, 0, r * 0.52, r * 0.62, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  } else {
    ctx.ellipse(-r * 0.1, 0, r * 1.05, r * 0.82, 0, 0, Math.PI * 2);
    ctx.fill();
    if (!flash && e.kind !== "segfault") {
      // A lighter wash over the shell so bugs read against the dark arena.
      ctx.globalAlpha = 0.4;
      ctx.fillStyle = palette.textDim;
      ctx.fill();
      ctx.globalAlpha = 1;
    }
    ctx.stroke();
    if (e.kind === "beetle" || e.kind === "segfault") {
      // The shell's seam.
      ctx.beginPath();
      ctx.moveTo(-r * 1.05, 0);
      ctx.lineTo(r * 0.45, 0);
      ctx.stroke();
    }
  }
  if (e.kind === "segfault") {
    ctx.save();
    ctx.rotate(-angle);
    ctx.fillStyle = palette.danger;
    ctx.font = `800 10px ${MONO}`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("SEGV", 0, 0);
    ctx.restore();
  }

  // Head with two eyes in danger red: the one thing every bug has in common.
  ctx.fillStyle = flash ? palette.text : palette.textDim;
  ctx.beginPath();
  ctx.arc(r * 0.88, 0, r * 0.42, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = palette.danger;
  const eye = Math.max(1.4, r * 0.17);
  ctx.beginPath();
  ctx.arc(r * 1.02, -r * 0.18, eye, 0, Math.PI * 2);
  ctx.arc(r * 1.02, r * 0.18, eye, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  // Elites wear a modifier ring and glyph above the bug.
  if (e.elite) drawElite(ctx, e, s, palette);

  const tough = e.kind === "beetle" || e.kind === "segfault" || e.kind === "splitter" || e.kind === "mosquito" || e.elite;
  if (tough && e.hp < e.maxHp) {
    const width = r * 2;
    ctx.fillStyle = palette.border;
    ctx.fillRect(e.x - width / 2, e.y - r - 10, width, 3);
    ctx.fillStyle = palette.danger;
    ctx.fillRect(e.x - width / 2, e.y - r - 10, width * Math.max(0, e.hp / e.maxHp), 3);
  }
}

/** The elite marker: an accent ring plus a modifier glyph (swift chevrons, armored plate, burst star). */
function drawElite(ctx: CanvasRenderingContext2D, e: Enemy, s: QuackState, palette: GamePalette) {
  const r = radiusOf(e);
  ctx.save();
  ctx.translate(e.x, e.y);
  ctx.strokeStyle = palette.accent;
  ctx.lineWidth = 1.5;
  ctx.globalAlpha = 0.9;
  ctx.beginPath();
  ctx.arc(0, 0, r + 5, 0, Math.PI * 2);
  ctx.stroke();
  ctx.globalAlpha = 1;
  ctx.fillStyle = palette.accent;
  ctx.strokeStyle = palette.accent;
  ctx.lineWidth = 1.6;
  const g = r + 13;
  ctx.beginPath();
  if (e.elite === "swift") {
    // Two forward chevrons.
    ctx.moveTo(-4, -g + 4); ctx.lineTo(0, -g); ctx.lineTo(4, -g + 4);
    ctx.moveTo(-4, -g + 8); ctx.lineTo(0, -g + 4); ctx.lineTo(4, -g + 8);
    ctx.stroke();
  } else if (e.elite === "armored") {
    // A filled plate.
    ctx.rect(-3.5, -g - 3.5, 7, 7);
    ctx.fill();
  } else {
    // A four-point burst.
    ctx.moveTo(0, -g - 4.5); ctx.lineTo(1.6, -g - 1.6); ctx.lineTo(4.5, -g); ctx.lineTo(1.6, -g + 1.6);
    ctx.lineTo(0, -g + 4.5); ctx.lineTo(-1.6, -g + 1.6); ctx.lineTo(-4.5, -g); ctx.lineTo(-1.6, -g - 1.6);
    ctx.closePath();
    ctx.fill();
  }
  ctx.restore();
}

/** The ember aura: a soft disc of accent light that breathes, drawn under the duck. */
function drawEmber(ctx: CanvasRenderingContext2D, s: QuackState, palette: GamePalette, reduced: boolean) {
  const level = s.weapons.ember;
  if (!level) return;
  const { radius } = emberStats(level, s.evolved.ember);
  const p = s.player;
  const pulse = reduced ? 1 : 1 + Math.sin(s.time * 0.006) * 0.04;
  ctx.fillStyle = palette.accent;
  ctx.globalAlpha = 0.1;
  ctx.beginPath();
  ctx.arc(p.x, p.y, radius * pulse, 0, Math.PI * 2);
  ctx.fill();
  ctx.globalAlpha = 0.35;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.arc(p.x, p.y, radius * pulse, 0, Math.PI * 2);
  ctx.stroke();
  ctx.globalAlpha = 1;
}

function drawDuck(ctx: CanvasRenderingContext2D, s: QuackState, palette: GamePalette, reduced: boolean) {
  const p = s.player;
  const bob = reduced ? 0 : Math.sin(s.time * 0.008) * 1.6;
  if (p.invuln > 0) ctx.globalAlpha = reduced ? 0.55 : Math.floor(p.invuln / 80) % 2 ? 0.3 : 1;
  // A faint pool of accent light under the duck so it reads on any background.
  ctx.fillStyle = palette.accent;
  const alpha = ctx.globalAlpha;
  ctx.globalAlpha = alpha * 0.12;
  ctx.beginPath();
  ctx.arc(p.x, p.y, PLAYER_RADIUS + 9, 0, Math.PI * 2);
  ctx.fill();
  ctx.globalAlpha = alpha;
  const path = duck();
  if (path) {
    const k = 32 / 256;
    ctx.save();
    ctx.translate(p.x, p.y + bob);
    ctx.scale(p.facing.x < 0 ? -k : k, k);
    ctx.translate(-134, -129);
    ctx.fill(path, "evenodd");
    ctx.restore();
  } else {
    ctx.beginPath();
    ctx.arc(p.x, p.y + bob, PLAYER_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.globalAlpha = 1;
}

/** A small danger chevron at the edge pointing toward any Segfault off screen. */
function bossPointers(ctx: CanvasRenderingContext2D, s: QuackState, palette: GamePalette, w: number, h: number, ox: number, oy: number) {
  for (const e of s.enemies) {
    if (e.kind !== "segfault") continue;
    const sx = e.x + ox;
    const sy = e.y + oy;
    if (sx > 0 && sx < w && sy > 0 && sy < h) continue;
    const angle = Math.atan2(sy - h / 2, sx - w / 2);
    const x = Math.max(18, Math.min(w - 18, sx));
    const y = Math.max(40, Math.min(h - 18, sy));
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate(angle);
    ctx.fillStyle = palette.danger;
    ctx.beginPath();
    ctx.moveTo(8, 0);
    ctx.lineTo(-5, -6);
    ctx.lineTo(-5, 6);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
  }
}

function hud(ctx: CanvasRenderingContext2D, s: QuackState, palette: GamePalette, w: number) {
  const p = s.player;
  // XP: a thin accent rule along the top edge.
  ctx.fillStyle = palette.border;
  ctx.fillRect(0, 0, w, 3);
  ctx.fillStyle = palette.accent;
  ctx.fillRect(0, 0, w * Math.min(1, p.xp / xpForLevel(p.level)), 3);

  // HP bar and level, top left.
  const maxHp = maxHpOf(s);
  const ratio = Math.max(0, p.hp / maxHp);
  ctx.fillStyle = palette.surface;
  ctx.strokeStyle = palette.border;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(12, 14, 96, 8, 4);
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = ratio > 0.3 ? palette.accent : palette.danger;
  ctx.beginPath();
  ctx.roundRect(12, 14, Math.max(0, 96 * ratio), 8, 4);
  ctx.fill();
  ctx.font = `650 11px ${FONT}`;
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";
  ctx.fillStyle = palette.textSoft;
  ctx.fillText(`Lv ${p.level}`, 116, 18.5);

  // Clock, top centre, counting toward the win.
  ctx.textAlign = "center";
  ctx.font = `650 13px ${MONO}`;
  ctx.fillStyle = s.time >= WIN_MS - 30_000 ? palette.accent : palette.text;
  ctx.fillText(formatClock(s.time), w / 2, 18.5);

  ctx.textAlign = "right";
  ctx.font = `650 11px ${FONT}`;
  ctx.fillStyle = palette.textSoft;
  ctx.fillText(`${s.kills} ${s.kills === 1 ? "bug" : "bugs"}`, w - 12, 18.5);
}
