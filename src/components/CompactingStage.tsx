import { useEffect, useRef, useState } from "react";
import { usePresence, useReducedMotion } from "motion/react";
import {
  FINALE_MS, actFor, collapse, createWell, isDone, render as renderWell, stillFrame, step,
  type Palette, type WellState,
} from "../gravity-well";

export type CompactionReason = "manual" | "threshold" | "overflow";

const REASON_TEXT: Record<CompactionReason, string> = {
  manual: "you asked",
  threshold: "context was getting full",
  overflow: "context overflowed",
};

const formatElapsed = (ms: number) => {
  const s = Math.floor(ms / 1_000);
  const h = Math.floor(s / 3_600);
  const m = Math.floor((s % 3_600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
};

/**
 * The live compaction scene: a canvas gravity well that swallows text-like fragments,
 * escalating through calm → busy → dramatic acts the longer the model takes (a compaction
 * on a big model can run for minutes). The status line under the stage carries the reason
 * and a wall-clock elapsed timer.
 *
 * Invariants:
 * - `usePresence` ties it to `AnimatePresence`: when the transcript drops the stage the sim
 *   collapses (all remaining fragments fall in, the core shrinks, a ring pings) and
 *   `onEnded` → `safeToRemove()` frees it. A `FINALE_MS + 30` timeout covers environments
 *   where rAF doesn't run — the stage can never wedge in the DOM.
 * - Simulation `now` is wall-clock since mount, so a hidden tab resumes into the right act;
 *   frame `dt` is clamped to 50ms so a resume never produces a physics jump.
 * - Reduced motion draws `stillFrame` once (a static warm well) — no rAF loop — while the
 *   clock keeps ticking and the finale resolves immediately.
 * - jsdom's canvas returns a null context: everything except drawing still works.
 * - The palette is polled once a second rather than observing theme mutations.
 */
export function CompactingStage({ reason, ending = false, onEnded }: {
  reason: CompactionReason;
  /** Forces the collapse without `AnimatePresence` (tests, embeds). */
  ending?: boolean;
  /** Fires once the finale has finished — `safeToRemove` is called at the same moment. */
  onEnded?: () => void;
}) {
  const reduced = useReducedMotion() ?? false;
  const [isPresent, safeToRemove] = usePresence();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const startRef = useRef(Date.now());
  const wellRef = useRef<WellState | null>(null);
  const paletteRef = useRef<Palette>({ accent: "#9fe870", text: "#d5d5d5", textDim: "#8a8a8a", surface: "#1a1a1a" });
  const ctxRef = useRef<CanvasRenderingContext2D | null | undefined>(undefined);
  const leaving = !isPresent || ending;
  const leavingRef = useRef(leaving);
  leavingRef.current = leaving;
  const safeRef = useRef(safeToRemove);
  safeRef.current = safeToRemove;
  const onEndedRef = useRef(onEnded);
  onEndedRef.current = onEnded;
  const endedRef = useRef(false);
  const [elapsedMs, setElapsedMs] = useState(0);

  const now = () => Date.now() - startRef.current;

  const finish = () => {
    if (endedRef.current) return;
    endedRef.current = true;
    onEndedRef.current?.();
    safeRef.current?.();
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    ctxRef.current = canvas?.getContext("2d") ?? null;
    const w = Math.max(1, canvas?.clientWidth || 400);
    const h = Math.max(1, canvas?.clientHeight || 140);
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (canvas) { canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr); }
    if (ctxRef.current) ctxRef.current.setTransform(dpr, 0, 0, dpr, 0, 0);
    const seed = Date.now() & 0xffff;
    wellRef.current = reduced ? stillFrame(seed, w, h) : createWell(seed, w, h);

    const resize = () => {
      const el = canvasRef.current;
      const well = wellRef.current;
      if (!el || !well) return;
      const box = el.getBoundingClientRect();
      const rw = Math.max(1, box.width || el.clientWidth || 400);
      const rh = Math.max(1, box.height || el.clientHeight || 140);
      const ratio = Math.min(2, window.devicePixelRatio || 1);
      if (el.width !== Math.round(rw * ratio)) el.width = Math.round(rw * ratio);
      if (el.height !== Math.round(rh * ratio)) el.height = Math.round(rh * ratio);
      if (ctxRef.current) ctxRef.current.setTransform(ratio, 0, 0, ratio, 0, 0);
      well.w = rw; well.h = rh; well.cx = rw / 2; well.cy = rh / 2;
      well.maxDist = Math.hypot(rw, rh) / 2 + 30;
      if (reduced) renderStill();
    };
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : undefined;
    if (observer && canvas) observer.observe(canvas);

    const renderStill = () => {
      if (ctxRef.current && wellRef.current) renderWell(ctxRef.current, wellRef.current, paletteRef.current, now());
    };
    if (reduced) renderStill();

    const clock = window.setInterval(() => setElapsedMs(now()), 250);

    let raf = 0;
    if (!reduced) {
      let last = performance.now();
      let paletteAt = -1;
      const frame = (t: number) => {
        raf = requestAnimationFrame(frame);
        const well = wellRef.current;
        if (!well) return;
        const dt = Math.min(50, Math.max(1, t - last));
        last = t;
        const nowMs = now();
        if (paletteAt < nowMs) {
          paletteAt = nowMs + 1_000;
          const css = getComputedStyle(document.documentElement);
          paletteRef.current = {
            accent: css.getPropertyValue("--wc-accent").trim() || paletteRef.current.accent,
            text: css.getPropertyValue("--text").trim() || paletteRef.current.text,
            textDim: css.getPropertyValue("--text-dim").trim() || paletteRef.current.textDim,
            surface: css.getPropertyValue("--surface").trim() || paletteRef.current.surface,
          };
        }
        if (leavingRef.current && well.ending === undefined) collapse(well, nowMs);
        if (!document.hidden) step(well, dt, nowMs);
        if (ctxRef.current) renderWell(ctxRef.current, well, paletteRef.current, nowMs);
        if (well.ending !== undefined && isDone(well, nowMs)) finish();
      };
      raf = requestAnimationFrame(frame);
    }

    return () => {
      cancelAnimationFrame(raf);
      clearInterval(clock);
      observer?.disconnect();
    };
    // Runs once per mount (and when `reduced` changes); presence/onEnded go through refs so
    // the well is never recreated mid-flight.
  }, [reduced]);

  // Collapse is also driven from here so it starts even before the next rAF tick, and a
  // timeout covers contexts where rAF can't run (jsdom, throttled tabs) — removal is
  // guaranteed either way. Under reduced motion the finale resolves immediately.
  useEffect(() => {
    if (!leaving) return;
    const well = wellRef.current;
    if (well && well.ending === undefined) collapse(well, now());
    const t = window.setTimeout(finish, reduced ? 0 : FINALE_MS + 30);
    return () => clearTimeout(t);
  }, [leaving, reduced]);

  const act = actFor(elapsedMs);
  return (
    <div className={`compacting-stage${leaving ? " ending" : ""}`} role="status"
      data-act={act} {...(reduced ? { "data-still": "true" } : {})}>
      <canvas className="compacting-well" ref={canvasRef} aria-hidden="true" />
      <div className="compacting-status">
        <span className="thinking-shimmer">Compacting context…</span>
        <span className="compacting-sep">·</span>
        <span className="compacting-reason">{REASON_TEXT[reason]}</span>
        <span className="compacting-sep">·</span>
        <span className="compacting-elapsed">{formatElapsed(elapsedMs)}</span>
      </div>
    </div>
  );
}
