/**
 * Games in progress, kept outside React. The side panel remounts its page on every view swap and
 * on every chat-tab switch, so a run held in component state would be lost the moment you peek
 * at the chat. A game saves its live state here and picks it back up, paused, when it mounts.
 * In memory only: a run never outlives the app (the per-game save modules cover that).
 *
 * The same store holds which game the Games panel has entered from its Arcade home, so a view
 * swap comes back to that game rather than the picker. Closing the panel resets it (`exitToArcade`
 * from the panel's own close paths), so every fresh open of Games starts at the Arcade.
 */
export type GameId = "quack" | "doodle";

const runs = new Map<GameId, unknown>();

export function savedRun<T>(id: GameId): T | undefined {
  return runs.get(id) as T | undefined;
}

export function saveRun(id: GameId, state: unknown): void {
  runs.set(id, state);
}

export function clearRun(id: GameId): void {
  runs.delete(id);
}

// ── The panel's open game ────────────────────────────

let open: GameId | null = null;

/** The game the panel has entered from the Arcade home, or null to show the picker. */
export function currentGame(): GameId | null {
  return open;
}

/** Entering a game from the Arcade home. */
export function enterGame(id: GameId): void {
  open = id;
}

/** Back at the Arcade home: the next open of the Games panel starts at the picker. */
export function exitToArcade(): void {
  open = null;
}
