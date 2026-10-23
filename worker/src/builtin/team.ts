/**
 * Team: a per-chat flag on the sub-agents payload (`SubagentRuntimeConfig.team`) that turns the
 * chat's model into a lead. While it is present the system prompt gains `teamPrompt()`, the
 * delegation guidance acts as trigger "auto" whatever Settings says, every child gets a `jobId`
 * (blocking launches too), and finished children's sessions stay alive so `job` continuations
 * re-prompt them with the feedback, keeping their context.
 *
 * It is deliberately soft: the lead keeps all its tools — the prompt says implement through the
 * crew and edit only for small fixes. The crew model itself was already written into the built-in
 * agents' specs by the host, so nothing here needs to know it.
 *
 * Like the other live settings it never enters the worker fingerprint: `set_subagents` applies it
 * from the next message. `src/promptDefaults.ts` mirrors the default byte for byte for
 * Settings › Prompts.
 */
import { promptOverrides } from "../prompt-overrides.js";

export const DEFAULT_TEAM_PROMPT = `# Team
This chat's Team is on: the built-in sub-agents (scout, worker, reviewer) run on a cheaper crew model, and you lead them. Plan, delegate, check on progress, review what comes back, then decide the next step.
- Implement through the crew: send substantive work to subagent, usually the worker. Keep your own edits to one-line fixes or small polish you make after review; everything else goes to a worker.
- Delegate exploration too: reading large parts of the codebase belongs to a scout, not to your own context.
- Workers share this conversation's workspace but not its context. Each task must carry every file path, requirement and piece of context it needs; assign different files to workers running in parallel.
- Workers you launch, blocking or in the background, get a jobId. Inspect progress with subagent_job status; wait for results with subagent_job wait.
- Review before reporting back: read the diff (git diff) or the files a worker touched, or delegate the review to the reviewer role. To send a worker back, call subagent again with job set to its jobId — the same worker continues with its context, so say specifically what to change and why.
- When a worker's session has ended, launch a fresh worker and include the feedback in its brief.
- Summarize what the crew did and what you verified; workers' answers are reports to check, not facts to repeat.`;

/** The instruction a Team chat's system prompt gains: the user's custom text, or the default. */
export function teamPrompt(): string {
  return promptOverrides().teamPrompt ?? DEFAULT_TEAM_PROMPT;
}
