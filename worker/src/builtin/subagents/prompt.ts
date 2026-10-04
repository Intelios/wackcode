import type { SubagentSpec, SubagentTrigger } from "../../protocol.js";
import { MAX_PARALLEL_TASKS, SUBAGENT_TOOL_NAME } from "./types.js";

export const SUBAGENT_PROMPT_SNIPPET = "Delegate self-contained tasks to specialised sub-agents, one at a time or several in parallel";

/** The tool description: how the modes work, then the roster the user has switched on. */
export function subagentDescription(agents: SubagentSpec[]): string {
  const roster = agents.length
    ? agents
        .map((agent) => `- ${agent.name} (${agent.readOnly ? "read-only" : "can edit files"}): ${agent.description}`)
        .join("\n")
    : "- (none are switched on)";
  return [
    "Run a sub-agent: a separate agent with its own fresh context window that works in this same workspace and returns only its final answer.",
    `Single mode: pass agent and task. Parallel mode: pass tasks (up to ${MAX_PARALLEL_TASKS}) to run independent tasks at the same time.`,
    "All sub-agents, including those that edit files, run concurrently up to the configured limit in the same workspace.",
    `Available sub-agents:\n${roster}`,
  ].join("\n");
}

/** Guideline bullets. Pi appends them flat, so each one names the tool. */
export function subagentGuidelines(trigger: SubagentTrigger, readOnlyPlanning = true): string[] {
  const when =
    trigger === "auto"
      ? `Use ${SUBAGENT_TOOL_NAME} when a task clearly benefits from delegation, such as broad codebase exploration, an independent review, or several separate investigations at once. Do small or tightly coupled work yourself: every sub-agent costs extra model usage.`
      : `Only call ${SUBAGENT_TOOL_NAME} when the user explicitly asks for sub-agents, delegation or parallel agents, or names one of the available sub-agents. Otherwise do the work yourself: every sub-agent costs extra model usage.`;
  return [
    when,
    `${SUBAGENT_TOOL_NAME} children cannot see this conversation. Put every file path, requirement, constraint and piece of context they need into each task.`,
    `Use the tasks parameter of ${SUBAGENT_TOOL_NAME} for independent work that can run at the same time; assign distinct files to editing children. Run dependent tasks or edits to the same files sequentially, waiting for each result before starting the next.`,
    `${SUBAGENT_TOOL_NAME} returns each child's final answer. Check important claims before relying on them, and summarize the results for the user instead of pasting them.`,
    readOnlyPlanning
      ? `In read-only Plan or Ultra Plan mode, ${SUBAGENT_TOOL_NAME} only runs read-only sub-agents.`
      : `${SUBAGENT_TOOL_NAME} follows each role's effective access shown above. During Plan or Ultra Plan, editing children may support planning but must not implement the finished plan before approval.`,
  ];
}
