/** Shared transcript annotations and identity cache for live and saved sessions. */
import type { CheckpointRef, CommandPresentation, MessageVersions, NormalizedMessage, TurnInfo } from "./protocol.js";
import type { ThinkingDurations } from "./run-timing.js";
import { checkpointBefore, commandPresentationBefore, turnsOnPath, versionsOf, type EntryLike, type TreeIndex } from "./tree.js";
import { sameCheckpoint, sameCommandPresentation, sameTurn, sameVersions } from "./delta.js";
import type { CompactionEntry, ContextEditEntry, SessionEntry } from "@earendil-works/pi-coding-agent";

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
  contextEdit?: ContextEditEntry;
}

interface TranscriptEntry {
  raw: unknown;
  cacheKey: object;
  entryId: string;
  contextEdit?: ContextEditEntry;
  compaction?: CompactionEntry;
}

/**
 * Display the current branch's history independently of Pi's model-context cut. Context edits
 * still replace/omit messages (including failed retry attempts); sibling branches stay hidden.
 * Keep the original object as the cache key when an edit replaces its content.
 */
export function transcriptEntries(path: EntryLike[]): TranscriptEntry[] {
  const entries = path as SessionEntry[];
  const edits = new Map(entries.flatMap((entry) => entry.type === "context_edit" ? [[entry.targetId, entry] as const] : []));
  return entries.flatMap<TranscriptEntry>((entry) => {
    if (entry.type === "compaction") return [{ raw: entry, cacheKey: entry, entryId: entry.id, compaction: entry }];
    if (entry.type !== "message" || !entry.message || typeof entry.message !== "object") return [];
    const contextEdit = edits.get(entry.id);
    if (contextEdit?.replacement === null) return [];
    let raw = entry.message;
    if (contextEdit?.replacement && ["user", "assistant", "toolResult"].includes(raw.role)) {
      const replacement = contextEdit.replacement.content;
      const content = (raw.role === "assistant" || raw.role === "toolResult") && typeof replacement === "string"
        ? [{ type: "text" as const, text: replacement }] : replacement;
      raw = { ...raw, content } as typeof raw;
    }
    return [{ raw, cacheKey: entry.message, entryId: entry.id, contextEdit }];
  });
}

/** The roles that reach the transcript; mirrors the gate at the top of normalizeMessage. */
function visibleRole(raw: unknown): NormalizedMessage["role"] | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const rawRole = String((raw as Record<string, unknown>).role ?? "system");
  const role = rawRole === "toolResult" ? "tool" : rawRole;
  return role === "user" || role === "assistant" || role === "tool" || role === "system" ? role : undefined;
}

/**
 * The transcript is the saved branch, including messages summarized away from model context
 * and explicit compaction boundaries. The runtime list contributes only an unsaved streaming
 * message; it must never replace the display history with the model's shortened context.
 *
 * Each message's normalized form is cached by its raw object and reused verbatim while the
 * entry id, position, and derived tree annotations hold steady, so unchanged messages keep
 * their object identity across emissions and the diff against the last sent state is
 * O(changes). Cached objects are never mutated; any change rebuilds from scratch. The streaming
 * row is never cached, so neither its unfinished content nor live thinking starts survive it.
 */
export function transcriptMessages(rawMessages: unknown[], path: EntryLike[], index: TreeIndex, thinking: Map<string, ThinkingDurations>, options: {
  normalize: (raw: unknown, position: number, thinking?: ThinkingDurations, starts?: Array<number | null>, cacheKey?: object) => NormalizedMessage | undefined;
  cache: WeakMap<object, CachedMessage>;
  streamingMessage?: unknown;
  durations?: (raw: unknown) => ThinkingDurations | undefined;
  /** Live-only thinking starts: called solely for `streamingMessage`, never an older row. */
  starts?: (raw: unknown) => Array<number | null> | undefined;
  /** Estimate from Pi's model projection AT this compaction, computed only on a cache miss. */
  compactionTokensAfter?: (entry: CompactionEntry) => number | undefined;
}): NormalizedMessage[] {
  const { cache: normalizedCache, streamingMessage, normalize: normalizeMessage } = options;
  const sources: Array<{ raw: unknown; cacheKey: object; entryId?: string; contextEdit?: ContextEditEntry; compaction?: CompactionEntry }> = transcriptEntries(path);
  if (streamingMessage && typeof streamingMessage === "object" && rawMessages.includes(streamingMessage)
    && !sources.some((source) => source.cacheKey === streamingMessage)) {
    sources.push({ raw: streamingMessage, cacheKey: streamingMessage, entryId: undefined });
  }
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
    cacheKey: object;
    contextEdit?: ContextEditEntry;
    compaction?: CompactionEntry;
  }
  const rows: Row[] = [];
  let currentUser: string | undefined;
  sources.forEach((source, position) => {
    const { raw, entryId } = source;
    const role = source.compaction ? "system" : visibleRole(raw);
    if (!role) return;
    const row: Row = { ...source, entryId, position, role, userEntryId: undefined, versions: undefined, checkpoint: undefined, commandPresentation: undefined, turn: undefined };
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
    const isStreaming = row.raw === streamingMessage;
    const cached = isStreaming ? undefined : normalizedCache.get(row.cacheKey);
    if (
      cached
      && cached.entryId === row.entryId
      && cached.contextEdit === row.contextEdit
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
    const starts = isStreaming ? options.starts?.(row.raw) : undefined;
    const estimatedTokensAfter = row.compaction ? options.compactionTokensAfter?.(row.compaction) : undefined;
    const message = row.compaction ? {
      id: row.compaction.id, role: "system" as const, blocks: [],
      timestamp: new Date(row.compaction.timestamp).getTime(),
      compaction: {
        summary: row.compaction.summary, tokensBefore: row.compaction.tokensBefore,
        ...(estimatedTokensAfter !== undefined ? { estimatedTokensAfter } : {})
      }
    } : normalizeMessage(row.raw, row.position, durations, starts, row.cacheKey);
    if (!message) return null;
    if (row.entryId) {
      message.entryId = row.entryId;
      message.id = row.entryId;
    }
    if (row.versions) message.versions = row.versions;
    if (row.checkpoint) message.checkpoint = row.checkpoint;
    if (row.commandPresentation) message.commandPresentation = row.commandPresentation;
    if (row.turn) message.turn = row.turn;
    if (!isStreaming) {
      normalizedCache.set(row.cacheKey, {
        message,
        entryId: row.entryId,
        position: row.position,
        versions: row.versions,
        checkpoint: row.checkpoint,
        commandPresentation: row.commandPresentation,
        turn: row.turn,
        contextEdit: row.contextEdit
      });
    }
    return message;
  }).filter((message): message is NormalizedMessage => message !== null);
}
