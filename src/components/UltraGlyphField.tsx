import { useLayoutEffect, useRef } from "react";
import { useReducedMotion } from "motion/react";
import { parseHex } from "../theme";
import {
  fieldDensity, glyphFor, mixRgb, scanCloseness, scanHead, spectrumAt, ULTRA_FLOOD, type Rgb
} from "../ultra-fx";

const STOP_TOKENS = ["--wc-ultra-cyan", "--wc-ultra-violet", "--wc-ultra-orchid", "--wc-ultra-pink", "--wc-ultra-peach"];
const PITCH = 6.6;
const LEFT = 8;
// The thumb sits over the track's last ~30px, so the field stops short of it.
const THUMB_ROOM = 30;
const REROLL_MS = 80;
const REROLL_SHARE = 0.35;
const ENTRANCE_MS = 750;
// The spectrum slides along the field this slowly (one stop-span per ~20s).
const DRIFT_PER_MS = 1 / 20000;

/**
 * The reasoning slider's fill at Ultra: a strip of monospace glyphs (`ultra-fx.ts`) sparse at the
 * start and packed toward the thumb, cells re-rolling, a bright scan head sweeping through, in the
 * prism running cyan to peach. Max's sheen and streaming dots belong to the model's own top level;
 * this is Ultra's alone. On mount the fill floods with dense noise that resolves left to right
 * (it replays when the popover opens at Ultra, beside the thumb's pop). Reduced motion paints one
 * still frame with the head parked. The canvas spans the whole track, so the fill's own width
 * transition reveals it; with no 2D context (jsdom) the wrapper stays empty.
 */
export function UltraGlyphField() {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const reduced = useReducedMotion() ?? false;

  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!wrap || !canvas || !ctx) return;
    const track = wrap.closest<HTMLElement>(".reasoning-track") ?? wrap;
    const width = track.offsetWidth;
    const height = track.offsetHeight || 30;
    const ratio = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    ctx.font = '11px "SFMono-Regular", Consolas, monospace';
    ctx.textBaseline = "middle";

    const css = getComputedStyle(document.documentElement);
    const glow: Rgb = parseHex(css.getPropertyValue("--wc-ultra-glow")) ?? [255, 255, 255];
    const stops = STOP_TOKENS.map((name) => parseHex(css.getPropertyValue(name))).filter((stop): stop is Rgb => Boolean(stop));
    const columns = Math.max(0, Math.floor((width - THUMB_ROOM - LEFT) / PITCH) + 1);
    const noise = Array.from({ length: columns }, () => Math.random());
    const born = performance.now();
    let rolledAt = born;
    let raf = 0;

    const paint = (now: number) => {
      const elapsed = now - born;
      ctx.clearRect(0, 0, width, height);
      if (!reduced && now - rolledAt > REROLL_MS) {
        rolledAt = now;
        for (let column = 0; column < columns; column++) if (Math.random() < REROLL_SHARE) noise[column] = Math.random();
      }
      const head = reduced ? columns * 0.7 : scanHead(elapsed, columns);
      const reveal = reduced ? 1 : Math.min(1, elapsed / ENTRANCE_MS);
      for (let column = 0; column < columns; column++) {
        const x = column / columns;
        let glyph: string;
        let colour: Rgb;
        let alpha: number;
        if (x > reveal) {
          glyph = ULTRA_FLOOD[Math.floor(Math.random() * ULTRA_FLOOD.length)];
          colour = glow;
          alpha = 0.95;
        } else {
          const near = scanCloseness(column, head);
          glyph = glyphFor(fieldDensity(x, noise[column], near));
          colour = mixRgb(spectrumAt(stops, x + elapsed * DRIFT_PER_MS), glow, near * 0.8);
          alpha = 0.5 + 0.5 * Math.min(1, near + x * 0.4);
        }
        if (glyph === " ") continue;
        ctx.fillStyle = `rgb(${colour.map(Math.round).join(" ")} / ${alpha})`;
        ctx.fillText(glyph, LEFT + column * PITCH, height / 2 + 0.5);
      }
      if (!reduced) raf = requestAnimationFrame(paint);
    };
    paint(born);
    return () => cancelAnimationFrame(raf);
  }, [reduced]);

  return (
    <div ref={wrapRef} className="reasoning-ultra-fx" aria-hidden="true">
      <canvas ref={canvasRef} />
    </div>
  );
}
