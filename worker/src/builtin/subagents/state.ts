/**
 * Durable background cards are branch-scoped custom entries. The original tool result stays
 * immutable; live/cold projections overlay the newest saved card. Transcripts never ride a
 * snapshot. Live job IDs are not restored: unfinished saved cards become interrupted.
 */
import type { NormalizedMessage, SubagentDetails } from "../../protocol.js";
import { withoutTranscripts } from "./details.js";

export const BACKGROUND_SUBAGENT_ENTRY = "wackcode-subagent-background";
export const BACKGROUND_SUBAGENT_MESSAGE = "wackcode-subagent-results";
export interface BackgroundSubagentCall { v: 1; toolCallId: string; details: SubagentDetails }

export function backgroundCalls(entries: readonly unknown[]): Map<string, BackgroundSubagentCall> {
  const calls = new Map<string, BackgroundSubagentCall>();
  for (const entry of entries) {
    const value = entry as { type?: unknown; customType?: unknown; data?: BackgroundSubagentCall };
    if (value?.type !== "custom" || value.customType !== BACKGROUND_SUBAGENT_ENTRY) continue;
    const data = value.data;
    if (data?.v !== 1 || typeof data.toolCallId !== "string" || data.details?.v !== 1 || !Array.isArray(data.details.results)) continue;
    calls.set(data.toolCallId, data);
  }
  return calls;
}

export function interruptedDetails(details: SubagentDetails): SubagentDetails {
  if (!details.results.some((result) => result.status === "running" || result.status === "queued")) return details;
  const cached = interruptions.get(details);
  if (cached) return cached;
  const interrupted: SubagentDetails = { ...details, results: details.results.map((result) => result.status === "running" || result.status === "queued"
    ? { ...result, status: "interrupted", error: "The worker stopped before this sub-agent finished. It was not restarted." }
    : result) };
  interruptions.set(details, interrupted);
  return interrupted;
}
const interruptions = new WeakMap<SubagentDetails, SubagentDetails>();

/** Preserve row identities until the source card changes, just like transcript normalization. */
export function projectBackgroundCards(messages: NormalizedMessage[], calls: Map<string, BackgroundSubagentCall>, cache: WeakMap<object, { details: SubagentDetails; message: NormalizedMessage }>): NormalizedMessage[] {
  return messages.map((message) => {
    const result = message.blocks.find((block) => block.type === "tool-result" && block.toolName === "subagent" && block.toolCallId && calls.has(block.toolCallId));
    const details = result?.toolCallId ? calls.get(result.toolCallId)?.details : undefined;
    if (!details) return message;
    const cached = cache.get(message);
    if (cached?.details === details) return cached.message;
    const projected = { ...message, blocks: message.blocks.map((block) => block === result ? { ...block, details: withoutTranscripts(details) } : block) };
    cache.set(message, { details, message: projected });
    return projected;
  });
}
