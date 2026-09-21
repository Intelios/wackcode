/**
 * Plan-mode session persistence. Simplified port of `@narumitw/pi-plan-mode` v0.58.3 (MIT)
 * `state.ts`: WackCode only needs mode + ready plan, not upstream's saved/active/fresh-
 * implementation bookkeeping.
 */
import { PLAN_MODE_COMPLETE_TOOL_NAME, normalizePlanModeCompletion, planFromCompletionDetails } from "./completion.js";

export const PLAN_STATE_ENTRY_TYPE = "wackcode-plan-state";
export const PLAN_STATE_VERSION = 1;

export interface PersistedPlanState {
  version: typeof PLAN_STATE_VERSION;
  enabled: boolean;
  /** Completed plan awaiting user action (the "ready" phase). */
  plan?: string;
}

export interface RestoredPlanState {
  enabled: boolean;
  plan?: string;
}

type SessionEntry = {
  type?: string;
  customType?: string;
  data?: unknown;
  message?: {
    role?: string;
    toolName?: string;
    details?: unknown;
  };
};

/** Restore mode + ready plan from the session branch's latest state entry. */
export function restorePlanState(branch: unknown[]): RestoredPlanState {
  const entries = branch as SessionEntry[];
  let stateIndex = -1;
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const candidate = entries[index];
    if (candidate?.type === "custom" && candidate.customType === PLAN_STATE_ENTRY_TYPE) {
      stateIndex = index;
      break;
    }
  }
  const entry = entries[stateIndex];
  if (!isRecord(entry?.data)) return { enabled: false };

  const enabled = entry.data.enabled === true;
  const persistedPlan = enabled ? normalizePersistedPlan(entry.data.plan) : undefined;
  // A crash between completion and state write loses `plan`; the completion's own tool
  // result is still on the branch, so recover from it like upstream does.
  const plan = persistedPlan ?? (enabled ? latestCompletionPlan(entries.slice(stateIndex + 1)) : undefined);
  return { enabled, plan };
}

function normalizePersistedPlan(value: unknown) {
  const normalized = normalizePlanModeCompletion({ plan: value });
  return normalized.ok ? normalized.plan : undefined;
}

function latestCompletionPlan(entries: SessionEntry[]) {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const message = entries[index]?.message;
    if (message?.role !== "toolResult" || message.toolName !== PLAN_MODE_COMPLETE_TOOL_NAME) {
      continue;
    }
    const plan = planFromCompletionDetails(message.details);
    if (plan) return plan;
  }
  return undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
