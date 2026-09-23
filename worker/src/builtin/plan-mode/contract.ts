/**
 * Cache-stable Plan/Ultra Plan/Normal mode contract messages. Ported from `@narumitw/pi-plan-mode`
 * v0.58.3 (MIT) `mode-contract.ts`: one hidden custom message is appended on each transition,
 * and the `context` hook reconciles the latest contract after compaction or restore. The
 * conversation stays append-only; no system prompt is rewritten.
 */
import { buildPlanModePrompt } from "./prompt.js";

export const MODE_CONTRACT_MESSAGE_TYPE = "wackcode-mode-contract";
export const MODE_CONTRACT_VERSION = 1;
export type PlanModeContract = "plan" | "ultraplan" | "normal";

const PLAN_CONTRACT_MARKER = `[WACKCODE PLAN MODE CONTRACT v${MODE_CONTRACT_VERSION}: PLAN]`;
const ULTRA_PLAN_CONTRACT_MARKER = `[WACKCODE PLAN MODE CONTRACT v${MODE_CONTRACT_VERSION}: ULTRAPLAN]`;
const NORMAL_CONTRACT_MARKER = `[WACKCODE PLAN MODE CONTRACT v${MODE_CONTRACT_VERSION}: NORMAL]`;
const NORMAL_CONTRACT = `${NORMAL_CONTRACT_MARKER}
Plan Mode is no longer active.
Follow the user's current Normal-mode request and ordinary system instructions.
Earlier Plan-mode restrictions no longer apply, but retain the planning conversation as context.
The visible tool schemas are session capabilities; use only tools appropriate for the current request.`;

interface ContractMessage {
  role?: string;
  customType?: string;
  content?: unknown;
  details?: unknown;
}

export function modeContractContent(mode: PlanModeContract) {
  if (mode === "plan") return `${PLAN_CONTRACT_MARKER}\n${buildPlanModePrompt()}`;
  if (mode === "ultraplan") return `${ULTRA_PLAN_CONTRACT_MARKER}\n${buildPlanModePrompt("ultraplan")}`;
  return NORMAL_CONTRACT;
}

const CONTRACT_MODES: readonly PlanModeContract[] = ["plan", "ultraplan", "normal"];

export function createModeContractMessage(mode: PlanModeContract, timestamp = Date.now()) {
  return {
    role: "custom" as const,
    customType: MODE_CONTRACT_MESSAGE_TYPE,
    content: modeContractContent(mode),
    display: false,
    details: { version: MODE_CONTRACT_VERSION, mode },
    timestamp,
  };
}

export function modeContractFromMessage(message: unknown): PlanModeContract | undefined {
  const candidate = unwrapMessage(message);
  if (candidate.customType !== MODE_CONTRACT_MESSAGE_TYPE) return undefined;
  return CONTRACT_MODES.find((mode) => candidate.content === modeContractContent(mode));
}

export function hasModeContractArtifact(messages: readonly unknown[]) {
  return messages.some((message) => unwrapMessage(message).customType === MODE_CONTRACT_MESSAGE_TYPE);
}

export function latestModeContract(messages: readonly unknown[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const mode = modeContractFromMessage(messages[index]);
    if (mode) return { index, mode };
  }
  return undefined;
}

export function reconcileModeContract<T>(messages: T[], expected: PlanModeContract): T[] {
  const latest = latestModeContract(messages);
  if (latest?.mode === expected) return messages;

  const latestContractIndex = findLatestContractArtifactIndex(messages);
  const insertionIndex = latestContractIndex >= 0 ? latestContractIndex + 1 : leadingSummaryBoundary(messages);
  const contract = createModeContractMessage(expected, 0) as unknown as T;
  return [
    ...messages.slice(0, insertionIndex),
    contract,
    ...messages.slice(insertionIndex),
  ];
}

function findLatestContractArtifactIndex(messages: readonly unknown[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (unwrapMessage(messages[index]).customType === MODE_CONTRACT_MESSAGE_TYPE) return index;
  }
  return -1;
}

function leadingSummaryBoundary(messages: readonly unknown[]) {
  let index = unwrapMessage(messages[0]).role === "system" ? 1 : 0;
  while (index < messages.length) {
    const role = unwrapMessage(messages[index]).role;
    if (role !== "compactionSummary" && role !== "branchSummary") break;
    index += 1;
  }
  return index;
}

function unwrapMessage(message: unknown): ContractMessage {
  const entry = message as { message?: unknown } | undefined;
  return (entry?.message ?? message ?? {}) as ContractMessage;
}
