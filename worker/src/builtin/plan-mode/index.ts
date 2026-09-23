/**
 * Built-in Plan mode extension — a Codex-style read-only planning mode. Forked from
 * `@narumitw/pi-plan-mode` v0.58.3 (MIT) and adapted to WackCode: mode transitions come from
 * the app's composer toggle (via `PlanModeController`) instead of `/plan` menus, and the
 * completed plan is reviewed in the desktop UI rather than Pi's TUI. The shell/tool policy,
 * mode contract, completion tool and session restore are ports of the upstream modules in
 * this directory.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { PlanState, TaskMode } from "../../protocol.js";
import { ASK_USER_QUESTION_TOOL_NAME } from "../ask-user-question.js";
import type { BuiltinHost } from "../host.js";
import { TODO_TOOL_NAME } from "../todo/types.js";
import {
  PLAN_MODE_COMPLETE_PARAMS,
  PLAN_MODE_COMPLETE_TOOL_NAME,
  normalizePlanModeCompletion,
  planModeCompleted,
} from "./completion.js";
import {
  type PlanModeContract,
  createModeContractMessage,
  hasModeContractArtifact,
  latestModeContract,
  reconcileModeContract,
} from "./contract.js";
import { SUBAGENT_TOOL_NAME } from "../subagents/types.js";
import {
  DEFAULT_SAFE_SUBCOMMANDS as SAFE_SUBCOMMANDS,
  classifyPlanTool,
  findBlockedCommandSegment,
  readCommand,
} from "./policy.js";
import { invalidPlanMessage, latestAssistantText, parseProposedPlan } from "./proposed-plan.js";
import { PLAN_STATE_ENTRY_TYPE, PLAN_STATE_VERSION, restorePlanState, type PersistedPlanState } from "./state.js";

export interface PlanModeController {
  /** The state the desktop renders (mode, phase, ready plan). */
  getState(): PlanState;
  /**
   * Switch modes. Publishes the hidden contract message, persists and re-emits state.
   * Throws when the extension has not finished loading.
   */
  setMode(mode: TaskMode): PlanState;
}

export function createPlanModeExtension(host: BuiltinHost) {
  let pi: ExtensionAPI | undefined;
  let enabled = false;
  let readyPlan: string | undefined;
  /** Whether contract messages exist (or could exist) on the branch — keeps reconcile active. */
  let contractsRelevant = false;
  let publishedContract: PlanModeContract | undefined;

  const getState = (): PlanState => ({
    mode: enabled ? "plan" : "build",
    phase: readyPlan !== undefined ? "ready" : "planning",
    ...(readyPlan !== undefined ? { plan: readyPlan } : {}),
  });

  const emit = () => host.publishPlanState(getState());

  const requirePi = () => {
    if (!pi) throw new Error("Plan mode is still loading; try again in a moment.");
    return pi;
  };

  const persist = () =>
    requirePi().appendEntry<PersistedPlanState>(PLAN_STATE_ENTRY_TYPE, {
      version: PLAN_STATE_VERSION,
      enabled,
      plan: readyPlan,
    });

  const publishContract = (mode: PlanModeContract) => {
    if (publishedContract === mode) return;
    const { role: _role, timestamp: _timestamp, ...message } = createModeContractMessage(mode);
    requirePi().sendMessage(message, { triggerTurn: false });
    publishedContract = mode;
    contractsRelevant = true;
  };

  const setMode = (mode: TaskMode): PlanState => {
    requirePi();
    const before = { enabled, readyPlan };
    if (mode === "plan") {
      if (!enabled) {
        publishContract("plan");
        enabled = true;
      }
    } else {
      if (enabled || contractsRelevant) publishContract("normal");
      enabled = false;
      // Leaving Plan mode abandons the proposed plan, same as upstream's exit.
      readyPlan = undefined;
    }
    // Every prompt carries the composer's mode. Writing an unchanged state before each user
    // message would only bury the conversation's structure in duplicate entries.
    if (before.enabled !== enabled || before.readyPlan !== readyPlan) persist();
    emit();
    return getState();
  };

  const acceptPlan = (plan: string) => {
    if (readyPlan === plan) return;
    readyPlan = plan;
    persist();
    emit();
  };

  const factory = (bound: ExtensionAPI) => {
    pi = bound;

    pi.registerTool({
      name: PLAN_MODE_COMPLETE_TOOL_NAME,
      label: "Complete plan",
      description:
        "Submit a decision-ready plan only while Plan mode is active, and call it alone as the final action. Never call for ordinary planning requests, roadmaps, checklists, or plan-file work.",
      parameters: PLAN_MODE_COMPLETE_PARAMS,
      async execute(_toolCallId, params: unknown) {
        if (!enabled) {
          throw new Error("plan_mode_complete is only available while Plan mode is active.");
        }
        const parsed = normalizePlanModeCompletion(params);
        if (!parsed.ok) throw new Error(parsed.error);
        acceptPlan(parsed.plan);
        return planModeCompleted(parsed.plan);
      },
    });

    // The branch is the store: a restart, a rewind, or a switch to another version of a message
    // re-derives the mode and any proposed plan from the entries on the new path.
    const restore = (branch: unknown[]) => {
      const restored = restorePlanState(branch);
      publishedContract = latestModeContract(branch)?.mode;
      contractsRelevant =
        restored.enabled || restored.plan !== undefined || hasModeContractArtifact(branch);
      enabled = restored.enabled;
      readyPlan = restored.plan;
      emit();
    };
    pi.on("session_start", (_event, ctx) => restore(ctx.sessionManager.getBranch()));
    pi.on("session_tree", (_event, ctx) => restore(ctx.sessionManager.getBranch()));

    // The runtime read-only policy. The prompt explains the rules; this enforces them.
    pi.on("tool_call", (event, ctx) => {
      // Built-in helpers always pass while planning. `todo` mutates only its own in-memory
      // list, never the workspace, so tracking a task list during planning stays on the
      // right side of the read-only policy. `subagent` refuses any agent that can edit files
      // while Plan mode is on, and read-only children run under this same shell policy.
      const helper =
        event.toolName === ASK_USER_QUESTION_TOOL_NAME ||
        event.toolName === PLAN_MODE_COMPLETE_TOOL_NAME ||
        event.toolName === TODO_TOOL_NAME ||
        event.toolName === SUBAGENT_TOOL_NAME;
      if (!enabled) {
        return event.toolName === PLAN_MODE_COMPLETE_TOOL_NAME
          ? { block: true, reason: "plan_mode_complete is only available while Plan mode is active." }
          : undefined;
      }
      if (helper) return undefined;

      const tool = pi!.getAllTools().find((candidate) => candidate.name === event.toolName);
      if (!tool) {
        return {
          block: true,
          reason: `Plan mode blocks '${event.toolName}' because it is not a registered tool.`,
        };
      }
      switch (classifyPlanTool(tool)) {
        case "read-only":
          return undefined;
        case "limited": {
          const blocked = findBlockedCommandSegment(readCommand(event.input), SAFE_SUBCOMMANDS, ctx.cwd);
          return blocked === undefined
            ? undefined
            : {
                block: true,
                reason: `Plan mode only allows read-only shell commands (inspection, status, tests).\nBlocked command: ${blocked}`,
              };
          }
        case "package":
          return {
            block: true,
            reason: `Plan mode blocks '${event.toolName}': tools from installed packages are unavailable while planning.`,
          };
        default:
          return {
            block: true,
            reason:
              event.toolName === "update_plan"
                ? "Plan mode blocks update_plan because it tracks execution progress rather than conversational planning."
                : `Plan mode blocks the mutating tool '${event.toolName}'.`,
          };
      }
    });

    // A fresh prompt while a plan awaits review is revision feedback: it can no longer be
    // implemented until the agent resubmits a complete (possibly unchanged) plan.
    pi.on("before_agent_start", () => {
      if (enabled && readyPlan !== undefined) {
        readyPlan = undefined;
        persist();
        emit();
      }
    });

    pi.on("context", (event) => {
      if (!enabled && !contractsRelevant) return undefined;
      return { messages: reconcileModeContract(event.messages, enabled ? "plan" : "normal") };
    });

    // Weaker models sometimes answer with a prose <proposed_plan> block instead of the tool.
    // A single well-formed block counts as a completion, exactly like upstream.
    pi.on("agent_end", (event, ctx) => {
      if (!enabled) return;
      const parsed = parseProposedPlan(latestAssistantText(event.messages));
      if (parsed.kind === "valid") {
        acceptPlan(parsed.plan);
      } else if (parsed.kind !== "absent") {
        ctx.ui.notify(invalidPlanMessage(parsed.kind), "warning");
      }
    });
  };

  const controller: PlanModeController = {
    getState,
    setMode,
  };

  return { factory, controller };
}
