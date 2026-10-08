/**
 * Games in progress, kept outside React. The side panel remounts its page on every view swap and
 * on every chat-tab switch, so a run held in component state would be lost the moment you peek
 * at the chat. A game saves its live state here and picks it back up, paused, when it mounts.
 * In memory only: a run never outlives the app.
 */
export type GameId = "quack";

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
