/**
 * Built-in goal extension — the harness-level loop behind `/goal`. The working model never
 * declares victory: after every settled round a separate no-tools verifier call (the host's
 * `runGoalVerification`, on the chat's own model) returns `{passed, reason, nextAction}`, and
 * a `passed:false` with a `nextAction` injects the next turn itself. Codex (`ext/goal`) and
 * ZCode converged on the same split.
 *
 * Timing: `agent_settled` extension handlers run after Pi clears its run-active flag but
 * before the public event reaches the worker, so a `sendUserMessage` here starts the next
 * round as a fresh nested run. The `continuing` flag tells the worker's run bookkeeping to
 * keep the chat busy instead of flashing idle between rounds.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { GoalState } from "../../protocol.js";
import type { BuiltinHost } from "../host.js";
import { goalContinuationPrompt, goalKickoffPrompt } from "./prompt.js";
import {
  DEFAULT_MAX_ITERATIONS,
  GOAL_ENTRY_TYPE,
  NO_PROGRESS_LIMIT,
  restoreGoalState,
  toPersistedGoal,
} from "./state.js";
import type { GoalVerdict } from "./verify.js";

export interface GoalController {
  /** The state the desktop banner renders; undefined when no goal exists. */
  getState(): GoalState | undefined;
  /**
   * The settled handler just queued a continuation run. The worker reads this at its own
   * `agent_settled` to keep `run_state` busy instead of emitting a false idle between rounds.
   */
  willContinue(): boolean;
  /** The queued continuation never became a run — pause rather than leaving the chat busy. */
  continuationDropped(): void;
  /** `/goal <objective>`: activate the goal; returns the first turn's prompt text. */
  start(objective: string): string;
  /** Pause after the current round; aborts an in-flight verifier. */
  pause(note?: string): void;
  /** Resume a paused goal; returns the continuation prompt for the worker to run. */
  resume(): string;
  clear(): void;
  /** The user pressed Stop: abort any in-flight verification and pause instead of completing. */
  userStop(): void;
}

const TERMINAL_PHASES: ReadonlySet<GoalState["phase"]> = new Set(["complete", "stopped"]);

interface SessionEntryLike {
  type?: string;
  message?: { role?: string; content?: unknown; toolName?: string };
}

const ROLE_LABEL: Record<string, string> = { user: "user", assistant: "assistant", toolResult: "tool" };

// The subset of Pi's AgentMessage the loop inspects (Pi's own type lives in pi-agent-core and
// isn't re-exported from the coding-agent entry point).
interface RoundMessage {
  role?: string;
  stopReason?: string;
  content?: unknown;
}

/**
 * Render the branch tail for the verifier: the newest ~40 visible messages, each truncated,
 * capped overall so the judge's context stays small even on long sessions.
 */
export function renderTranscriptSlice(branch: Iterable<unknown>): string {
  const lines: string[] = [];
  let total = 0;
  for (const raw of branch) {
    const entry = raw as SessionEntryLike;
    if (entry?.type !== "message") continue;
    const message = entry.message;
    if (!message?.role) continue;
    const label = message.role === "custom" ? "user" : ROLE_LABEL[message.role];
    if (!label) continue;
    const text = messageText(message.content);
    const toolSuffix = message.role === "toolResult" && message.toolName ? ` ${message.toolName}` : "";
    if (!text && !toolSuffix) continue;
    lines.push(`[${label}${toolSuffix}] ${text.slice(0, 600)}`);
  }
  const tail = lines.slice(-40);
  const out: string[] = [];
  for (let index = tail.length - 1; index >= 0 && total < 12_000; index -= 1) {
    total += tail[index].length;
    out.unshift(tail[index]);
  }
  return out.join("\n");
}

function messageText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => {
      if (!block || typeof block !== "object") return "";
      const part = block as Record<string, unknown>;
      if (part.type === "text") return String(part.text ?? "");
      if (part.type === "toolCall") return `[calls ${String(part.name ?? "tool")}]`;
      if (part.type === "image") return "[image]";
      return "";
    })
    .filter(Boolean)
    .join(" ");
}

/** The round made progress if it ran a tool or the verifier's next step moved. */
function roundMadeProgress(round: RoundMessage[], nextAction: string, lastNextAction: string | undefined): boolean {
  if (nextAction !== lastNextAction) return true;
  return round.some((message) =>
    message.role === "toolResult" ||
    (Array.isArray(message.content) && message.content.some((block: { type?: string }) => block.type === "toolCall")));
}

export function createGoalExtension(host: BuiltinHost, isPlanning: () => boolean): {
  factory: (api: ExtensionAPI) => void;
  controller: GoalController;
} {
  let pi: ExtensionAPI | undefined;
  let goal: GoalState | undefined;
  /** Messages produced since the last settle, accumulated across agent_ends of one run. */
  let pendingRound: RoundMessage[] | undefined;
  /** A blocking extension dialog is open — verifying now would judge an unfinished turn. */
  let uiPromptOpen = false;
  /** Set inside the settled handler the moment a continuation is queued. */
  let continuing = false;
  let verifyAbort: AbortController | undefined;

  const emit = () => host.publishGoalState(goal ?? null);
  const persist = () => {
    try {
      pi?.appendEntry(GOAL_ENTRY_TYPE, toPersistedGoal(goal));
    } catch { /* Persistence is best-effort; the loop keeps working without it. */ }
  };

  const finish = (phase: "complete" | "stopped", note?: string) => {
    if (!goal) return;
    goal.phase = phase;
    goal.note = note;
    continuing = false;
    persist();
    emit();
  };

  const controller: GoalController = {
    getState: () => goal,
    willContinue: () => continuing,
    continuationDropped() {
      if (!goal) return;
      continuing = false;
      goal.phase = "paused";
      goal.note = "The next round could not start — resume with /goal resume.";
      persist();
      emit();
    },
    start(objective) {
      if (!pi) throw new Error("The goal extension is still loading; try again in a moment.");
      if (goal && !TERMINAL_PHASES.has(goal.phase) && goal.phase !== "paused") {
        throw new Error("This chat already has a goal. Run /goal clear before starting a new one.");
      }
      goal = { objective, phase: "active", iteration: 0, maxIterations: DEFAULT_MAX_ITERATIONS, noProgress: 0 };
      continuing = false;
      pendingRound = undefined;
      persist();
      emit();
      return goalKickoffPrompt(objective);
    },
    pause(note) {
      if (!goal) throw new Error("This chat has no goal.");
      if (TERMINAL_PHASES.has(goal.phase) || goal.phase === "paused") return;
      verifyAbort?.abort();
      goal.phase = "paused";
      goal.note = note ?? "Paused.";
      continuing = false;
      persist();
      emit();
    },
    resume() {
      if (!pi) throw new Error("The goal extension is still loading; try again in a moment.");
      if (!goal) throw new Error("This chat has no goal.");
      if (goal.phase !== "paused") {
        throw new Error(
          TERMINAL_PHASES.has(goal.phase)
            ? "That goal already ended. Start a new one with /goal."
            : "The goal is still running.",
        );
      }
      goal.phase = "active";
      goal.note = undefined;
      const text = goalContinuationPrompt(goal, goal.lastReason, goal.lastNextAction ?? "Continue working toward the objective.");
      persist();
      emit();
      return text;
    },
    clear() {
      if (!goal) return;
      verifyAbort?.abort();
      goal = undefined;
      continuing = false;
      persist();
      emit();
    },
    userStop() {
      if (!goal || TERMINAL_PHASES.has(goal.phase) || goal.phase === "paused") return;
      verifyAbort?.abort();
      goal.phase = "paused";
      goal.note = "Stopped by user.";
      continuing = false;
      persist();
      emit();
    },
  };

  const factory = (bound: ExtensionAPI) => {
    pi = bound;

    // The desktop intercepts `/goal` before anything reaches Pi; this registration keeps the
    // name reserved inside Pi itself (a package command that grabs it is renamed) and gives
    // bare-Pi surfaces a minimal pause/resume/clear handler.
    pi.registerCommand("goal", {
      description: "Goal loop control: /goal pause, /goal resume, /goal clear.",
      async handler(args) {
        const action = args.trim();
        if (action === "pause") controller.pause();
        else if (action === "resume") pi?.sendUserMessage(controller.resume());
        else if (action === "clear") controller.clear();
      },
    });

    const restore = (ctx: { sessionManager: { getBranch(): unknown[] } }) => {
      const restored = restoreGoalState(ctx.sessionManager.getBranch());
      goal = restored;
      continuing = false;
      pendingRound = undefined;
      verifyAbort?.abort();
      verifyAbort = undefined;
      emit();
    };
    pi.on("session_start", (_event, ctx) => restore(ctx));
    pi.on("session_tree", (_event, ctx) => restore(ctx));
    pi.on("session_compact", (_event, ctx) => restore(ctx));

    pi.on("ui_prompt_start", () => { uiPromptOpen = true; });
    pi.on("ui_prompt_end", () => { uiPromptOpen = false; });

    // Stash the run's messages; several agent_ends can precede one settle when follow-ups or
    // retries extend the same run.
    pi.on("agent_end", (event) => {
      pendingRound = [...(pendingRound ?? []), ...(event.messages as RoundMessage[])];
    });

    pi.on("agent_settled", async (_event, ctx) => {
      continuing = false;
      const round = pendingRound;
      pendingRound = undefined;
      if (!goal || goal.phase !== "active" || !round) return;
      // Planning modes never loop; a queued steer/follow-up or an open dialog means the
      // user's own input drives the next turn.
      if (isPlanning() || uiPromptOpen || ctx.hasPendingMessages()) return;
      const lastAssistant = [...round].reverse().find((message) => message.role === "assistant");
      const stopReason = lastAssistant?.stopReason;
      if (stopReason === "aborted") { goal.phase = "paused"; goal.note = "Stopped by user."; persist(); emit(); return; }
      if (stopReason === "error") { goal.phase = "paused"; goal.note = "The model failed — the goal paused."; persist(); emit(); return; }

      goal.phase = "verifying";
      persist();
      emit();

      verifyAbort?.abort();
      const abort = new AbortController();
      verifyAbort = abort;
      const verdict: GoalVerdict = await host.runGoalVerification(
        { objective: goal.objective, iteration: goal.iteration + 1, transcript: renderTranscriptSlice(ctx.sessionManager.getBranch()) },
        abort.signal,
      );
      if (verifyAbort === abort) verifyAbort = undefined;
      // Paused, cleared or stopped while the verifier ran — its verdict no longer applies.
      if (!goal || goal.phase !== "verifying") return;

      goal.iteration += 1;
      const priorNextAction = goal.lastNextAction;

      switch (verdict.kind) {
        case "pass":
          goal.lastReason = verdict.reason;
          finish("complete", verdict.reason);
          return;
        case "inconclusive":
          finish("complete", `Verifier could not judge — treating as complete. ${verdict.reason}`);
          return;
        case "aborted":
          goal.phase = "paused";
          goal.note = "Stopped by user.";
          persist();
          emit();
          return;
        case "stop":
          goal.lastReason = verdict.reason;
          finish("stopped", verdict.reason);
          return;
        case "continue": {
          goal.lastReason = verdict.reason;
          goal.lastNextAction = verdict.nextAction;
          goal.noProgress = roundMadeProgress(round, verdict.nextAction, priorNextAction) ? 0 : goal.noProgress + 1;
          if (goal.noProgress >= NO_PROGRESS_LIMIT) {
            goal.phase = "paused";
            goal.note = `Paused: ${NO_PROGRESS_LIMIT} rounds made no progress.`;
            persist();
            emit();
            return;
          }
          if (goal.iteration >= goal.maxIterations) {
            finish("stopped", `Stopped at the ${goal.maxIterations}-round limit.`);
            return;
          }
          goal.phase = "active";
          goal.note = undefined;
          continuing = true;
          pi?.sendUserMessage(goalContinuationPrompt(goal, verdict.reason, verdict.nextAction));
          persist();
          emit();
          return;
        }
      }
    });
  };

  return { factory, controller };
}
