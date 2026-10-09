import { memo, useEffect, useState, type ComponentType } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { bestScore } from "../games/scores";
import { hasSave as hasQuackSave } from "../games/save";
import { hasSave as hasDoodleSave } from "../games/doodle-save";
import { currentGame, enterGame, exitToArcade, type GameId } from "../games/session";
import type { TaskStatus } from "../types";
import { Icon } from "./Icons";
import { DuckMark } from "./DuckMark";
import { QuackSurvivors } from "./QuackSurvivors";
import { DoodleDuck } from "./DoodleDuck";

const TOAST_MS = 9000;

/** The arcade's roster. A new game is an entry here and its own module under `src/games/`. */
interface GameEntry {
  id: GameId;
  name: string;
  blurb: string;
  /** The best score's unit, shown beside the number ("m" for a climb, nothing for a count). */
  unit?: string;
  /** The card's "Resume" dot: a save means a run is waiting, whatever the panel's history.
   *  Reads localStorage during render, which is fine here: entering or leaving a game — the
   *  only things that change it — re-renders this panel. */
  hasSave: () => boolean;
  Component: ComponentType<{ onLeave?: () => void }>;
}

const GAMES: GameEntry[] = [
  {
    id: "quack",
    name: "Quack Survivors",
    blurb: "Weapons fire on their own. Outlast the bugs for five minutes.",
    hasSave: hasQuackSave,
    Component: QuackSurvivors
  },
  {
    id: "doodle",
    name: "Doodle Duck",
    blurb: "Bounce from lily pad to cloud to star, and don't look down.",
    unit: "m",
    hasSave: hasDoodleSave,
    Component: DoodleDuck
  }
];

interface GamesPanelProps {
  /** The open chat's status: the header shows it working, and its end raises the toast. */
  status: TaskStatus;
  /** The open chat. A status change that comes from switching chats is not a run ending. */
  watchKey: string;
  agentName: string;
  onClose: () => void;
  onBackToChat: () => void;
}

type Ending = "finished" | "stopped" | "failed";

function endingFor(status: TaskStatus): Ending {
  return status === "error" ? "failed" : status === "interrupted" ? "stopped" : "finished";
}

const busy = (status: TaskStatus) => status === "running" || status === "stopping";

/**
 * The side panel's Games view: something small to play while a slow model works. The panel is a
 * thin shell — header, the Arcade home or the entered game, the run-ended toast. Which game is
 * open lives in `games/session.ts`, so a view or tab swap comes back to it paused, while closing
 * the panel resets to the Arcade home (`exitToArcade`, called from every close path).
 *
 * Memo'd: `App` re-renders on every streaming delta, and nothing here depends on the stream.
 */
export const GamesPanel = memo(function GamesPanel({ status, watchKey, agentName, onClose, onBackToChat }: GamesPanelProps) {
  const reduce = useReducedMotion();
  const [gameId, setGameId] = useState<GameId | null>(currentGame());
  const game = GAMES.find((entry) => entry.id === gameId) ?? null;
  const [toast, setToast] = useState<{ ending: Ending; id: number }>();
  // The last status seen for this chat, tracked during render (React's pattern for state that
  // follows a prop): a run ending while the panel is open raises the toast, a mount never does.
  const [seen, setSeen] = useState({ key: watchKey, status });
  if (seen.key !== watchKey || seen.status !== status) {
    setSeen({ key: watchKey, status });
    if (seen.key === watchKey && busy(seen.status) && !busy(status)) setToast({ ending: endingFor(status), id: Date.now() });
    else if (busy(status)) setToast(undefined);
  }

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(undefined), TOAST_MS);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const working = busy(status);
  const message = toast?.ending === "failed" ? `${agentName} hit an error` : toast?.ending === "stopped" ? `${agentName} stopped` : `${agentName} finished`;
  /** Closing the panel is a real close, not a view swap: the next open starts at the Arcade. */
  const close = () => { exitToArcade(); onClose(); };
  /** A game's "back to chat" leaves too — its run stays saved, the Arcade comes next. */
  const backToChat = () => { exitToArcade(); onBackToChat(); };
  const enter = (id: GameId) => { enterGame(id); setGameId(id); };
  const backToArcade = () => { exitToArcade(); setGameId(null); };

  const cards = reduce
    ? { initial: { opacity: 0 }, animate: { opacity: 1 }, exit: { opacity: 0 } }
    : { initial: { opacity: 0, y: 14 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 8 } };

  return (
    <div className="games-panel">
      <header className="panel-header">
        {game ? (
          <button type="button" className="icon-button" aria-label="All games" onClick={backToArcade}><Icon name="back" /></button>
        ) : (
          <span className="terminal-glyph" aria-hidden="true"><Icon name="gamepad" /></span>
        )}
        <div className="panel-title">
          <span className="panel-kicker">Arcade</span>
          <h3>{game ? game.name : "Games"}</h3>
        </div>
        {working && (
          <span className="terminal-pill running" role="status">
            <span className="terminal-pill-dot" aria-hidden="true" />
            {agentName} is working
          </span>
        )}
        <div className="panel-header-actions">
          <button type="button" className="icon-button" aria-label="Close Games panel" onClick={close}><Icon name="close" /></button>
        </div>
      </header>
      <div className="games-body">
        {game ? (
          <game.Component onLeave={backToChat} />
        ) : (
          <div className="games-arcade">
            {GAMES.map((entry, index) => {
              const best = bestScore(entry.id);
              const resumable = entry.hasSave();
              return (
                <motion.button
                  key={entry.id}
                  type="button"
                  className="game-card"
                  onClick={() => enter(entry.id)}
                  initial={cards.initial}
                  animate={cards.animate}
                  transition={{ duration: 0.26, delay: reduce ? 0 : 0.06 * index }}
                >
                  <span className={`game-card-poster ${entry.id}`} aria-hidden="true">
                    <DuckMark className="game-card-duck" />
                    <i className="marks" />
                  </span>
                  <span className="game-card-copy">
                    <strong>{entry.name}</strong>
                    <span>{entry.blurb}</span>
                    <span className="game-card-meta">
                      {best > 0 && <span className="game-best">Best {best.toLocaleString()}{entry.unit ? ` ${entry.unit}` : ""}</span>}
                      {resumable && (
                        <span className="game-card-resume">
                          <i aria-hidden="true" />
                          Run in progress
                        </span>
                      )}
                    </span>
                  </span>
                  <Icon name="chevron" />
                </motion.button>
              );
            })}
          </div>
        )}
        <AnimatePresence>
          {toast && (
            <motion.div
              key={toast.id}
              className={`games-toast ${toast.ending}`}
              role="status"
              initial={reduce ? { opacity: 0 } : { opacity: 0, y: 18, scale: 0.97 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }}
              transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 30 }}
            >
              <span className="games-toast-dot" aria-hidden="true" />
              <span className="games-toast-text">{message}</span>
              <button type="button" className="primary-button compact" onClick={() => { setToast(undefined); backToChat(); }}>Back to chat</button>
              <button type="button" className="icon-button" aria-label="Dismiss" onClick={() => setToast(undefined)}><Icon name="close" /></button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
});
