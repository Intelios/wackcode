/** One persistent decorative layer for both areas. Simulation and frame writes stay outside
 * React. Pausing cancels the loop (rather than drawing invisible frames); a fresh clock on
 * resume prevents time jumps. Reduce Motion always draws the same still composition. */
import { useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type Ref } from "react";
import { useReducedMotion } from "motion/react";
import { PHASE_MOTION, type BackgroundPhase } from "../dynamic-background";
import { resolveTheme } from "../theme";
import type { DynamicBackgroundPalette, DynamicBackgroundStyle } from "../types";
import { createGlyphField, type Ripple } from "./dynamic-glyphs";

export interface DynamicBackdropHandle {
  pulse: () => void;
  ripple: (clientX: number, clientY: number) => void;
}
interface Props {
  ref?: Ref<DynamicBackdropHandle>;
  style: DynamicBackgroundStyle;
  palette: DynamicBackgroundPalette;
  intensity: number;
  accent?: string | null;
  background?: string | null;
  scene: "hero" | "chat";
  phase: BackgroundPhase;
  focused: boolean;
}

export function DynamicBackdrop({ ref, style, palette, intensity, accent, background, scene, phase, focused }: Props) {
  const reduce = useReducedMotion() ?? false;
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const blooms = useRef<(HTMLDivElement | null)[]>([]);
  const engine = useRef<DynamicBackdropHandle | null>(null);
  const current = useRef({ phase, focused, reduce, intensity });
  useLayoutEffect(() => { current.current = { phase, focused, reduce, intensity }; }, [phase, focused, reduce, intensity]);
  const [visible, setVisible] = useState(() => !document.hidden);
  const colours = useMemo(() => {
    const vars = resolveTheme({ accent, background }).variables;
    return [1, 2, 3].map((n) => vars[`--wc-dynamic-${palette}-${n}`]);
  }, [accent, background, palette]);
  const running = focused && visible && !reduce && intensity > 0;
  const runningRef = useRef(running);
  useLayoutEffect(() => { runningRef.current = running; }, [running]);
  const lifecycle = useRef<(() => void) | null>(null);
  useImperativeHandle(ref, () => ({
    pulse: () => engine.current?.pulse(),
    ripple: (x, y) => engine.current?.ripple(x, y)
  }), []);
  useEffect(() => {
    const change = () => setVisible(!document.hidden);
    document.addEventListener("visibilitychange", change);
    return () => document.removeEventListener("visibilitychange", change);
  }, []);
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const glyphs = style === "glyphFlow" && canvas.current ? createGlyphField(canvas.current, colours) : undefined;
    let raf = 0, previous = 0, time = 0, energy = .18, direction = 0, swell = 0;
    let width = 1, height = 1;
    let profileFrames = 0, profileMs = 0, profileMax = 0;
    const profile = window.setInterval(() => {
      console.warn("dynamic-profile", JSON.stringify({ style, phase: current.current.phase, frames: profileFrames, meanMs: profileFrames ? profileMs / profileFrames : 0, maxMs: profileMax, running: runningRef.current }));
      profileFrames = 0; profileMs = 0; profileMax = 0;
    }, 10000);
    let ripples: Ripple[] = [];
    // Intensity now also buys movement: low values stay gentle, zero still turns the effect off.
    const motion = () => .4 + 1.15 * Math.max(0, Math.min(100, current.current.intensity)) / 100;
    const draw = (still = false) => {
      const profileStart = performance.now();
      const t = still ? 0 : time;
      const e = still ? .18 : energy;
      const d = still ? 0 : direction;
      const m = still ? 1 : motion();
      if (glyphs) glyphs.draw(t, e, d, still ? [] : ripples, still ? 0 : swell, m);
      if (style === "colourDrift") blooms.current.forEach((bloom, i) => {
        if (!bloom) return;
        const angle = t * (.19 + e * .1) + i * Math.PI * 2 / 3;
        const spread = 1 + d * .22;
        const x = Math.cos(angle) * 45 * spread * m;
        const y = Math.sin(angle * .83) * 34 * spread * m;
        bloom.style.transform = `translate(${x}%, ${y}%) rotate(${Math.sin(angle * .7) * 30}deg) scale(${1 + Math.sin(angle) * .18 * m + e * .16 + (still ? 0 : swell * .18)})`;
        bloom.style.opacity = String(.6 + Math.sin(angle + i) * .12 + e * .14);
      });
      const profileElapsed = performance.now() - profileStart; profileFrames++; profileMs += profileElapsed; profileMax = Math.max(profileMax, profileElapsed);
    };
    const resize = () => {
      const box = el.getBoundingClientRect();
      width = Math.max(1, box.width); height = Math.max(1, box.height);
      glyphs?.resize(width, height, Math.min(2, window.devicePixelRatio || 1));
      draw(current.current.reduce);
    };
    const frame = (now: number) => {
      raf = 0;
      if (!runningRef.current) return;
      if (!previous) previous = now;
      const elapsed = now - previous;
      if (elapsed >= 1000 / 30) {
        const dt = Math.min(.06, elapsed / 1000);
        previous = now;
        const target = PHASE_MOTION[current.current.phase];
        const blend = 1 - Math.exp(-dt * 2);
        energy += (target.energy - energy) * blend;
        direction += (target.direction - direction) * blend;
        time += dt * 2.2 * motion();
        swell = Math.max(0, swell - dt / 1.8);
        ripples = ripples.filter((ripple) => (ripple.age += dt) < 2.4);
        draw();
      }
      raf = requestAnimationFrame(frame);
    };
    const sync = () => {
      if (raf) cancelAnimationFrame(raf);
      raf = 0; previous = 0;
      if (runningRef.current && (style !== "glyphFlow" || glyphs)) raf = requestAnimationFrame(frame);
      else {
        ripples = []; swell = 0;
        if (current.current.reduce) draw(true);
      }
    };
    engine.current = {
      pulse: () => { if (runningRef.current) swell = 1; },
      ripple: (x, y) => {
        if (!runningRef.current || style !== "glyphFlow" || !glyphs) return;
        const box = el.getBoundingClientRect();
        ripples = [...ripples.slice(-3), { x: (x - box.left) / width, y: (y - box.top) / height, age: 0 }];
      }
    };
    lifecycle.current = sync;
    const observer = new ResizeObserver(resize);
    observer.observe(el);
    resize(); sync();
    return () => {
      if (raf) cancelAnimationFrame(raf);
      observer.disconnect();
      clearInterval(profile);
      engine.current = null;
      lifecycle.current = null;
    };
  }, [style, colours]);
  useEffect(() => { lifecycle.current?.(); }, [running, reduce]);

  const paint = {
    "--wc-dynamic-strength": Math.max(0, Math.min(100, intensity)) / 100,
    ...Object.fromEntries(colours.map((colour, i) => [`--wc-dynamic-paint-${i + 1}`, colour]))
  } as CSSProperties;
  return <div ref={root} className="dynamic-backdrop" data-scene={scene} data-style={style} style={paint} aria-hidden="true">
    <div className="dynamic-ink">
      {style === "glyphFlow" ? <canvas ref={canvas} /> : <div className="dynamic-blooms">
        {[0, 1, 2].map((i) => <div key={i} ref={(node) => { blooms.current[i] = node; }} className={`dynamic-bloom bloom-${i + 1}`} />)}
      </div>}
    </div>
    <div className="dynamic-veil" />
  </div>;
}
