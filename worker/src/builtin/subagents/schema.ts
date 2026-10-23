import { MAX_PARALLEL_TASKS } from "./types.js";

export interface SubagentTaskInput {
  /** The role's name, or — for a `job` continuation — the retained worker's own role. */
  agent: string;
  task: string;
  /** A finished child's jobId this task continues, instead of launching a fresh agent. */
  job?: string;
  /** The retained session's next prompt round (2+); absent on a first launch. */
  round?: number;
}

export type SubagentMode = "single" | "parallel";

type NormalizeResult =
  | { ok: true; mode: SubagentMode; tasks: SubagentTaskInput[]; background?: true }
  | { ok: false; error: string };

/**
 * The tool's parameters. Agent names become an enum so the model can only pick an agent that
 * is switched on; with none known yet it stays a plain string and `execute` reports the list.
 * `continuable` (a Team chat) adds `job`, which continues a finished child with its context.
 */
export function subagentParams(agentNames: string[], continuable = false) {
  const agent = {
    type: "string",
    description: "The sub-agent to run, by name.",
    ...(agentNames.length > 0 ? { enum: [...agentNames] } : {}),
  };
  const job = {
    type: "string",
    description: "The jobId of a finished sub-agent to continue instead of starting a fresh one: the same worker takes the task with all the context it built. Only workers this session launched can be continued.",
  };
  const task = {
    type: "string",
    description:
      "The complete task. The sub-agent cannot see this conversation, so include every file path, requirement and piece of context it needs.",
  };
  const itemProperties: Record<string, unknown> = continuable ? { agent, job, task } : { agent, task };
  return {
    type: "object",
    additionalProperties: false,
    properties: {
      background: { type: "boolean", description: "Return immediately with job IDs immediately and continue your own independent work. Completed background results arrive automatically; subagent_job can inspect, wait for or stop them." },
      agent: { ...agent, description: `Single mode: ${agent.description}` },
      ...(continuable ? { job } : {}),
      task: { ...task, description: `Single mode: ${task.description}` },
      tasks: {
        type: "array",
        minItems: 1,
        maxItems: MAX_PARALLEL_TASKS,
        description: `Parallel mode: up to ${MAX_PARALLEL_TASKS} independent tasks that run at the same time. Use instead of agent and task.`,
        items: {
          type: "object",
          additionalProperties: false,
          required: continuable ? ["task"] : ["agent", "task"],
          properties: itemProperties,
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

/** Exactly one mode: `agent` or `job` + `task`, or `tasks`. `job` needs Team on. */
export function normalizeSubagentParams(input: unknown, continuable = false): NormalizeResult {
  if (!isRecord(input)) return { ok: false, error: "subagent needs either agent and task, or tasks." };
  if (input.background !== undefined && typeof input.background !== "boolean") return { ok: false, error: "background must be a boolean." };
  const background = input.background === true ? { background: true as const } : {};
  const single = input.agent !== undefined || input.task !== undefined || input.job !== undefined;
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
      const job = isRecord(item) ? text(item.job) : "";
      const task = isRecord(item) ? text(item.task) : "";
      if (agent && job) return { ok: false, error: `tasks[${index}] needs either agent or job, not both.` };
      if (job && !continuable) return { ok: false, error: `tasks[${index}]: job is only available with Team on.` };
      if (!agent && !job) return { ok: false, error: `tasks[${index}] needs an agent (or a job to continue).` };
      if (!task) return { ok: false, error: `tasks[${index}] needs a task.` };
      tasks.push({ agent: agent || `job:${job}`, task, ...(job ? { job } : {}) });
    }
    return { ok: true, mode: "parallel", tasks, ...background };
  }
  const agent = text(input.agent);
  const job = text(input.job);
  const task = text(input.task);
  if (agent && job) return { ok: false, error: "Pass either agent or job with task, not both." };
  if (job && !continuable) return { ok: false, error: "job is only available with Team on." };
  if (!agent && !job) return { ok: false, error: "subagent needs either agent and task, or tasks." };
  if (!task) return { ok: false, error: "subagent needs a task." };
  return { ok: true, mode: "single", tasks: [{ agent: agent || `job:${job}`, task, ...(job ? { job } : {}) }], ...background };
}
