import type { NormalizedBlock, NormalizedMessage, TaskStatus } from "./types";
import type { TranscriptLayout, TranscriptSlot } from "./explore-utils";
import { SKILL_CREATOR_TOOL_NAME, parseSkillPreviewDetails } from "./tool-utils";

/**
 * Presentation only: each user starts a turn on the visible session branch. The worker's
 * `turn` marks the latest assistant, NOT run completion. Never fold an active run's earlier
 * turns just because steering delivered another user message. Keep original messages and
 * block indices so results, cached signatures, copy/actions and checkpoints stay authoritative.
 * Exploration slots contain only technical work, so they cannot absorb a visible outcome.
 */
export type WorkRow =
  | { type: "message"; index: number; slots: TranscriptSlot[]; actions: boolean }
  | { type: "orphans"; index: number; blocks: NormalizedBlock[] }
  | { type: "compaction"; index: number }
  | { type: "switches"; position: number };

export interface WorkTurn {
  userIndex: number;
  userKey: string;
  endIndex: number;
  /** Actual entry/outcome identity, not the version group's shared identity. */
  key: string;
  body: WorkRow[];
  /** Prepare the same work/outcome boundary while live, without hiding any content. */
  partition?: { work: WorkRow[]; outcome: WorkRow[] };
  /** Present only for a settled, successful turn with both work and an outcome. */
  folded?: { work: WorkRow[]; outcome: WorkRow[] };
}

/** The result is authoritative: rejected or unfinished plan calls are never plan cards. */
export function completedPlan(result?: NormalizedBlock): string | undefined {
  if (result?.isError) return undefined;
  const details = result?.details as { plan?: unknown } | undefined;
  return typeof details?.plan === "string" && details.plan.trim() ? details.plan : undefined;
}

/** Same rule for skill-creator previews: only a finished, valid result is a review card. */
export function completedSkillDraft(result?: NormalizedBlock): boolean {
  return !result?.isError && parseSkillPreviewDetails(result?.details) !== undefined;
}

interface Options {
  layout: TranscriptLayout;
  results: Map<string, NormalizedBlock>;
  callIds: Set<string>;
  switchPositions: Set<number>;
  running: boolean;
  activeRun?: { startedAt: number };
  partial?: NormalizedMessage;
  status?: TaskStatus;
  /** Saved run durations prove historical completion when a live run has no clock yet. */
  settledUserIds?: Set<string>;
}

const failed = (message: NormalizedMessage) => message.stopReason === "error" || message.stopReason === "aborted";
const identity = (message: NormalizedMessage) => message.entryId ?? message.id;

export function layoutWorkTurns(messages: NormalizedMessage[], options: Options): Map<number, WorkTurn> {
  const { layout, results, callIds, switchPositions, running, activeRun, partial, status, settledUserIds } = options;
  const starts = messages.flatMap((message, index) => message.role === "user" ? [index] : []);
  // With no start clock, keep all unfinished turns open rather than guess which steering
  // messages belong to the run. Historical turns with completed outcomes stay unchanged.
  const activeStart = (running || activeRun) && activeRun
    ? starts.find((index) => messages[index].timestamp !== undefined && messages[index].timestamp! >= activeRun.startedAt)
    : undefined;
  const turns = new Map<number, WorkTurn>();

  starts.forEach((userIndex, ordinal) => {
    const user = messages[userIndex];
    const endIndex = starts[ordinal + 1] ?? messages.length;
    const body: WorkRow[] = [];
    let terminal = -1;
    let annotated = -1;
    let plan: { index: number; block: number; id: string } | undefined;
    let skill: { index: number; block: number; id: string } | undefined;
    let systemNotice = false;
    let unfinishedCall = false;

    for (let index = userIndex + 1; index < endIndex; index += 1) {
      const message = messages[index];
      if (switchPositions.has(index)) body.push({ type: "switches", position: index });
      if (message.compaction) {
        body.push({ type: "compaction", index });
        continue;
      }
      if (message.role === "tool") {
        const blocks = message.blocks.filter((block) => !block.toolCallId || !callIds.has(block.toolCallId));
        if (blocks.length) body.push({ type: "orphans", index, blocks });
      } else {
        const slots = layout.messages.get(message.id) ?? [];
        body.push({ type: "message", index, slots, actions: true });
        if (message.role === "system" && message.blocks.some((block) => block.text?.trim())) systemNotice = true;
        if (message.role !== "assistant") continue;
        terminal = index;
        if (message.turn?.userEntryId === identity(user)) annotated = index;
        message.blocks.forEach((block, blockIndex) => {
          if (block.type !== "tool-call") return;
          const result = block.toolCallId ? results.get(block.toolCallId) : undefined;
          if (!result) unfinishedCall = true;
          if (block.toolName === "plan_mode_complete" && completedPlan(result)) {
            plan = { index, block: blockIndex, id: block.toolCallId ?? `${identity(message)}:${blockIndex}` };
          }
          if (block.toolName === SKILL_CREATOR_TOOL_NAME && completedSkillDraft(result) && block.toolCallId) {
            skill = { index, block: blockIndex, id: block.toolCallId };
          }
        });
      }
    }

    // A just-written assistant may not have its entry annotation until the next snapshot.
    // An older annotation with a newer assistant is not a settled outcome yet.
    const staleAnnotation = annotated >= 0 && annotated !== terminal;
    const tail = endIndex === messages.length;
    const active = activeStart !== undefined ? userIndex >= activeStart
      : (running || Boolean(activeRun)) && !settledUserIds?.has(user.id);
    const unsettled = active || (tail && (Boolean(partial) || status === "interrupted" || status === "error" || status === "stopping"));
    const answer = terminal >= 0 ? messages[terminal] : undefined;
    const turn: WorkTurn = {
      userIndex, userKey: identity(user), endIndex,
      key: JSON.stringify([identity(user), answer ? identity(answer) : null, plan?.id ?? null, skill?.id ?? null]),
      body
    };
    turns.set(userIndex, turn);
    if (staleAnnotation || systemNotice || unfinishedCall || !answer || failed(answer)) return;
    // A turn split by compaction keeps its chronological detail. A compaction AFTER the
    // answer can accompany the outcome without ever disappearing inside the work fold.
    if (body.some((row) => row.type === "compaction" && row.index < terminal)) return;

    let lastCall = -1;
    answer.blocks.forEach((block, index) => { if (block.type === "tool-call") lastCall = index; });
    const textIndices = new Set(answer.blocks.flatMap((block, index) =>
      index > lastCall && block.type === "text" && block.text?.trim() ? [index] : []));
    if (!textIndices.size && !plan && !skill) return;

    const work: WorkRow[] = [];
    const outcome: WorkRow[] = [];
    const visible = (index: number, slot: TranscriptSlot) => slot.type === "block"
      && ((index === terminal && textIndices.has(slot.index)) || (index === plan?.index && slot.index === plan.block) || (index === skill?.index && slot.index === skill.block));
    for (const row of body) {
      if (row.type === "compaction") { outcome.push(row); continue; }
      if (row.type !== "message") { work.push(row); continue; }
      const outcomeSlots = row.slots.filter((slot) => visible(row.index, slot));
      const workSlots = row.slots.filter((slot) => !visible(row.index, slot));
      if (workSlots.length) work.push({ ...row, slots: workSlots, actions: false });
      // Keep the terminal footer even if a plan or skill card, not this message, is the outcome.
      if (outcomeSlots.length || (row.index === terminal && answer.turn)) {
        outcome.push({ ...row, slots: outcomeSlots, actions: row.index === terminal });
      }
    }
    const hasWork = work.some((row) => row.type === "orphans" || (row.type === "message" && row.slots.length > 0));
    // An earlier failed assistant is diagnostic work, not a hidden error label. Leave such a
    // turn open; errors reported by tools are ordinary inspectable work after a valid answer.
    if (hasWork && !messages.slice(userIndex + 1, endIndex).some(failed)) {
      turn.partition = { work, outcome };
      if (!unsettled) turn.folded = turn.partition;
    }
  });
  return turns;
}
