/**
 * Shell commands belong to an agent run, not to one tool wait. After at most 60 seconds a
 * command yields its output and a worker-local job id; bash_job can inspect, wait or stop it.
 * A model-supplied timeout is still a hard execution deadline, never a longer blocking wait.
 *
 * Keep Pi's shell backend: it supplies the environment, bounded/UTF-8-safe output, spill files
 * and process-group cancellation. Each job owns its execution signal so yielding does not
 * kill it. Only active waits forward updates; a yielded tool must never emit late events.
 * Jobs are private to one session and are stopped BEFORE agent_settled reaches the host (or a
 * goal starts its next round). Nothing may keep writing after the chat becomes idle. Children
 * get separate instances, and Stop/shutdown also clean up explicitly, including between tools.
 */
import { randomUUID } from "node:crypto";
import {
  createBashToolDefinition,
  createLocalBashOperations,
  type AgentToolResult,
  type AgentToolUpdateCallback,
  type BashOperations,
  type BashToolDetails,
  type ExtensionAPI,
  type ToolDefinition,
} from "@earendil-works/pi-coding-agent";

export const BASH_JOB_TOOL_NAME = "bash_job";
export const BASH_CHECKIN_SECONDS = 60;
const MAX_RUNNING_JOBS = 8;
const MAX_RETAINED_JOBS = 32;

type ShellResult = AgentToolResult<BashToolDetails | undefined>;
type JobStatus = "running" | "completed" | "failed" | "stopped";
interface Job {
  id: string;
  startedAt: number;
  lastOutputAt?: number;
  outputBytes: number;
  observedBytes: number;
  controller: AbortController;
  latest: ShellResult;
  status: JobStatus;
  done: Promise<void>;
  updates: Set<AgentToolUpdateCallback<BashToolDetails | undefined>>;
}

const BASH_JOB_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["jobId", "action"],
  properties: {
    jobId: { type: "string", description: "The job id returned by bash in this session." },
    action: { type: "string", enum: ["status", "wait", "stop"], description: "Inspect progress, wait for completion, or stop the command's process group." },
    waitSeconds: { type: "number", minimum: 0, description: `Seconds to wait (default ${BASH_CHECKIN_SECONDS}, capped at ${BASH_CHECKIN_SECONDS}). Only used for wait.` },
  },
} as const;

const OUTPUT_SCHEMA = {
  type: "object",
  required: ["output", "truncated", "exit_code", "wall_time_seconds", "status"],
  properties: {
    output: { type: "string" },
    truncated: { type: "boolean" },
    full_output_path: { type: "string" },
    exit_code: { type: ["number", "null"] },
    wall_time_seconds: { type: "number" },
    status: { type: "string", enum: ["running", "completed", "failed", "stopped"] },
    job_id: { type: "string" },
  },
} as const;

/** This cap is enforced in code even if a caller bypasses schema validation. */
function waitSeconds(value: unknown): number {
  if (value === undefined) return BASH_CHECKIN_SECONDS;
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new Error("The check-in wait must be a finite, non-negative number of seconds.");
  }
  return Math.min(value, BASH_CHECKIN_SECONDS);
}

export interface BashJobsController {
  /** Supplied as an SDK override, keeping bash on the ordinary tool denylist and shell policy. */
  tool(cwd: string): ToolDefinition;
  stopAll(): Promise<void>;
}

/** `operations` is an injectable shell backend for tests; production always uses Pi's own. */
export function createBashJobsExtension(operations: BashOperations = createLocalBashOperations()) {
  const jobs = new Map<string, Job>();

  async function stopAll(): Promise<void> {
    const running = [...jobs.values()].filter((job) => job.status === "running");
    for (const job of running) job.controller.abort();
    await Promise.all(running.map((job) => job.done));
  }

  function result(job: Job, includeJob: boolean): AgentToolResult<unknown> {
    const elapsed = Math.round((Date.now() - job.startedAt) / 100) / 10;
    const quiet = Math.floor((Date.now() - (job.lastOutputAt ?? job.startedAt)) / 1000);
    const newBytes = job.outputBytes - job.observedBytes;
    job.observedBytes = job.outputBytes;
    const { latest, status } = job;
    const output = latest.content.filter((part) => part.type === "text").map((part) => part.text).join("\n");
    const structured = latest.structuredContent as { output?: string; truncated?: boolean; exit_code?: number; full_output_path?: string } | undefined;
    const fullOutputPath = latest.details?.fullOutputPath ?? structured?.full_output_path;
    const summary = status === "running"
      ? `Job ${job.id} is still running after ${elapsed} seconds; it has NOT completed.\n${newBytes > 0 ? `${newBytes} new output bytes since the last check-in.` : "No new output since the last check-in."} ${job.lastOutputAt ? `Last output ${quiet} seconds ago.` : `No output yet (${quiet} seconds).`}\nUse bash_job with this jobId to inspect (status), wait (up to ${BASH_CHECKIN_SECONDS} seconds), or stop it. Do not rerun the command. If progress stalls, investigate or stop it rather than repeatedly waiting.`
      : `Job ${job.id} ${status} after ${elapsed} seconds.`;
    return {
      ...latest,
      content: includeJob ? [...latest.content, { type: "text", text: summary }] : latest.content,
      details: includeJob
        ? { ...latest.details, shellJob: { id: job.id, status, elapsedSeconds: elapsed, quietSeconds: quiet, outputBytes: job.outputBytes, newOutputBytes: newBytes } }
        : latest.details,
      structuredContent: {
        output: structured?.output ?? output,
        truncated: structured?.truncated ?? latest.details?.truncation?.truncated ?? false,
        ...(fullOutputPath ? { full_output_path: fullOutputPath } : {}),
        exit_code: structured?.exit_code ?? null,
        wall_time_seconds: elapsed,
        status,
        ...(includeJob ? { job_id: job.id } : {}),
      },
    };
  }

  async function wait(job: Job, seconds: number, signal?: AbortSignal, onUpdate?: AgentToolUpdateCallback<BashToolDetails | undefined>): Promise<void> {
    if (job.status !== "running") return;
    // A wait's cancellation stops its job; the original run signal also remains linked after
    // a yield, so Stop can cancel commands while the model is thinking or using another tool.
    const onAbort = () => { job.controller.abort(); };
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) onAbort();
    if (onUpdate) job.updates.add(onUpdate);
    let timer: NodeJS.Timeout | undefined;
    try {
      if (onUpdate) onUpdate(job.latest);
      await Promise.race([job.done, new Promise<void>((resolve) => { timer = setTimeout(resolve, seconds * 1000); })]);
    } finally {
      if (timer) clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      if (onUpdate) job.updates.delete(onUpdate);
    }
  }

  function tool(cwd: string): ToolDefinition {
    const base = createBashToolDefinition(cwd);
    return {
      name: base.name,
      label: base.label,
      promptSnippet: base.promptSnippet,
      constrainedSampling: base.constrainedSampling,
      description: `${base.description} Commands return control after at most ${BASH_CHECKIN_SECONDS} seconds. If still running, returns a job id and output so far; use bash_job to inspect, wait or stop that same command. timeout is an optional hard execution deadline, not the check-in wait.`,
      promptGuidelines: [
        ...(base.promptGuidelines ?? []),
        `Shell calls yield after at most ${BASH_CHECKIN_SECONDS} seconds. A running job is not success: use bash_job to check, wait or stop it; never launch a duplicate. If check-ins show no progress, inspect the cause or stop it instead of blindly waiting.`,
        "Finish or stop foreground shell jobs before concluding your run. Unfinished jobs are stopped when the run ends; use the project's Run command for long-lived servers.",
      ],
      parameters: {
        ...base.parameters,
        properties: {
          ...base.parameters.properties,
          yieldTimeout: { type: "number", minimum: 0, description: `Seconds to wait before returning a running job (default ${BASH_CHECKIN_SECONDS}, capped at ${BASH_CHECKIN_SECONDS}). Use 0 for an immediate check-in.` },
        },
      },
      outputSchema: OUTPUT_SCHEMA,
      async execute(toolCallId, params: unknown, signal, onUpdate, ctx) {
        const input = params as { command: string; timeout?: number; yieldTimeout?: number };
        const seconds = waitSeconds(input.yieldTimeout);
        if (signal?.aborted) throw new Error("Command aborted");
        if ([...jobs.values()].filter((job) => job.status === "running").length >= MAX_RUNNING_JOBS) {
          throw new Error(`Already running ${MAX_RUNNING_JOBS} shell jobs. Wait for or stop an existing job before starting another.`);
        }
        // Retain only a bounded number of finished jobs; never evict a live command.
        for (const [id, job] of jobs) {
          if (jobs.size < MAX_RETAINED_JOBS) break;
          if (job.status !== "running") jobs.delete(id);
        }
        const job: Job = {
          id: randomUUID(), startedAt: Date.now(), outputBytes: 0, observedBytes: 0,
          controller: new AbortController(), latest: { content: [], details: undefined },
          status: "running", done: Promise.resolve(), updates: new Set(),
        };
        jobs.set(job.id, job);
        const onAbort = () => { job.controller.abort(); };
        signal?.addEventListener("abort", onAbort, { once: true });
        // Record raw bytes rather than comparing tail text: a chatty command can repeatedly
        // print the same bounded tail while still making progress.
        const command = createBashToolDefinition(cwd, { operations: {
          exec: (command, folder, options) => operations.exec(command, folder, {
            ...options,
            onData: (data) => {
              job.outputBytes += data.length;
              job.lastOutputAt = Date.now();
              options.onData(data);
            },
          }),
        } });
        job.done = Promise.resolve().then(() => command.execute(toolCallId, input, job.controller.signal, (partial) => {
          job.latest = partial;
          for (const update of job.updates) update(partial);
        }, ctx)).then((final) => {
          job.latest = final;
          job.status = job.controller.signal.aborted ? "stopped" : final.isError ? "failed" : "completed";
        }, (error: unknown) => {
          job.latest = {
            content: [{ type: "text", text: error instanceof Error ? error.message : String(error) }],
            details: job.latest.details,
            isError: true,
          };
          job.status = job.controller.signal.aborted ? "stopped" : "failed";
        }).finally(() => { signal?.removeEventListener("abort", onAbort); });
        // The originating signal is already linked for the whole job lifetime. Do not add a
        // second listener during its initial wait (parallel calls can share one run signal).
        await wait(job, seconds, undefined, onUpdate);
        const yielded = job.status === "running";
        const output = result(job, yielded);
        if (!yielded) jobs.delete(job.id);
        return output;
      },
    };
  }

  const factory = (pi: ExtensionAPI) => {
    pi.registerTool({
      name: BASH_JOB_TOOL_NAME,
      label: "Shell job",
      description: `Inspect, wait for, or stop a shell command that bash returned as still running. Jobs belong to this session only. Every wait returns within ${BASH_CHECKIN_SECONDS} seconds, with output, elapsed time and time since the last output, so you can investigate stalled commands.`,
      promptSnippet: "inspect, wait for or stop a running shell job",
      // Check-ins consume a per-job progress cursor. Serialize them so sibling calls do not
      // race that cursor or attach many simultaneous waits to the same run's abort signal.
      executionMode: "sequential",
      parameters: BASH_JOB_PARAMS,
      outputSchema: OUTPUT_SCHEMA,
      async execute(_toolCallId, params: unknown, signal, onUpdate) {
        const input = params as { jobId?: unknown; action?: unknown; waitSeconds?: unknown };
        if (!input || typeof input.jobId !== "string" || !["status", "wait", "stop"].includes(String(input.action))) {
          throw new Error("Provide a shell jobId and an action: status, wait or stop.");
        }
        const seconds = input.action === "wait" ? waitSeconds(input.waitSeconds) : BASH_CHECKIN_SECONDS;
        const job = jobs.get(input.jobId);
        if (!job) throw new Error("That shell job is no longer available in this session. It may have finished or the worker restarted; do not assume the command succeeded.");
        if (input.action === "stop") job.controller.abort();
        if (input.action !== "status") await wait(job, seconds, signal, onUpdate);
        const output = result(job, true);
        // An intentional stop succeeded even though the shell itself reports cancellation.
        if (input.action === "stop" && job.status === "stopped") output.isError = false;
        return output;
      },
    });
    pi.on("agent_settled", stopAll);
    pi.on("session_shutdown", stopAll);
  };
  const controller: BashJobsController = { tool, stopAll };
  return { factory, controller };
}
