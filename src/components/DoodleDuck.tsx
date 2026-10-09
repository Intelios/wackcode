import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { FALLBACK_PALETTE, readPalette, type GamePalette } from "../games/palette";
import { bestScore, recordScore } from "../games/scores";
import { clearSave, readSave, writeSave } from "../games/doodle-save";
import { clearRun, savedRun, saveRun } from "../games/session";
import { formatClock, render } from "../games/doodle-render";
import { createDoodle, score, setView, step, STEP_MS, type DoodleState } from "../games/doodle";
import { DuckMark } from "./DuckMark";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

type Screen = "title" | "playing" | "paused" | "over";

interface Result { score: number; best: number; isBest: boolean; time: number; squash: number }

const MOVE_KEYS: Record<string, "left" | "right"> = {
  a: "left", arrowleft: "left", d: "right", arrowright: "right"
};
const NO_KEYS = { left: false, right: false };

/** Where a fresh mount starts: a run left mid-climb comes back paused. */
function resumeScreen(state: DoodleState | undefined): Screen {
  return state && state.phase === "playing" ? "paused" : "title";
}

/**
 * Doodle Duck (`games/doodle.ts`) in the Games panel: the canvas, its rAF loop, input, and the
 * screens over it (title, paused, game over) as real buttons. The same contract as
 * `QuackSurvivors`: React state changes only when a screen changes, the run lives in
 * `games/session.ts` and autosaves through `games/doodle-save.ts`, keys are read only while the
 * stage has focus, and jsdom's canvas-less environment still works.
 */
export function DoodleDuck({ onLeave }: { onLeave?: () => void } = {}) {
  const reduced = useReducedMotion() ?? false;
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stateRef = useRef<DoodleState | undefined>(savedRun<DoodleState>("doodle"));
  const [screen, setScreen] = useState<Screen>(() => resumeScreen(stateRef.current));
  const [result, setResult] = useState<Result>();
  const [best, setBest] = useState(() => bestScore("doodle"));
  const [saved, setSaved] = useState<DoodleState | null>(() => readSave());
  const keys = useRef({ ...NO_KEYS });
  const pointer = useRef<number | null>(null);
  const size = useRef({ w: 0, h: 0 });
  const palette = useRef<GamePalette>(FALLBACK_PALETTE);
  const reducedRef = useRef(reduced);
  reducedRef.current = reduced;
  const screenRef = useRef(screen);
  screenRef.current = screen;
  // Screens take focus only once the player has acted here: a remount (chat or tab switch with
  // the panel open) must not pull the caret out of the composer.
  const [engaged, setEngaged] = useState(false);

  const draw = useCallback(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (!ctx) return;
    const ratio = Math.min(2, window.devicePixelRatio || 1);
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    render(ctx, stateRef.current ?? null, palette.current, { ...size.current, reduced: reducedRef.current });
  }, []);

  const pause = useCallback(() => {
    if (screenRef.current === "playing") setScreen("paused");
  }, []);

  // Canvas sizing follows the panel; the theme is polled once a second like CompactingStage, and
  // the same tick carries the throttled autosave.
  useEffect(() => {
    const stage = stageRef.current;
    const canvas = canvasRef.current;
    if (!stage || !canvas) return;
    palette.current = readPalette(palette.current);
    const resize = () => {
      const ratio = Math.min(2, window.devicePixelRatio || 1);
      const w = stage.clientWidth;
      const h = stage.clientHeight;
      size.current = { w, h };
      canvas.width = Math.round(w * ratio);
      canvas.height = Math.round(h * ratio);
      if (stateRef.current) setView(stateRef.current, w, h);
      draw();
    };
    resize();
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(resize) : undefined;
    observer?.observe(stage);
    const poll = window.setInterval(() => {
      palette.current = readPalette(palette.current);
      if (screenRef.current !== "playing") draw();
      else {
        const state = stateRef.current;
        if (state && state.phase === "playing") writeSave(state);
      }
    }, 1000);
    return () => {
      observer?.disconnect();
      window.clearInterval(poll);
    };
  }, [draw]);

  // Hiding the window or switching apps pauses; so does unmounting (the run stays saved).
  useEffect(() => {
    const onHidden = () => { if (document.hidden) pause(); };
    window.addEventListener("blur", pause);
    document.addEventListener("visibilitychange", onHidden);
    return () => {
      window.removeEventListener("blur", pause);
      document.removeEventListener("visibilitychange", onHidden);
    };
  }, [pause]);

  // Leaving a screen (pause, quit) force-writes the run, and unmounting the panel does too, so
  // the save never lags more than the moment play actually stopped.
  useEffect(() => {
    if (screen === "playing") return;
    const state = stateRef.current;
    if (state && state.phase === "playing") writeSave(state, true);
  }, [screen]);

  useEffect(() => () => {
    const state = stateRef.current;
    if (state && state.phase === "playing") writeSave(state, true);
  }, []);

  const finish = useCallback((state: DoodleState) => {
    clearRun("doodle");
    clearSave();
    const final = score(state);
    const recorded = recordScore("doodle", final);
    setBest(recorded.best);
    setResult({ score: final, ...recorded, time: state.time, squash: state.squash });
    setScreen("over");
  }, []);

  // The loop runs only while playing; every other screen shows the frozen last frame.
  useEffect(() => {
    draw();
    if (screen !== "playing" || typeof requestAnimationFrame === "undefined") return;
    let raf = 0;
    let last = performance.now();
    let carry = 0;
    const input = () => {
      if (pointer.current !== null && stateRef.current) {
        // Steering toward the held pointer: close in means ease off, far means full tilt.
        const dx = pointer.current - stateRef.current.duck.x;
        return { x: Math.max(-1, Math.min(1, dx / 24)) };
      }
      const k = keys.current;
      return { x: Number(k.right) - Number(k.left) };
    };
    const frame = (now: number) => {
      const state = stateRef.current;
      if (!state) return;
      carry += Math.min(50, now - last);
      last = now;
      while (carry >= STEP_MS && state.phase === "playing") {
        step(state, input(), STEP_MS);
        carry -= STEP_MS;
      }
      draw();
      if (state.phase !== "playing") { finish(state); return; }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [screen, draw, finish]);

  function focusStage() {
    setEngaged(true);
    requestAnimationFrame(() => stageRef.current?.focus());
  }

  function start() {
    const state = createDoodle(Date.now() % 2_147_483_647, size.current.w ? size.current : undefined);
    stateRef.current = state;
    saveRun("doodle", state);
    // The new run replaces any old save immediately, not on the next autosave tick.
    writeSave(state, true);
    setSaved(state);
    keys.current = { ...NO_KEYS };
    pointer.current = null;
    setResult(undefined);
    setScreen("playing");
    focusStage();
  }

  function continueRun() {
    const loaded = readSave();
    if (!loaded) { setSaved(null); return; }
    stateRef.current = loaded;
    saveRun("doodle", loaded);
    setSaved(loaded);
    keys.current = { ...NO_KEYS };
    pointer.current = null;
    setResult(undefined);
    setScreen(resumeScreen(loaded));
    focusStage();
  }

  function resume() {
    keys.current = { ...NO_KEYS };
    setScreen("playing");
    focusStage();
  }

  /** Saves right now and leaves for the chat; the title screen is the fallback. */
  function saveQuit() {
    const state = stateRef.current;
    if (state && state.phase === "playing") writeSave(state, true);
    setSaved(readSave());
    keys.current = { ...NO_KEYS };
    pointer.current = null;
    if (onLeave) onLeave();
    else setScreen("title");
  }

  function endRun() {
    clearRun("doodle");
    clearSave();
    stateRef.current = undefined;
    setSaved(null);
    setScreen("title");
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const key = event.key.toLowerCase();
    if (screen === "playing") {
      if (MOVE_KEYS[key]) {
        keys.current[MOVE_KEYS[key]] = true;
        event.preventDefault();
      } else if (key === "escape" || key === "p") {
        event.preventDefault();
        setScreen("paused");
      } else if (key === " ") event.preventDefault();
    } else if (screen === "paused" && (key === "escape" || key === "p")) {
      event.preventDefault();
      resume();
    }
  }

  function onKeyUp(event: React.KeyboardEvent) {
    const move = MOVE_KEYS[event.key.toLowerCase()];
    if (move) keys.current[move] = false;
  }

  const state = stateRef.current;
  const overlay = reduced
    ? { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 } }
    : { initial: { opacity: 0, scale: 0.96, y: 8 }, animate: { opacity: 1, scale: 1, y: 0 }, exit: { opacity: 0, scale: 0.98 } };

  return (
    <div
      ref={stageRef}
      className={`game-stage ${screen === "playing" ? "playing" : ""}`}
      tabIndex={0}
      role="application"
      aria-label="Doodle Duck"
      aria-roledescription="game"
      onKeyDown={onKeyDown}
      onKeyUp={onKeyUp}
      onBlur={() => {
        // Checked a beat later: a screen's button unmounting drops focus for a moment before
        // `focusStage` takes it back, and that must not pause a run that just resumed.
        window.setTimeout(() => {
          if (stageRef.current?.contains(document.activeElement)) return;
          keys.current = { ...NO_KEYS };
          pointer.current = null;
          pause();
        }, 60);
      }}
    >
      <canvas
        ref={canvasRef}
        className="game-canvas"
        aria-hidden="true"
        onPointerDown={(event) => {
          if (screen !== "playing") return;
          event.currentTarget.setPointerCapture?.(event.pointerId);
          const box = event.currentTarget.getBoundingClientRect();
          pointer.current = event.clientX - box.left;
        }}
        onPointerMove={(event) => {
          if (pointer.current === null) return;
          const box = event.currentTarget.getBoundingClientRect();
          pointer.current = event.clientX - box.left;
        }}
        onPointerUp={() => { pointer.current = null; }}
        onPointerCancel={() => { pointer.current = null; }}
      />
      <AnimatePresence initial={false}>
        {screen === "title" && (
          <motion.div key="title" className="game-overlay game-title" {...overlay} transition={{ duration: 0.24, ease: EASE }}>
            <DuckMark className="game-title-duck" />
            <h2>Doodle Duck</h2>
            <p>Bounce from lily pad to cloud to star. How high can the duck climb?</p>
            {saved ? (
              <div className="game-continue">
                <button type="button" className="primary-button" autoFocus={engaged} onClick={continueRun}>
                  Continue · {score(saved)} m
                </button>
                <span className="game-best">{formatClock(saved.time)} · {saved.squash} {saved.squash === 1 ? "bug" : "bugs"} squashed</span>
                <button type="button" className="secondary-button" autoFocus={!engaged} onClick={start}>New run</button>
              </div>
            ) : (
              <button type="button" className="primary-button" autoFocus={engaged} onClick={start}>Play</button>
            )}
            {best > 0 && <span className="game-best">Best {best.toLocaleString()} m</span>}
            <dl className="game-controls">
              <div><dt><kbd>A</kbd><kbd>D</kbd></dt><dd>Steer (or arrows, or hold the mouse)</dd></div>
              <div><dt><kbd>esc</kbd></dt><dd>Pause</dd></div>
            </dl>
          </motion.div>
        )}
        {screen === "paused" && state && (
          <motion.div key="paused" className="game-overlay game-dim" {...overlay} transition={{ duration: 0.2, ease: EASE }}>
            <h2>Paused</h2>
            <p>{score(state)} m climbed · {formatClock(state.time)} · {state.squash} {state.squash === 1 ? "bug" : "bugs"} squashed</p>
            <div className="game-actions">
              <button type="button" className="primary-button" autoFocus={engaged} onClick={resume}>Resume</button>
              <button type="button" className="secondary-button" onClick={saveQuit}>Save &amp; quit</button>
              <button type="button" className="secondary-button" onClick={endRun}>End run</button>
            </div>
          </motion.div>
        )}
        {screen === "over" && result && (
          <motion.div key="over" className="game-overlay game-dim" {...overlay} transition={{ duration: 0.26, ease: EASE }}>
            <span className="game-kicker lost">You fell</span>
            <h2 className="game-score">{result.score.toLocaleString()} m</h2>
            {result.isBest ? <span className="game-best new">New best</span> : <span className="game-best">Best {result.best.toLocaleString()} m</span>}
            <p>{formatClock(result.time)} · {result.squash} {result.squash === 1 ? "bug" : "bugs"} squashed</p>
            <div className="game-actions">
              <button type="button" className="primary-button" autoFocus={engaged} onClick={start}>Play again</button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
