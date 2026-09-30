/** Shared transcript annotations and identity cache for live and saved sessions. */
import type { CheckpointRef, CommandPresentation, MessageVersions, NormalizedMessage, TurnInfo } from "./protocol.js";
import type { ThinkingDurations } from "./run-timing.js";
import { checkpointBefore, commandPresentationBefore, turnsOnPath, versionsOf, type EntryLike, type TreeIndex } from "./tree.js";
import { sameCheckpoint, sameCommandPresentation, sameTurn, sameVersions } from "./delta.js";

export interface CachedMessage {
  message: NormalizedMessage;
  /** The entry id the message was normalized under; a change forces a rebuild. */
  entryId: string | undefined;
  /** Positions back the positional id used while the entry id is still unknown. */
  position: number;
  versions: MessageVersions | undefined;
  checkpoint: CheckpointRef | undefined;
  commandPresentation: CommandPresentation | undefined;
  turn: TurnInfo | undefined;
}


/** The roles that reach the transcript; mirrors the gate at the top of normalizeMessage. */
function visibleRole(raw: unknown): NormalizedMessage["role"] | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const rawRole = String((raw as Record<string, unknown>).role ?? "system");
  const role = rawRole === "toolResult" ? "tool" : rawRole;
  return role === "user" || role === "assistant" || role === "tool" || role === "system" ? role : undefined;
}

/**
 * The transcript is `session.messages` (what the model sees). Pi stores those same message
 * objects on its session entries, so each one maps to its entry by identity, the same way image
 * previews are keyed. A message that has just finished can be in state a moment before Pi saves
 * it; it goes without an entry id until the next snapshot.
 *
 * Each message's normalized form is cached by its raw object and reused verbatim while the
 * entry id, position, and derived tree annotations hold steady, so unchanged messages keep
 * their object identity across emissions and the diff against the last sent state is
 * O(changes). Cached objects are never mutated; any change rebuilds from scratch.
 */
export function transcriptMessages(rawMessages: unknown[], path: EntryLike[], index: TreeIndex, thinking: Map<string, ThinkingDurations>, options: {
  normalize: (raw: unknown, position: number, thinking?: ThinkingDurations) => NormalizedMessage | undefined;
  cache: WeakMap<object, CachedMessage>;
  streamingMessage?: unknown;
  durations?: (raw: unknown) => ThinkingDurations | undefined;
}): NormalizedMessage[] {
  const { cache: normalizedCache, streamingMessage, normalize: normalizeMessage } = options;
  const entryIds = new Map<unknown, string>();
  for (const entry of path) if (entry.type === "message") entryIds.set(entry.message, entry.id);
  const turns = turnsOnPath(path);

  interface Row {
    raw: unknown;
    position: number;
    entryId: string | undefined;
    role: NormalizedMessage["role"];
    /** Assistant messages: the user entry id their turn answers to. */
    userEntryId: string | undefined;
    versions: MessageVersions | undefined;
    checkpoint: CheckpointRef | undefined;
    commandPresentation: CommandPresentation | undefined;
    turn: TurnInfo | undefined;
  }
  const rows: Row[] = [];
  let currentUser: string | undefined;
  rawMessages.forEach((raw, position) => {
    const role = visibleRole(raw);
    if (!role) return;
    const entryId = entryIds.get(raw);
    const row: Row = { raw, position, entryId, role, userEntryId: undefined, versions: undefined, checkpoint: undefined, commandPresentation: undefined, turn: undefined };
    if (role === "user") {
      currentUser = entryId;
      if (entryId) {
        const versions = versionsOf(index, entryId);
        if (versions && versions.total > 1) row.versions = versions;
        const checkpoint = checkpointBefore(index, entryId);
        if (checkpoint) row.checkpoint = checkpoint;
        row.commandPresentation = commandPresentationBefore(index, entryId);
      }
    } else if (role === "assistant") {
      row.userEntryId = currentUser;
    }
    rows.push(row);
  });
  // Only the last assistant message of a turn carries the turn, and a newer answer strips it
  // from the previous one, so this is decided per emission rather than cached per message.
  const seenTurns = new Set<string>();
  for (let i = rows.length - 1; i >= 0; i--) {
    const row = rows[i];
    if (row.role === "assistant" && row.userEntryId && !seenTurns.has(row.userEntryId)) {
      seenTurns.add(row.userEntryId);
      row.turn = turns.get(row.userEntryId);
    }
  }

  return rows.map((row): NormalizedMessage | null => {
    const cached = row.raw === streamingMessage ? undefined : normalizedCache.get(row.raw as object);
    if (
      cached
      && cached.entryId === row.entryId
      && (row.entryId !== undefined || cached.position === row.position)
      && sameVersions(cached.versions, row.versions)
      && sameCheckpoint(cached.checkpoint, row.checkpoint)
      && sameCommandPresentation(cached.commandPresentation, row.commandPresentation)
      && sameTurn(cached.turn, row.turn)
    ) {
      return cached.message;
    }
    // Clocked by this worker, or saved with an earlier run.
    const durations = options.durations?.(row.raw) ?? (row.entryId ? thinking.get(row.entryId) : undefined);
    const message = normalizeMessage(row.raw, row.position, durations);
    if (!message) return null;
    if (row.entryId) {
      message.entryId = row.entryId;
      message.id = row.entryId;
    }
    if (row.versions) message.versions = row.versions;
    if (row.checkpoint) message.checkpoint = row.checkpoint;
    if (row.commandPresentation) message.commandPresentation = row.commandPresentation;
    if (row.turn) message.turn = row.turn;
    normalizedCache.set(row.raw as object, {
      message,
      entryId: row.entryId,
      position: row.position,
      versions: row.versions,
      checkpoint: row.checkpoint,
      commandPresentation: row.commandPresentation,
      turn: row.turn
    });
    return message;
  }).filter((message): message is NormalizedMessage => message !== null);
}

