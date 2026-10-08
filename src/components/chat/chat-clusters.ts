/**
 * Chat mode's conversation layout, as pure functions. The Chat area reads a transcript as a
 * messenger does: runs of one speaker are a cluster, and a quiet time divider opens each
 * stretch of conversation. Nothing here changes the normalized messages or their identity.
 *
 * - Assistant and tool messages between two user messages form one reply (the agent
 *   "speaking"), whatever their role split or how long its tools ran.
 * - `first` marks the start of a speaker's cluster: a change of speaker or `GAP_MS` of silence.
 *   Only a cluster's first bubble carries the agent's name.
 * - A time divider precedes the first entry and any entry `DIVIDER_GAP_MS` after the last.
 * - Compaction boundaries and system notices stand alone as their own entries.
 */
import type { NormalizedMessage } from "../../types";

/** A new cluster (and a time divider) after this much silence. */
export const GAP_MS = 3 * 60 * 1000;
/** A time divider only when the conversation resumes after a real pause. */
export const DIVIDER_GAP_MS = 30 * 60 * 1000;

export type ChatEntry =
  | { type: "divider"; key: string; at: number }
  | { type: "user"; key: string; message: NormalizedMessage; first: boolean; index: number }
  /** One reply: every assistant/tool message answering a user message. */
  | { type: "reply"; key: string; messages: NormalizedMessage[]; first: boolean }
  | { type: "compaction"; key: string; message: NormalizedMessage }
  | { type: "system"; key: string; message: NormalizedMessage };

function at(message: NormalizedMessage): number | undefined {
  return typeof message.timestamp === "number" ? message.timestamp : undefined;
}

/** Builds the Chat transcript's rows from the transcript's messages (pending echo included). */
export function clusterMessages(messages: NormalizedMessage[]): ChatEntry[] {
  const entries: ChatEntry[] = [];
  let last: number | undefined;
  let lastSpeaker: "user" | "agent" | undefined;
  let reply: Extract<ChatEntry, { type: "reply" }> | undefined;
  /** Replies are keyed by the user message they answer, so a streamed reply keeps its rows
   *  when its partial becomes the saved message (whose id differs). */
  let lastUserKey: string | undefined;
  const replyKeys = new Set<string>();

  const stamp = (message: NormalizedMessage) => {
    const time = at(message);
    if (time === undefined) return false;
    const gap = last === undefined ? Infinity : time - last;
    if (gap > DIVIDER_GAP_MS) entries.push({ type: "divider", key: `divider:${message.id}`, at: time });
    const broke = gap > GAP_MS;
    last = time;
    return broke;
  };

  messages.forEach((message, index) => {
    if (message.compaction) {
      reply = undefined;
      lastSpeaker = undefined;
      entries.push({ type: "compaction", key: `compaction:${message.id}`, message });
      return;
    }
    if (message.role === "system") {
      reply = undefined;
      entries.push({ type: "system", key: `system:${message.id}`, message });
      return;
    }
    // A reply stays one bubble group for its whole turn, however long its tools ran.
    if (reply && message.role !== "user") {
      reply.messages.push(message);
      last = at(message) ?? last;
      return;
    }
    const broke = stamp(message);
    if (message.role === "user") {
      reply = undefined;
      lastUserKey = `user:${message.versions?.group ?? message.id}`;
      entries.push({ type: "user", key: lastUserKey, message, first: broke || lastSpeaker !== "user", index });
      lastSpeaker = "user";
      return;
    }
    const candidate = lastUserKey ? `reply-to:${lastUserKey}` : `reply:${message.id}`;
    const key = replyKeys.has(candidate) ? `reply:${message.id}` : candidate;
    replyKeys.add(key);
    reply = { type: "reply", key, messages: [message], first: broke || lastSpeaker !== "agent" };
    entries.push(reply);
    lastSpeaker = "agent";
  });
  return entries;
}

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfDay(time: number): number {
  const date = new Date(time);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** "Today · 14:02", "Yesterday · 09:15", or "Mon 3 Mar · 18:40" for anything older. */
export function dividerLabel(time: number, now = Date.now()): string {
  const clock = new Date(time).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const days = Math.round((startOfDay(now) - startOfDay(time)) / DAY_MS);
  if (days <= 0) return `Today · ${clock}`;
  if (days === 1) return `Yesterday · ${clock}`;
  const date = new Date(time).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  return `${date} · ${clock}`;
}
