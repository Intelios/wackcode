import { memo, useEffect, useState, type ComponentType } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { GameId } from "../games/session";
import type { TaskStatus } from "../types";
import { Icon } from "./Icons";
import { SwarmGame } from "./SwarmGame";

const TOAST_MS = 9000;

/** The arcade's cabinet. A new game is an entry here and its own module under `src/games/`. */
const GAMES: { id: GameId; name: string; Component: ComponentType }[] = [
  { id: "swarm", name: "Swarm", Component: SwarmGame }
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
 * The side panel's Games view: something small to play while a slow model works. The panel
 * itself is a thin shell (header, the game, the run-ended toast); each game owns its canvas and
 * keeps its run in `games/session.ts`, so leaving the panel pauses rather than loses it.
 *
 * Memo'd: `App` re-renders on every streaming delta, and nothing here depends on the stream.
 */
export const GamesPanel = memo(function GamesPanel({ status, watchKey, agentName, onClose, onBackToChat }: GamesPanelProps) {
  const reduce = useReducedMotion();
  const game = GAMES[0];
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

  return (
    <div className="games-panel">
      <header className="panel-header">
        <span className="terminal-glyph" aria-hidden="true"><Icon name="gamepad" /></span>
        <div className="panel-title"><span className="panel-kicker">Arcade</span><h3>{game.name}</h3></div>
        {working && (
          <span className="terminal-pill running" role="status">
            <span className="terminal-pill-dot" aria-hidden="true" />
            {agentName} is working
          </span>
        )}
        <div className="panel-header-actions">
          <button type="button" className="icon-button" aria-label="Close Games panel" onClick={onClose}><Icon name="close" /></button>
        </div>
      </header>
      <div className="games-body">
        <game.Component />
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
              <button type="button" className="primary-button compact" onClick={() => { setToast(undefined); onBackToChat(); }}>Back to chat</button>
              <button type="button" className="icon-button" aria-label="Dismiss" onClick={() => setToast(undefined)}><Icon name="close" /></button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  );
});
