import type { GameId } from "./session";

/**
 * One best score per game, per machine. A convenience: unreadable storage just reads as 0.
 * A renamed game keeps the old best once: reading `quack` falls back to the pre-rename `swarm`
 * key, and the next record rewrites it under the new name.
 */
export const BEST_SCORES_KEY = "wackcode:gameBest";

/** Where a game's best lived before it was renamed, so history isn't thrown away. */
const LEGACY_IDS: Partial<Record<GameId, string>> = { quack: "swarm" };

function read(): Partial<Record<GameId, number>> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(BEST_SCORES_KEY) ?? "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed as Partial<Record<GameId, number>> : {};
  } catch {
    return {};
  }
}

export function bestScore(id: GameId): number {
  const scores = read();
  const legacy = LEGACY_IDS[id] ? (scores as Record<string, unknown>)[LEGACY_IDS[id]!] : undefined;
  const value = scores[id] ?? legacy;
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
