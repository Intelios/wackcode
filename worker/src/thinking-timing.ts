import { RUN_TIMING_ENTRY_TYPE, type ThinkingDurations } from "./run-timing.js";

/**
 * How long the model reasoned in each thinking block. Pi's thinking content carries no timing,
 * so the worker clocks every block of the streaming message from its `thinking_start` stream
 * event to its `thinking_end`, and keeps the result against the finished message object (the
 * one Pi stores on its session entry). The run's timing entry persists them by assistant entry
 * id, so they survive a restart and travel with a fork. Open start timestamps are live-only:
 * they let the renderer clock a block while it is being written, and are never persisted.
 */
export class ThinkingClock {
  /** Content index -> start time, for thinking blocks of the streaming message still being written. */
  private readonly open = new Map<number, number>();
  /** Content index -> duration, for thinking blocks of the streaming message that have ended. */
  private readonly closed = new Map<number, number>();
  private readonly finished = new WeakMap<object, ThinkingDurations>();
  /** Finished since the run's timing was last written. */
  private unsaved = new Map<object, ThinkingDurations>();

  /** An assistant message has started streaming. */
  begin(): void {
    this.open.clear();
    this.closed.clear();
  }

  /** One of Pi's `assistantMessageEvent`s for the streaming message. */
  update(event: unknown, now = Date.now()): void {
    if (!event || typeof event !== "object") return;
    const { type, contentIndex } = event as { type?: unknown; contentIndex?: unknown };
    if (typeof type !== "string" || typeof contentIndex !== "number") return;
    if (type === "thinking_start" || type === "thinking_delta") {
      // A delta without its start event still opens the block.
      if (!this.open.has(contentIndex) && !this.closed.has(contentIndex)) this.open.set(contentIndex, now);
    } else if (type === "thinking_end") {
      this.close(contentIndex, now);
    } else if (type.endsWith("_start")) {
      // Anything the model starts writing ends the reasoning before it, even without its end event.
      this.closeAll(now);
    }
  }

  /** The streaming message's durations so far; a block still being written has none yet. */
  live(message: unknown): ThinkingDurations {
    return ordinal(message, this.closed);
  }

  /** The streaming message's open start timestamps (epoch ms), in thinking-block order. */
  liveStarts(message: unknown): Array<number | null> {
    return ordinal(message, this.open);
  }

  /** The streaming message ended as `message`: every block still open ends now. */
  end(message: unknown, now = Date.now()): void {
    this.closeAll(now);
    if (message && typeof message === "object") {
      const durations = ordinal(message, this.closed);
      if (durations.some((duration) => duration !== null)) {
        this.finished.set(message, durations);
        this.unsaved.set(message, durations);
      }
    }
    this.begin();
  }

  /** A finished message's durations, if this worker streamed it. */
  durations(message: unknown): ThinkingDurations | undefined {
    return message && typeof message === "object" ? this.finished.get(message) : undefined;
  }

  /** Messages finished since the last call, keyed by the object Pi stores on the session entry. */
  takeUnsaved(): Map<object, ThinkingDurations> {
    const taken = this.unsaved;
    this.unsaved = new Map();
    return taken;
  }

  private close(contentIndex: number, now: number): void {
    const startedAt = this.open.get(contentIndex);
    if (startedAt === undefined) return;
    this.open.delete(contentIndex);
    this.closed.set(contentIndex, Math.max(0, now - startedAt));
  }

  private closeAll(now: number): void {
    for (const contentIndex of [...this.open.keys()]) this.close(contentIndex, now);
  }
}

/** Values by content index, as one entry per thinking block in content order. */
function ordinal(message: unknown, byIndex: Map<number, number>): Array<number | null> {
  const content = message && typeof message === "object" ? (message as { content?: unknown }).content : undefined;
  if (!Array.isArray(content)) return [];
  const values: Array<number | null> = [];
  content.forEach((block, index) => {
    if (block && typeof block === "object" && (block as { type?: unknown }).type === "thinking") {
      values.push(byIndex.get(index) ?? null);
    }
  });
  return values;
}

interface SessionEntryLike {
  type?: string;
  customType?: string;
  data?: unknown;
}

/** Saved thinking durations by assistant entry id, from the run-timing entries on a branch. */
export function resolveThinkingDurations(branch: unknown[]): Map<string, ThinkingDurations> {
  const result = new Map<string, ThinkingDurations>();
  for (const raw of branch) {
    if (!raw || typeof raw !== "object") continue;
    const entry = raw as SessionEntryLike;
    if (entry.type !== "custom" || entry.customType !== RUN_TIMING_ENTRY_TYPE) continue;
    const thinking = (entry.data as { thinking?: unknown } | undefined)?.thinking;
    if (!thinking || typeof thinking !== "object" || Array.isArray(thinking)) continue;
    for (const [entryId, durations] of Object.entries(thinking)) {
      if (isDurations(durations)) result.set(entryId, durations);
    }
  }
  return result;
}

function isDurations(value: unknown): value is ThinkingDurations {
  return Array.isArray(value)
    && value.every((duration) => duration === null || (typeof duration === "number" && Number.isFinite(duration) && duration >= 0));
}
