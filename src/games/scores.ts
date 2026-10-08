import type { GameId } from "./session";

/** One best score per game, per machine. A convenience: unreadable storage just reads as 0. */
export const BEST_SCORES_KEY = "wackcode:gameBest";

function read(): Partial<Record<GameId, number>> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(BEST_SCORES_KEY) ?? "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed as Partial<Record<GameId, number>> : {};
  } catch {
    return {};
  }
}

export function bestScore(id: GameId): number {
  const value = read()[id];
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

/** Records a finished run; `isBest` is true only when it beat the previous best. */
export function recordScore(id: GameId, score: number): { best: number; isBest: boolean } {
  const previous = bestScore(id);
  if (score <= previous) return { best: previous, isBest: false };
  try {
    localStorage.setItem(BEST_SCORES_KEY, JSON.stringify({ ...read(), [id]: score }));
  } catch {
    // Storage is a convenience; the run's result still shows.
  }
  return { best: score, isBest: true };
}
