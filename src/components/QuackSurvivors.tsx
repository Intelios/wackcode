import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { FALLBACK_PALETTE, readPalette, type GamePalette } from "../games/palette";
import { bestScore, recordScore } from "../games/scores";
import { clearSave, readSave, writeSave } from "../games/save";
import { clearRun, savedRun, saveRun } from "../games/session";
import { formatClock, render } from "../games/quack-render";
import {
  EVOLUTIONS, MAX_LEVEL, STEP_MS, UPGRADE_NAMES, applyUpgrade, createQuack, score, setView, step, upgradeDetail, upgradeLevel,
  type Input, type QuackState, type UpgradeId
} from "../games/quack";
import { DuckMark } from "./DuckMark";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

type Screen = "title" | "playing" | "paused" | "levelup" | "over";

interface Result { score: number; best: number; isBest: boolean; won: boolean; time: number; kills: number; level: number }

const MOVE_KEYS: Record<string, keyof typeof NO_KEYS> = {
  w: "up", arrowup: "up", s: "down", arrowdown: "down", a: "left", arrowleft: "left", d: "right", arrowright: "right"
};
const NO_KEYS = { up: false, down: false, left: false, right: false };

/** Where a fresh mount starts: a run left mid-game comes back paused (or at its open level-up). */
function resumeScreen(state: QuackState | undefined): Screen {
  if (!state || state.phase !== "playing") return "title";
  return state.choices ? "levelup" : "paused";
}

/**
 * Quack Survivors (`games/quack.ts`) in the Games panel: the canvas, its rAF loop, input, and
 * the screens over it (title, paused, level-up, game over) as real buttons.
 *
 * Invariants:
 * - React state changes only when a screen changes. The loop steps the simulation at `STEP_MS`
 *   from an accumulator (frame time clamped to 50 ms) and draws the HUD on canvas, so `App`'s
 *   streaming re-renders and this loop never touch each other.
 * - The run lives in `games/session.ts`, so leaving the panel or switching chats or tabs pauses
 *   it instead of losing it, and it autosaves through `games/save.ts` so it also survives a quit.
 * - Keys are read only while the stage has focus, so the composer never steers the duck. Losing
 *   focus, hiding the window and Esc/P all pause; resuming is always explicit.
 * - jsdom's canvas has no context and no rAF loop runs there: everything but drawing still works.
 */
export function QuackSurvivors({ onLeave }: { onLeave?: () => void } = {}) {
  const reduced = useReducedMotion() ?? false;
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stateRef = useRef<QuackState | undefined>(savedRun<QuackState>("quack"));
  const [screen, setScreen] = useState<Screen>(() => resumeScreen(stateRef.current));
  const [result, setResult] = useState<Result>();
  const [best, setBest] = useState(() => bestScore("quack"));
  // The saved run, for the title screen's Continue card and its "1:23 · Level 4" line.
  const [saved, setSaved] = useState<QuackState | null>(() => readSave());
  const keys = useRef({ ...NO_KEYS });
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const size = useRef({ w: 0, h: 0 });
  const palette = useRef<GamePalette>(FALLBACK_PALETTE);
  const reducedRef = useRef(reduced);
  reducedRef.current = reduced;
  const screenRef = useRef(screen);
  screenRef.current = screen;
  const [choiceKey, setChoiceKey] = useState(0);
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

  // Leaving a screen (pause, level-up, quit) force-writes the run, and unmounting the panel does
  // too, so the save never lags more than the moment play actually stopped.
  useEffect(() => {
    if (screen === "playing") return;
    const state = stateRef.current;
    if (state && state.phase === "playing") writeSave(state, true);
  }, [screen]);

  useEffect(() => () => {
    const state = stateRef.current;
    if (state && state.phase === "playing") writeSave(state, true);
  }, []);

  const finish = useCallback((state: QuackState) => {
    clearRun("quack");
    clearSave();
    const final = score(state);
    const recorded = recordScore("quack", final);
    setBest(recorded.best);
    setResult({ score: final, ...recorded, won: state.phase === "won", time: state.time, kills: state.kills, level: state.player.level });
    setScreen("over");
  }, []);

  // The loop runs only while playing; every other screen shows the frozen last frame.
  useEffect(() => {
    draw();
    if (screen !== "playing" || typeof requestAnimationFrame === "undefined") return;
    let raf = 0;
    let last = performance.now();
    let carry = 0;
    const input = (): Input => {
      if (pointer.current) {
        const dx = pointer.current.x - size.current.w / 2;
        const dy = pointer.current.y - size.current.h / 2;
        const length = Math.hypot(dx, dy);
        return length < 14 ? { x: 0, y: 0 } : { x: dx / length, y: dy / length };
      }
      const k = keys.current;
      return { x: Number(k.right) - Number(k.left), y: Number(k.down) - Number(k.up) };
    };
    const frame = (now: number) => {
      const state = stateRef.current;
      if (!state) return;
      carry += Math.min(50, now - last);
      last = now;
      while (carry >= STEP_MS && state.phase === "playing" && !state.choices) {
        step(state, input(), STEP_MS);
        carry -= STEP_MS;
      }
      draw();
      if (state.phase !== "playing") { finish(state); return; }
      if (state.choices) { setScreen("levelup"); return; }
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
    const state = createQuack(Date.now() % 2_147_483_647, size.current.w ? size.current : undefined);
    stateRef.current = state;
    saveRun("quack", state);
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
    saveRun("quack", loaded);
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
    clearRun("quack");
    clearSave();
    stateRef.current = undefined;
    setSaved(null);
    setScreen("title");
  }

  function choose(id: UpgradeId) {
    const state = stateRef.current;
    if (!state) return;
    applyUpgrade(state, id);
    if (state.choices) {
      // Another banked level: the same screen comes back with fresh cards.
      setChoiceKey((key) => key + 1);
    } else {
      setScreen("playing");
      focusStage();
    }
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
    } else if (screen === "levelup" && /^[1-3]$/.test(key)) {
      const choice = stateRef.current?.choices?.[Number(key) - 1];
      if (choice) { event.preventDefault(); choose(choice); }
    }
  }

  function onKeyUp(event: React.KeyboardEvent) {
    const move = MOVE_KEYS[event.key.toLowerCase()];
    if (move) keys.current[move] = false;
  }

  const state = stateRef.current;
  const choices = screen === "levelup" ? state?.choices ?? [] : [];
  const overlay = reduced
    ? { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 } }
    : { initial: { opacity: 0, scale: 0.96, y: 8 }, animate: { opacity: 1, scale: 1, y: 0 }, exit: { opacity: 0, scale: 0.98 } };

  return (
    <div
      ref={stageRef}
      className={`game-stage ${screen === "playing" ? "playing" : ""}`}
      tabIndex={0}
      role="application"
      aria-label="Quack Survivors"
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
          pointer.current = { x: event.clientX - box.left, y: event.clientY - box.top };
        }}
        onPointerMove={(event) => {
          if (!pointer.current) return;
          const box = event.currentTarget.getBoundingClientRect();
          pointer.current = { x: event.clientX - box.left, y: event.clientY - box.top };
        }}
        onPointerUp={() => { pointer.current = null; }}
        onPointerCancel={() => { pointer.current = null; }}
      />
      <AnimatePresence initial={false}>
        {screen === "title" && (
          <motion.div key="title" className="game-overlay game-title" {...overlay} transition={{ duration: 0.24, ease: EASE }}>
            <DuckMark className="game-title-duck" />
            <h2>Quack Survivors</h2>
            <p>Bugs are closing in. Your weapons fire on their own. Survive five minutes.</p>
            {saved ? (
              <div className="game-continue">
                <button type="button" className="primary-button" autoFocus={engaged} onClick={continueRun}>
                  Continue · {formatClock(saved.time)}
                </button>
                <span className="game-best">Level {saved.player.level} · {Math.round(saved.player.hp)} HP · {saved.kills} {saved.kills === 1 ? "bug" : "bugs"}</span>
                <button type="button" className="secondary-button" autoFocus={!engaged} onClick={start}>New run</button>
              </div>
            ) : (
              <button type="button" className="primary-button" autoFocus={engaged} onClick={start}>Play</button>
            )}
            {best > 0 && <span className="game-best">Best {best.toLocaleString()}</span>}
            <dl className="game-controls">
              <div><dt><kbd>W</kbd><kbd>A</kbd><kbd>S</kbd><kbd>D</kbd></dt><dd>Move (or arrows, or hold the mouse)</dd></div>
              <div><dt><kbd>esc</kbd></dt><dd>Pause</dd></div>
            </dl>
          </motion.div>
        )}
        {screen === "paused" && state && (
          <motion.div key="paused" className="game-overlay game-dim" {...overlay} transition={{ duration: 0.2, ease: EASE }}>
            <h2>Paused</h2>
            <p>{formatClock(state.time)} survived · Level {state.player.level} · {state.kills} {state.kills === 1 ? "bug" : "bugs"}</p>
            <div className="game-actions">
              <button type="button" className="primary-button" autoFocus={engaged} onClick={resume}>Resume</button>
              <button type="button" className="secondary-button" onClick={saveQuit}>Save &amp; quit</button>
              <button type="button" className="secondary-button" onClick={endRun}>End run</button>
            </div>
          </motion.div>
        )}
        {screen === "levelup" && state && (
          <motion.div key={`levelup-${choiceKey}`} className="game-overlay game-dim" {...overlay} transition={{ duration: 0.22, ease: EASE }}>
            <span className="game-kicker">Level {state.player.level - state.pendingLevels + 1}</span>
            <h2>Pick an upgrade</h2>
            <div className="game-upgrades" role="group" aria-label="Upgrades">
              {choices.map((id, index) => {
                const level = upgradeLevel(state, id);
                const evolution = (EVOLUTIONS as readonly string[]).includes(id);
                return (
                  <motion.button
                    key={id}
                    type="button"
                    className={`game-upgrade ${evolution ? "evolution" : ""}`}
                    autoFocus={engaged && index === 0}
                    onClick={() => choose(id)}
                    initial={reduced ? false : { opacity: 0, y: 10 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.22, delay: reduced ? 0 : 0.05 * index, ease: EASE }}
                  >
                    <kbd>{index + 1}</kbd>
                    <span className="game-upgrade-text">
                      <strong>
                        {UPGRADE_NAMES[id]}
                        {level === 0 && id !== "snack" ? <em>{evolution ? "Evolution" : "New"}</em> : null}
                      </strong>
                      <span>{upgradeDetail(state, id)}</span>
                    </span>
                    {id !== "snack" && (
                      <span className="game-pips" aria-label={evolution ? "Evolution" : `Level ${level + 1} of ${MAX_LEVEL}`}>
                        {Array.from({ length: MAX_LEVEL }, (_, pip) => <i key={pip} className={pip < level || evolution ? "on" : pip === level ? "next" : ""} />)}
                      </span>
                    )}
                  </motion.button>
                );
              })}
            </div>
          </motion.div>
        )}
        {screen === "over" && result && (
          <motion.div key="over" className="game-overlay game-dim" {...overlay} transition={{ duration: 0.26, ease: EASE }}>
            <span className={`game-kicker ${result.won ? "" : "lost"}`}>{result.won ? "You survived" : "Squashed"}</span>
            <h2 className="game-score">{result.score.toLocaleString()}</h2>
            {result.isBest ? <span className="game-best new">New best</span> : <span className="game-best">Best {result.best.toLocaleString()}</span>}
            <p>{formatClock(result.time)} · Level {result.level} · {result.kills} {result.kills === 1 ? "bug" : "bugs"}{result.won ? " · ×2 for surviving" : ""}</p>
            <div className="game-actions">
              <button type="button" className="primary-button" autoFocus={engaged} onClick={start}>Play again</button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
