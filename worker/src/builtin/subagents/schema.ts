import { MAX_PARALLEL_TASKS } from "./types.js";

export interface SubagentTaskInput {
  agent: string;
  task: string;
}

export type SubagentMode = "single" | "parallel";

type NormalizeResult =
  | { ok: true; mode: SubagentMode; tasks: SubagentTaskInput[] }
  | { ok: false; error: string };

/**
 * The tool's parameters. Agent names become an enum so the model can only pick an agent that
 * is switched on; with none known yet it stays a plain string and `execute` reports the list.
 */
export function subagentParams(agentNames: string[]) {
  const agent = {
    type: "string",
    description: "The sub-agent to run, by name.",
    ...(agentNames.length > 0 ? { enum: [...agentNames] } : {}),
  };
  const task = {
    type: "string",
    description:
      "The complete task. The sub-agent cannot see this conversation, so include every file path, requirement and piece of context it needs.",
  };
  return {
    type: "object",
    additionalProperties: false,
    properties: {
      agent: { ...agent, description: "Single mode: the sub-agent to run, by name." },
      task: { ...task, description: `Single mode: ${task.description}` },
      tasks: {
        type: "array",
        minItems: 1,
        maxItems: MAX_PARALLEL_TASKS,
        description: `Parallel mode: up to ${MAX_PARALLEL_TASKS} independent tasks that run at the same time. Use instead of agent and task.`,
        items: {
          type: "object",
          additionalProperties: false,
          required: ["agent", "task"],
          properties: { agent, task },
        },
      },
    },
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/** Exactly one mode: `agent` + `task`, or `tasks`. */
export function normalizeSubagentParams(input: unknown): NormalizeResult {
  if (!isRecord(input)) return { ok: false, error: "subagent needs either agent and task, or tasks." };
  const single = input.agent !== undefined || input.task !== undefined;
  const parallel = input.tasks !== undefined;
  if (single && parallel) {
    return { ok: false, error: "Pass either agent and task (one sub-agent) or tasks (several in parallel), not both." };
  }
  if (parallel) {
    if (!Array.isArray(input.tasks) || input.tasks.length === 0) {
      return { ok: false, error: "tasks must be a non-empty array." };
    }
    if (input.tasks.length > MAX_PARALLEL_TASKS) {
      return { ok: false, error: `tasks can hold at most ${MAX_PARALLEL_TASKS} items.` };
    }
    const tasks: SubagentTaskInput[] = [];
    for (const [index, item] of input.tasks.entries()) {
      const agent = isRecord(item) ? text(item.agent) : "";
      const task = isRecord(item) ? text(item.task) : "";
      if (!agent || !task) return { ok: false, error: `tasks[${index}] needs both agent and task.` };
      tasks.push({ agent, task });
    }
    return { ok: true, mode: "parallel", tasks };
  }
  const agent = text(input.agent);
  const task = text(input.task);
  if (!agent || !task) return { ok: false, error: "subagent needs either agent and task, or tasks." };
  return { ok: true, mode: "single", tasks: [{ agent, task }] };
}
