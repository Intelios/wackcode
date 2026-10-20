import { useRef, useSyncExternalStore } from "react";
import { useReducedMotion } from "motion/react";
import { ULTRA_RING, ULTRA_SETTLED, ULTRA_STEP_MS, ultraMarkFrame } from "../ultra-fx";

interface UltraMarkProps {
  /** Run the LED cycle. Off for marks that only label (sidebar, menus) and while a run owns the control. */
  animated?: boolean;
  /** "ink" draws the mark in one dark violet, for the glow-white slider thumb. */
  tone?: "prism" | "ink";
  className?: string;
}

// Ring position → spectrum stop, bouncing back round the far side so neighbours always blend.
const STOPS = ["cyan", "violet", "orchid", "pink", "peach", "pink", "orchid", "violet"] as const;
const CELL = 3;
const PITCH = 4;

// One clock for every animated mark on screen: it only runs while a mark is listening and the
// window is visible, and each mark reads its own phase from it (see `born` below).
let tick = 0;
let timer: number | undefined;
const listeners = new Set<() => void>();
function start() {
  if (timer !== undefined || document.hidden) return;
  timer = window.setInterval(() => {
    tick += 1;
    listeners.forEach((listener) => listener());
  }, ULTRA_STEP_MS);
}
function stop() {
  window.clearInterval(timer);
  timer = undefined;
}
function onVisibility() {
  if (document.hidden) stop();
  else if (listeners.size) start();
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) document.addEventListener("visibilitychange", onVisibility);
  start();
  return () => {
    listeners.delete(listener);
    if (listeners.size) return;
    stop();
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
const still = () => () => {};
const readTick = () => tick;

/**
 * Ultra's mark, shared by Ultra Plan and the Ultra reasoning level: a 3×3 grid of pixels that
 * spells a wildcard. Animated, a lit pixel orbits the ring twice with two beads trailing it (the
 * bead trail's cousin), the grid flashes `+`, flips to `×`, then holds the settled `✱`; the cycle
 * is in `ultra-fx.ts`. Each mark starts its cycle when it starts animating, so switching into
 * Ultra boots it from the orbit. It lives in JS rather than CSS because the frames are discrete
 * LED steps across nine cells. Static marks and reduced motion show the settled wildcard.
 * Always decorative: the label next to it, or the chip's hidden words, carry the meaning.
 */
export function UltraMark({ animated = false, tone = "prism", className = "" }: UltraMarkProps) {
  const reduced = useReducedMotion() ?? false;
  const live = animated && !reduced;
  const born = useRef<number | null>(null);
  if (!live) born.current = null;
  else if (born.current === null) born.current = performance.now();
  useSyncExternalStore(live ? subscribe : still, readTick, readTick);
  const frame = live && born.current !== null ? ultraMarkFrame(performance.now() - born.current) : ULTRA_SETTLED;

  return (
    <svg viewBox="0 0 11 11" className={`ultra-mark ${tone} ${live ? "live" : ""} ${className}`} aria-hidden="true" focusable="false">
      {frame.map((opacity, cell) => {
        const ring = ULTRA_RING.indexOf(cell as (typeof ULTRA_RING)[number]);
        return (
          <rect
            key={cell}
            className={ring < 0 ? "core" : STOPS[ring]}
            x={(cell % 3) * PITCH}
            y={Math.floor(cell / 3) * PITCH}
            width={CELL}
            height={CELL}
            rx="0.9"
            style={{ opacity }}
          />
        );
      })}
    </svg>
  );
}
