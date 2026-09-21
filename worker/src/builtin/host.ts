import type { AskQuestion, PlanState, QuestionAnswer, TodoState } from "../protocol.js";

/**
 * The bridge built-in extensions use to reach the desktop UI. Implemented by the worker
 * (`index.ts`); extensions never see the host process or its protocol directly. Dialogs go
 * through the same `extension_ui_request` / `extension_ui_response` channel package
 * extensions use, which bypasses the worker's command queue so a tool awaiting an answer
 * inside a prompt can never deadlock.
 */
export interface BuiltinHost {
  /**
   * Ask the user a structured questionnaire. Resolves to one answer per question, or
   * `undefined` when the user dismisses the dialog or the run is aborted — callers report
   * that as a cancelled result rather than an error, matching the upstream contract.
   */
  askQuestions(questions: AskQuestion[]): Promise<QuestionAnswer[] | undefined>;
  /** Publish Plan mode state so the desktop can render the toggle and review card. */
  publishPlanState(state: PlanState): void;
  /** Publish the todo list so the desktop can render the panel above the composer. */
  publishTodoState(state: TodoState): void;
}
