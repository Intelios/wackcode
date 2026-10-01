/**
 * The model-facing guidance for the todo tool. Adapted from `@juicesharp/rpiv-todo`
 * v2.11.0 (MIT) `todo.ts` (the `DEFAULT_PROMPT_SNIPPET` / `DEFAULT_PROMPT_GUIDELINES`
 * there); the behavioral rules are kept verbatim — they encode why the tool exists:
 * keeping the agent honest about multi-step progress — except the final line, a
 * WackCode addition telling the model several `todo` calls may share one response
 * (the agent loop applies them in emission order), so creating a list isn't one
 * turn per task.
 */
export const TODO_PROMPT_SNIPPET = "Manage a task list to track multi-step progress";

export const TODO_PROMPT_GUIDELINES: string[] = [
  "Use `todo` for complex work with 3+ steps, when the user gives you a list of tasks, or immediately after receiving new instructions to capture requirements. Skip it for single trivial tasks and purely conversational requests.",
  "When starting a task from the todo list, mark it in_progress BEFORE beginning work. Mark it completed IMMEDIATELY when done — never batch completions. Exactly one task in_progress at a time.",
  "Never mark a task completed if tests are failing, the implementation is partial, or you hit unresolved errors — keep it in_progress and create a new task for the blocker instead.",
  "Task status is a 4-state machine: pending → in_progress → completed, plus deleted as a tombstone. Pass activeForm (present-continuous label, e.g. 'researching existing tool') when marking in_progress.",
  'To change a task\'s status, call update with the task id and the target status, e.g. {"action":"update","id":3,"status":"completed"} or {"action":"update","id":3,"status":"in_progress","activeForm":"writing tests"}. status is the field that changes the task; an update without a mutable field (status or another) is rejected.',
  "Use blockedBy to express dependencies (A is blocked by B). On create, pass blockedBy as the initial set. On update, use addBlockedBy / removeBlockedBy (additive merge — do not resend the full array). Cycles are rejected.",
  "list hides tombstoned (deleted) tasks by default; pass includeDeleted:true to see them. Pass status to filter by a single status.",
  "Subject must be short and imperative (e.g. 'Research existing tool'); description is for long-form detail. activeForm is a present-continuous label shown while in_progress.",
  "You can emit several `todo` calls in one response — create all known tasks at once instead of one per turn. A task created in the same batch can't be referenced by id (e.g. as a blockedBy target) until the next turn.",
];
