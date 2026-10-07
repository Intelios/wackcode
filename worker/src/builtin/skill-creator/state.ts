/**
 * Skill-creator session persistence, following the goal entry's pattern: one
 * `wackcode-skill-creator` custom entry per transition, the latest on the branch wins. A rewind
 * to before the draft removes the workflow; a restored draft keeps its identity so later turns
 * can still preview and the review card stays actionable.
 */
import type { SkillCreatorState } from "../../protocol.js";

export const SKILL_CREATOR_ENTRY_TYPE = "wackcode-skill-creator";
export const SKILL_CREATOR_STATE_VERSION = 1;

export interface PersistedSkillCreatorState {
  version: typeof SKILL_CREATOR_STATE_VERSION;
  draftId?: string;
  name?: string;
  revision?: string;
  /** "cleared" writes the workflow away without deleting anything on disk. */
  phase?: "active" | "cleared";
}

export function toPersistedSkillCreator(state: SkillCreatorState | undefined): PersistedSkillCreatorState {
  if (!state) return { version: SKILL_CREATOR_STATE_VERSION, phase: "cleared" };
  return { version: SKILL_CREATOR_STATE_VERSION, phase: "active", ...state };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Restore the workflow from the branch's latest skill-creator entry (last write wins). */
export function restoreSkillCreatorState(branch: Iterable<unknown>): SkillCreatorState | undefined {
  let data: unknown;
  for (const raw of branch) {
    const entry = raw as { type?: string; customType?: string; data?: unknown };
    if (entry?.type === "custom" && entry.customType === SKILL_CREATOR_ENTRY_TYPE && isRecord(entry.data)) {
      data = entry.data;
    }
  }
  if (!isRecord(data) || data.version !== SKILL_CREATOR_STATE_VERSION || data.phase === "cleared") return undefined;
  const draftId = typeof data.draftId === "string" ? data.draftId : "";
  const name = typeof data.name === "string" ? data.name : "";
  if (!draftId || !name) return undefined;
  const state: SkillCreatorState = { draftId, name };
  if (typeof data.revision === "string" && data.revision) state.revision = data.revision;
  return state;
}
