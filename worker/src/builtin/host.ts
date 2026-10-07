import type { Usage } from "@earendil-works/pi-ai";
import type { InlineExtension } from "@earendil-works/pi-coding-agent";
import type { AskQuestion, CommandPresentation, GoalState, PlanState, QuestionAnswer, SkillCreatorState, SubagentSpec, SubagentTranscript, TodoState } from "../protocol.js";
import type { GoalVerdict, GoalVerifyInput } from "./goal/verify.js";

/** What a running child reports while it works. */
export interface SubagentObserver {
  /** The child's session exists; `model` is its "Provider · Model" label. */
  started(model: string): void;
  /** The child called a tool. */
  tool(name: string, args: unknown): void;
  /** Running totals over the child's assistant messages so far. */
  usage(total: Usage, turns: number): void;
}

export interface SubagentRunRequest {
  /** The `subagent` call running this child, and the child's position in it: what the side
   *  panel watches it by. */
  toolCallId: string;
  index: number;
  spec: SubagentSpec;
  task: string;
  /** The final tool list: role allowlist, filtered for availability and the user's denylist. */
  tools: string[];
  /** Child-only extensions, e.g. the read-only guard. Nothing else ever loads in a child. */
  extensions: InlineExtension[];
  /** The child's own abort signal, linked to the parent only for blocking delegation. */
  signal: AbortSignal | undefined;
  observer: SubagentObserver;
}

export interface SubagentOutcome {
  status: "done" | "failed" | "aborted";
  /** The child's final answer (partial when it failed or was stopped). */
  output: string;
  /** Already redacted. */
  error?: string;
  usage: Usage;
  turns: number;
  /** Everything the child did, normalized, redacted and capped for the side panel. */
  transcript?: SubagentTranscript;
}

/**
 * The bridge built-in extensions use to reach the desktop UI. Implemented by the worker
 * (`index.ts`); extensions never see the host process or its protocol directly. Dialogs go
 * through the same `extension_ui_request` / `extension_ui_response` channel package
 * extensions use, which bypasses the worker's command queue so a tool awaiting an answer
 * inside a prompt can never deadlock.
 */
export interface BuiltinHost {
  publishTitleResult(attemptId: string, title?: string): void;
  /**
   * Ask the user a structured questionnaire. Resolves to one answer per question, or
   * `undefined` when the user dismisses the dialog or the run is aborted — callers report
   * that as a cancelled result rather than an error, matching the upstream contract. With
   * `offerWrapUp` (Ultra Plan) the card also offers "Write the plan now", which resolves to
   * `"wrap_up"` instead of answers.
   */
  askQuestions(
    questions: AskQuestion[],
    options?: { offerWrapUp?: boolean },
  ): Promise<QuestionAnswer[] | "wrap_up" | undefined>;
  /** Publish Plan mode state so the desktop can render the toggle and review card. */
  publishPlanState(state: PlanState): void;
  /** Publish the /skill-creator workflow's branch state (null clears it). */
  publishSkillCreatorState(state: SkillCreatorState | null): void;
  /** Publish the todo list so the desktop can render the panel above the composer. */
  publishTodoState(state: TodoState): void;
  /** Publish goal-loop state so the desktop can render the banner (null clears it). */
  publishGoalState(state: GoalState | null): void;
  /** Persist UI-only provenance immediately before a command-generated user message. */
  recordCommandPresentation(presentation: CommandPresentation): void;
  /** Desktop messages waiting outside Pi's own boundary queues take priority over a goal loop. */
  hasQueuedMessages?(): boolean;
  /** Includes pending result delivery, so terminal workflows never outrun their helpers. */
  hasOutstandingSubagents?(): boolean;
  /** Job progress/completion schedules snapshots and safe parent result delivery. */
  subagentsChanged?(): void;
  /** Background usage is attributed once, independently of status/wait tool calls. */
  recordSubagentUsage?(usage: Usage, spec: SubagentSpec): void;
  /** Capture a child's model/runtime when admitted, before it waits for a scheduler slot. */
  prepareSubagent?(request: SubagentRunRequest): { run(): Promise<SubagentOutcome>; dispose(): void };
  /**
   * Run the goal loop's completion verifier: one no-tools `completeSimple` on the chat's own
   * model, implemented by the worker which owns the model runtime. Never rejects — failures
   * come back as `inconclusive` (fail-open) or `aborted` verdicts.
   */
  runGoalVerification(input: GoalVerifyInput, signal?: AbortSignal): Promise<GoalVerdict>;
  /** Tools a sub-agent could be given (Pi's own, plus `web_fetch`) that are available here and
   *  that the user has not switched off. */
  childToolNames(): string[];
  /**
   * Run one sub-agent to completion in its own in-process session. Never rejects once the
   * child has started: failures and aborts come back as an outcome so their usage still counts.
   */
  runSubagent(request: SubagentRunRequest): Promise<SubagentOutcome>;
  /** Remove any credential the worker holds from text a child produced. */
  redact(text: string): string;
  /** Show a short message in the chat (redacted by the worker). */
  notice(message: string, level: "info" | "warning" | "error"): void;
  /** The chat's workspace folder, once the worker is initialized. */
  workspace(): string | undefined;
  /** This chat's id, for card details that must stay bound to their owning chat. */
  taskId(): string | undefined;
  /** Send one browser operation to the native host. The host owns the per-chat WebKit view. */
  browser(request: Record<string, unknown>, signal?: AbortSignal): Promise<unknown>;
  /** Send one skill-creator operation (prepare/preview) to the native host, which owns the
   *  managed draft workspace, validation and publication receipts. */
  skillCreator(request: Record<string, unknown>, signal?: AbortSignal): Promise<unknown>;
  /**
   * Send one computer-use operation to the native host, which owns capture, input, the per-app
   * grants and the access card. Rejects when the host refuses or the signal aborts.
   */
  computer(request: Record<string, unknown>, signal?: AbortSignal): Promise<unknown>;
  /** Whether this chat's current model can receive an image tool result. */
  supportsVision(): boolean;
}
