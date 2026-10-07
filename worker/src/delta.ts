import type {
  CheckpointRef,
  CommandPresentation,
  GoalState,
  MessageVersions,
  ModelSwitch,
  NormalizedMessage,
  PlanState,
  RunTiming,
  SessionSnapshot,
  SkillCreatorState,
  TodoState,
  TurnInfo
} from "./protocol.js";

/**
 * Incremental snapshot plumbing. The worker diffs two message lists by object identity —
 * unchanged messages reuse their normalized object — and sends only what moved. A delta is an
 * optimization over full snapshots, never a correctness requirement: when the diff cannot be
 * expressed as "remove these ids, then replace-or-append these messages", `diffMessages`
 * returns undefined and the caller sends a full snapshot instead.
 */

export interface MessagesDiff {
  upserts: NormalizedMessage[];
  removed: string[];
}

function sameNumberList(a: number[] | undefined, b: number[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((value, index) => value === b[index]);
}

export function samePlanState(a: PlanState | undefined, b: PlanState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.mode === b.mode && a.phase === b.phase && a.plan === b.plan;
}

export function sameTodoState(a: TodoState | undefined, b: TodoState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b || a.tasks.length !== b.tasks.length) return false;
  return a.tasks.every((task, index) => {
    const other = b.tasks[index];
    return task.id === other.id
      && task.subject === other.subject
      && task.description === other.description
      && task.activeForm === other.activeForm
      && task.status === other.status
      && sameNumberList(task.blockedBy, other.blockedBy);
  });
}

export function sameGoalState(a: GoalState | undefined, b: GoalState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.objective === b.objective
    && a.phase === b.phase
    && a.iteration === b.iteration
    && a.maxIterations === b.maxIterations
    && a.noProgress === b.noProgress
    && a.lastReason === b.lastReason
    && a.lastNextAction === b.lastNextAction
    && a.note === b.note;
}

export function sameSkillCreatorState(a: SkillCreatorState | undefined, b: SkillCreatorState | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.draftId === b.draftId && a.name === b.name && a.revision === b.revision;
}

export function sameVersions(a: MessageVersions | undefined, b: MessageVersions | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.index === b.index && a.total === b.total && a.previous === b.previous && a.next === b.next && a.group === b.group;
}

export function sameCheckpoint(a: CheckpointRef | undefined, b: CheckpointRef | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.id === b.id && a.head === b.head;
}

export function sameCommandPresentation(a: CommandPresentation | undefined, b: CommandPresentation | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.id === b.id && a.name === b.name && a.arguments === b.arguments && a.kind === b.kind
    && a.round === b.round && a.nextAction === b.nextAction;
}

export function sameTurn(a: TurnInfo | undefined, b: TurnInfo | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return a.userEntryId === b.userEntryId && a.endEntryId === b.endEntryId && sameCheckpoint(a.after, b.after);
}

export function sameRunTimings(a: RunTiming[], b: RunTiming[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  return a.every((timing, index) => timing.userMessageId === b[index].userMessageId && timing.durationMs === b[index].durationMs);
}

export function sameModelSwitches(a: ModelSwitch[], b: ModelSwitch[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  return a.every((entry, index) => {
    const other = b[index];
    return entry.id === other.id && entry.at === other.at
      && entry.from.providerId === other.from.providerId && entry.from.modelId === other.from.modelId
      && entry.to.providerId === other.to.providerId && entry.to.modelId === other.to.modelId;
  });
}

export function sameStats(a: SessionSnapshot["stats"], b: SessionSnapshot["stats"]): boolean {
  return a.cost === b.cost
    && a.tokens.input === b.tokens.input
    && a.tokens.output === b.tokens.output
    && a.tokens.cacheRead === b.tokens.cacheRead
    && a.tokens.cacheWrite === b.tokens.cacheWrite
    && a.tokens.total === b.tokens.total
    && a.contextUsage?.tokens === b.contextUsage?.tokens
    && a.contextUsage?.contextWindow === b.contextUsage?.contextWindow
    && a.contextUsage?.percent === b.contextUsage?.percent
    && a.contextBreakdown?.cacheHitRate === b.contextBreakdown?.cacheHitRate
    && a.contextBreakdown?.entries.length === b.contextBreakdown?.entries.length
    && (a.contextBreakdown?.entries ?? []).every((entry, index) =>
      entry.id === b.contextBreakdown?.entries[index]?.id && entry.tokens === b.contextBreakdown?.entries[index]?.tokens);
}

export function sameTree(a: SessionSnapshot["tree"], b: SessionSnapshot["tree"]): boolean {
  return a.leafId === b.leafId && a.undo === b.undo;
}

/**
 * The message-list diff behind `snapshot_delta`. `next` must already be identity-stable
 * (unchanged messages reuse their objects from `prev`); an object that differs by identity is
 * a change. The self-check replays the receiver's apply semantics — remove, then
 * replace-by-id or append — and returns undefined unless it reproduces `next`'s exact id
 * sequence, so a shape the protocol cannot express degrades to a full snapshot.
 */
export function diffMessages(prev: NormalizedMessage[], next: NormalizedMessage[]): MessagesDiff | undefined {
  const nextIds = new Set(next.map((message) => message.id));
  const removed = prev.filter((message) => !nextIds.has(message.id)).map((message) => message.id);
  const prevById = new Map(prev.map((message) => [message.id, message] as const));
  const upserts = next.filter((message) => prevById.get(message.id) !== message);

  const removedIds = new Set(removed);
  const applied = prev.filter((message) => !removedIds.has(message.id));
  for (const upsert of upserts) {
    const index = applied.findIndex((message) => message.id === upsert.id);
    if (index >= 0) applied[index] = upsert;
    else applied.push(upsert);
  }
  if (applied.length !== next.length || applied.some((message, index) => message.id !== next[index].id)) return undefined;
  return { upserts, removed };
}
