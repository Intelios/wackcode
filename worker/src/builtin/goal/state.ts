/**
 * Goal-loop session persistence. One `wackcode-goal` custom entry carries the whole state and
 * is appended at every transition, so the latest entry on the branch wins — the same pattern
 * plan-mode uses. A rewind or compaction that drops the last entry simply restores an earlier
 * phase; a restored "active"/"verifying" phase becomes "paused" because the run it belonged
 * to is gone and only the user can resume it.
 */
import type { GoalState } from "../../protocol.js";

export const GOAL_ENTRY_TYPE = "wackcode-goal";
export const GOAL_STATE_VERSION = 1;

export const DEFAULT_MAX_ITERATIONS = 25;
/** Consecutive rounds with no detected progress before the loop pauses itself. */
export const NO_PROGRESS_LIMIT = 3;

export interface PersistedGoalState {
  version: typeof GOAL_STATE_VERSION;
  objective?: string;
  phase?: GoalState["phase"] | "cleared";
  iteration?: number;
  maxIterations?: number;
  noProgress?: number;
  lastReason?: string;
  lastNextAction?: string;
  note?: string;
}

export function toPersistedGoal(state: GoalState | undefined): PersistedGoalState {
  if (!state) return { version: GOAL_STATE_VERSION, phase: "cleared" };
  return { version: GOAL_STATE_VERSION, ...state };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const PHASES: readonly GoalState["phase"][] = ["active", "verifying", "paused", "complete", "stopped"];

/** Restore the goal state from the session branch's latest goal entry (last write wins). */
export function restoreGoalState(branch: Iterable<unknown>): GoalState | undefined {
  let data: unknown;
  for (const raw of branch) {
    const entry = raw as { type?: string; customType?: string; data?: unknown };
    if (entry?.type === "custom" && entry.customType === GOAL_ENTRY_TYPE && isRecord(entry.data)) {
      data = entry.data;
    }
  }
  if (!isRecord(data) || data.version !== GOAL_STATE_VERSION || typeof data.objective !== "string" || !data.objective) {
    return undefined;
  }
  const phase = PHASES.find((candidate) => candidate === data.phase);
  if (!phase) return undefined;
  const state: GoalState = {
    objective: data.objective,
    // A loop restored without its run can only wait for the user.
    phase: phase === "active" || phase === "verifying" ? "paused" : phase,
    iteration: typeof data.iteration === "number" ? data.iteration : 0,
    maxIterations: typeof data.maxIterations === "number" ? data.maxIterations : DEFAULT_MAX_ITERATIONS,
    noProgress: typeof data.noProgress === "number" ? data.noProgress : 0,
  };
  if (typeof data.lastReason === "string" && data.lastReason) state.lastReason = data.lastReason;
  if (typeof data.lastNextAction === "string" && data.lastNextAction) state.lastNextAction = data.lastNextAction;
  if (typeof data.note === "string" && data.note) state.note = data.note;
  if (state.phase === "paused" && !state.note) state.note = "Paused when the chat reloaded.";
  return state;
}
