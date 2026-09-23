import type { RunTiming } from "./protocol.js";

export const RUN_TIMING_ENTRY_TYPE = "wackcode-run-timing";
export const RUN_TIMING_VERSION = 1;

/** Each thinking block's duration in content order; null where a block was never clocked. */
export type ThinkingDurations = Array<number | null>;

export interface PersistedRunTiming {
  version: typeof RUN_TIMING_VERSION;
  runId: string;
  userMessageEntryId: string;
  startedAt: number;
  endedAt: number;
  durationMs: number;
  /** How long the run's assistant messages reasoned, by entry id. Absent when none did. */
  thinking?: Record<string, ThinkingDurations>;
}

interface SessionEntryLike {
  type?: string;
  id?: string;
  customType?: string;
  data?: unknown;
}

/** Map timing records to user messages still visible on Pi's active, compaction-aware branch. */
export function resolveRunTimings(
  branch: unknown[],
  visibleUserEntryIds: string[],
  visibleUserMessageIds: string[]
): RunTiming[] {
  const visibleEntries = new Map<string, string>();
  const count = Math.min(visibleUserEntryIds.length, visibleUserMessageIds.length);
  for (let index = 0; index < count; index += 1) {
    visibleEntries.set(visibleUserEntryIds[index], visibleUserMessageIds[index]);
  }

  const result: RunTiming[] = [];
  const usedMessages = new Set<string>();
  for (const raw of branch) {
    if (!raw || typeof raw !== "object") continue;
    const entry = raw as SessionEntryLike;
    if (entry.type !== "custom" || entry.customType !== RUN_TIMING_ENTRY_TYPE || !isTiming(entry.data)) continue;
    const userMessageId = visibleEntries.get(entry.data.userMessageEntryId);
    if (!userMessageId || usedMessages.has(userMessageId)) continue;
    usedMessages.add(userMessageId);
    result.push({ userMessageId, durationMs: entry.data.durationMs });
  }
  return result;
}

function isTiming(value: unknown): value is PersistedRunTiming {
  if (!value || typeof value !== "object") return false;
  const data = value as Partial<PersistedRunTiming>;
  return data.version === RUN_TIMING_VERSION
    && typeof data.runId === "string"
    && typeof data.userMessageEntryId === "string"
    && Number.isFinite(data.startedAt)
    && Number.isFinite(data.endedAt)
    && Number.isFinite(data.durationMs)
    && data.durationMs! >= 0;
}
