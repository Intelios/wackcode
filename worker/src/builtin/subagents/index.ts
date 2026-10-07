/**
 * Built-in sub-agents — the `subagent` tool lets the chat's agent hand a self-contained task to
 * a child agent with its own context window, or run several at once. Modelled on Pi's
 * `examples/extensions/subagent` (MIT; single and parallel modes, output format, caps) and the
 * foreground model of `pi-subagents` (MIT), with WackCode-owned background job lifetimes.
 * Children run in-process as their own Pi sessions
 * (see `subagent-runner.ts`) rather than as spawned `pi` processes, which WackCode's worker
 * cannot start. Agents come from Settings, not from `.md` files on disk.
 *
 * Unlike the other built-ins this one can be switched off, and is off by default because every
 * child is extra model usage. The factory always loads; while switched off its tool is simply
 * kept out of the active set (`inactiveTools`), so turning it on or off never restarts a chat.
 */
import { randomUUID } from "node:crypto";
import type { ExtensionAPI, InlineExtension } from "@earendil-works/pi-coding-agent";
import type { ExecutionPolicyConfig, SubagentDetails, SubagentRuntimeConfig, SubagentSpec, TaskMode } from "../../protocol.js";
import type { BuiltinHost, SubagentOutcome, SubagentRunRequest } from "../host.js";
import { MAX_CARD_OUTPUT, MAX_MODEL_OUTPUT, cardUsage, createDetails, createThrottle, emptyUsage, modelContent, progressText, recordActivity, snapshotDetails, summarizeActivity, truncate, addUsage } from "./details.js";
import { DEFAULT_EXECUTION_POLICY, isReadOnlyPlanning } from "../../execution-policy.js";
import { resolveSubagentSpec } from "./access.js";
import { readOnlyGuard } from "./guard.js";
import { SUBAGENT_PROMPT_SNIPPET, subagentDescription, subagentGuidelines } from "./prompt.js";
import { normalizeSubagentParams, subagentParams } from "./schema.js";
import { WEB_FETCH_TOOL_NAME } from "../web-fetch/index.js";
import { SUBAGENT_JOB_TOOL_NAME, SUBAGENT_TOOL_LABEL, SUBAGENT_TOOL_NAME, resolveChildTools } from "./types.js";
import { SubagentJobs } from "./jobs.js";
import { BACKGROUND_SUBAGENT_ENTRY, type BackgroundSubagentCall } from "./state.js";

interface Job {
  id: string;
  call: BackgroundSubagentCall;
  index: number;
  background: boolean;
  controller: AbortController;
  done: Promise<void>;
  outcome?: SubagentOutcome;
  delivered: boolean;
  reserved: boolean;
}

export interface SubagentsController {
  configure(config: SubagentRuntimeConfig | null): Promise<void>;
  inactiveTools(): string[];
  refresh(): void;
  stopAll(interrupted?: boolean): Promise<void>;
  activeCount(): number;
  pendingCount(): number;
  hasWork(): boolean;
  waiting(): boolean;
  calls(): BackgroundSubagentCall[];
  /** Claim only completed, unreserved results. Explicit waits own their targets instead. */
  takeResults(): { jobIds: string[]; text: string } | undefined;
}

const JOB_PARAMS = {
  type: "object", additionalProperties: false, required: ["action"],
  properties: {
    action: { type: "string", enum: ["status", "wait", "stop"], description: "Inspect, wait for all targeted children, or stop them." },
    jobIds: { type: "array", minItems: 1, maxItems: 8, items: { type: "string" }, description: "IDs returned by a background subagent launch. Omit to target all outstanding children at invocation time." },
  },
} as const;

/** Background jobs own their signals, never an already-settled parent tool's callbacks. */
export function createSubagentsExtension(host: BuiltinHost, currentMode: () => TaskMode, webFetch: InlineExtension, executionPolicy: () => ExecutionPolicyConfig = () => DEFAULT_EXECUTION_POLICY) {
  let config: SubagentRuntimeConfig | null = null;
  let pi: ExtensionAPI | undefined;
  let registeredKey: string | undefined;
  let stopped = false;
  let waiting = false;
  const failedCalls = new Set<string>();
  const interruptedJobs = new Set<string>();
  const pool = new SubagentJobs();
  const jobs = new Map<string, Job>();
  const calls = new Map<string, BackgroundSubagentCall>();
  const cardCache = new WeakMap<BackgroundSubagentCall, { json: string; card: BackgroundSubagentCall }>();
  const changed = () => host.subagentsChanged?.();
  const persist = (call: BackgroundSubagentCall) => {
    try { pi?.appendEntry(BACKGROUND_SUBAGENT_ENTRY, structuredClone(call)); }
    catch (error) { host.notice(`Could not save the sub-agent outcome: ${host.redact(error instanceof Error ? error.message : String(error))}`, "error"); }
  };
  const outstanding = (job: Job) => !job.outcome || !job.delivered;
  const textFor = (targets: Job[]) => targets.map((job) => {
    const result = job.call.details.results[job.index];
    if (!job.outcome) return `Job ${job.id}: ${result.agent} ${result.status}. Task: ${result.task}${result.model ? ` · ${result.model}` : ""}.`;
    return `Job ${job.id}: ` + modelContent({ v: 1, mode: "single", results: [result] }, [job.outcome?.output]);
  }).join("\n\n");

  async function stopAll(interrupted = false): Promise<void> {
    const hadJobs = jobs.size > 0;
    stopped = true;
    for (const job of jobs.values()) {
      job.delivered = true;
      if (interrupted && !job.outcome) interruptedJobs.add(job.id);
    }
    await pool.stopAll();
    if (hadJobs) changed();
  }

  async function execute(toolCallId: string, params: unknown, signal: AbortSignal | undefined, onUpdate: ((result: { content: { type: "text"; text: string }[]; details: unknown }) => void) | undefined) {
    const current = config;
    if (!current || !current.agents.length) throw new Error("Sub-agents are switched off in Settings.");
    const parsed = normalizeSubagentParams(params);
    if (!parsed.ok) throw new Error(parsed.error);
    if (signal?.aborted) throw new Error("Sub-agent launch was stopped.");
    const mode = currentMode();
    const policy = executionPolicy();
    const specs = parsed.tasks.map((input) => {
      const saved = current.agents.find((agent) => agent.name === input.agent);
      if (!saved) throw new Error(`There is no sub-agent called '${input.agent}'. Available: ${current.agents.map((agent) => agent.name).join(", ") || "none"}.`);
      if (isReadOnlyPlanning(mode, policy) && !saved.readOnly) throw new Error(`Plan mode only runs read-only sub-agents, and ${saved.name} can edit files. Use a read-only sub-agent while planning.`);
      return resolveSubagentSpec(structuredClone(saved), mode, policy);
    });
    pool.admit(parsed.tasks.length);
    stopped = false;
    // Retain a bounded in-memory cache; durable outcomes remain in their branch entries.
    for (const [id, job] of jobs) if (jobs.size + parsed.tasks.length > 32 && job.outcome && job.delivered && !job.reserved) jobs.delete(id);
    for (const [id, call] of calls) if (![...jobs.values()].some((job) => job.call === call)) calls.delete(id);
    const details = createDetails(parsed.mode, parsed.tasks.map((input, index) => ({ input, readOnly: specs[index].readOnly })));
    if (parsed.background) details.background = true;
    const call: BackgroundSubagentCall = { v: 1, toolCallId, details };
    const outputs: (string | undefined)[] = [];
    const updates = createThrottle(() => {
      if (parsed.background) changed();
      else onUpdate?.({ content: [{ type: "text", text: progressText(details) }], details: snapshotDetails(details) });
    });
    const available = [...host.childToolNames()];
    const launched: Job[] = [];
    for (const [index, input] of parsed.tasks.entries()) {
      const spec = specs[index];
      const result = details.results[index];
      const controller = new AbortController();
      const job: Job = { id: randomUUID(), call, index, background: parsed.background === true, controller, done: Promise.resolve(), delivered: !parsed.background, reserved: false };
      if (parsed.background) result.jobId = job.id;
      const tools = resolveChildTools(spec.tools, spec.readOnly, available);
      const request: SubagentRunRequest = {
        toolCallId, index, spec, task: input.task, tools, signal: controller.signal,
        extensions: [...(spec.readOnly ? [readOnlyGuard()] : []), ...(tools.includes(WEB_FETCH_TOOL_NAME) ? [webFetch] : [])],
        observer: {
          started: (model) => { result.model = model; updates.schedule(); },
          tool: (name, args) => { recordActivity(result, summarizeActivity(name, args)); updates.schedule(); },
          usage: (usage, turns) => { result.usage = cardUsage(usage, turns); updates.schedule(); },
        },
      };
      // Capture the connection and model before a queued child waits for a slot.
      const prepared = !spec.unavailable ? host.prepareSubagent?.(request) : undefined;
      const run = !spec.unavailable ? prepared?.run ?? (() => host.runSubagent(request)) : undefined;
      jobs.set(job.id, job);
      launched.push(job);
      job.done = pool.enqueue(controller, async () => {
        let outcome: SubagentOutcome;
        try {
          if (controller.signal.aborted) {
            prepared?.dispose();
            outcome = { status: "aborted", output: "", usage: emptyUsage(), turns: 0 };
          }
          else if (spec.unavailable) outcome = { status: "failed", output: "", error: spec.unavailable, usage: emptyUsage(), turns: 0 };
          else {
            result.status = "running";
            result.startedAt = Date.now();
            updates.schedule();
            outcome = await run!();
          }
        } catch (error) {
          outcome = { status: controller.signal.aborted ? "aborted" : "failed", output: "", error: error instanceof Error ? error.message : String(error), usage: emptyUsage(), turns: 0 };
        }
        outcome.output = truncate(host.redact(outcome.output), MAX_MODEL_OUTPUT).text;
        if (outcome.error) outcome.error = truncate(host.redact(outcome.error), MAX_CARD_OUTPUT).text;
        job.outcome = outcome;
        outputs[index] = outcome.output;
        const card = truncate(outcome.output, MAX_CARD_OUTPUT);
        result.status = interruptedJobs.delete(job.id) ? "interrupted" : outcome.status;
        if (card.text) result.output = card.text;
        if (card.truncated) result.outputTruncated = true;
        if (outcome.error) result.error = outcome.error;
        if (result.status === "interrupted") result.error = "The worker stopped before this sub-agent finished. It was not restarted.";
        if (outcome.transcript) result.transcript = outcome.transcript;
        result.usage = cardUsage(outcome.usage, outcome.turns);
        result.endedAt = Date.now();
        if (job.background) {
          if (stopped) job.delivered = true;
          try { host.recordSubagentUsage?.(outcome.usage, spec); }
          catch (error) { host.notice(`Could not save sub-agent usage: ${host.redact(error instanceof Error ? error.message : String(error))}`, "error"); }
          persist(call);
          changed();
        }
        updates.schedule();
      });
    }
    if (parsed.background) {
      calls.set(toolCallId, call);
      persist(call);
      changed();
      return { content: [{ type: "text" as const, text: `Background sub-agents started. ${launched.map((job) => `${job.call.details.results[job.index].agent}: jobId ${job.id}`).join("; ")}. Continue independent work. Results arrive automatically; use subagent_job to inspect, wait or stop.` }], details: snapshotDetails(details) };
    }
    const abort = () => { for (const job of launched) job.controller.abort(); };
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) abort();
    try { await Promise.all(launched.map((job) => job.done)); }
    finally { signal?.removeEventListener("abort", abort); updates.cancel(); }
    if (!details.results.some((result) => result.status === "done")) failedCalls.add(toolCallId);
    return { content: [{ type: "text" as const, text: modelContent(details, outputs) }], details, usage: launched.reduce((total, job) => addUsage(total, job.outcome?.usage), emptyUsage()) };
  }

  async function control(params: unknown, signal?: AbortSignal) {
    const input = params as { action?: unknown; jobIds?: unknown } | null;
    if (!input || !["status", "wait", "stop"].includes(String(input.action))) throw new Error("Provide an action: status, wait or stop.");
    if (input.jobIds !== undefined && (!Array.isArray(input.jobIds) || !input.jobIds.length || input.jobIds.length > 8 || !input.jobIds.every((id) => typeof id === "string"))) throw new Error("jobIds must contain between 1 and 8 job IDs.");
    const targets = input.jobIds === undefined
      ? [...jobs.values()].filter((job) => job.background && outstanding(job))
      : [...new Set(input.jobIds as string[])].map((id) => {
        const job = jobs.get(id);
        if (!job?.background) throw new Error("That sub-agent job is no longer available in this worker. It may have restarted; do not assume the task succeeded.");
        return job;
      });
    if (input.action === "status") return { content: [{ type: "text" as const, text: targets.length ? textFor(targets) : "No outstanding sub-agent jobs." }] };
    // Reserve before awaiting anything: completion delivery cannot steal these results.
    for (const job of targets) job.reserved = true;
    waiting = input.action === "wait" && targets.some((job) => !job.outcome);
    const abort = () => { for (const job of targets) job.controller.abort(); };
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted || input.action === "stop") abort();
    changed();
    try {
      await Promise.all(targets.map((job) => job.done));
      for (const job of targets) job.delivered = true;
      return { content: [{ type: "text" as const, text: targets.length ? textFor(targets) : "No outstanding sub-agent jobs." }] };
    } finally {
      waiting = false;
      for (const job of targets) job.reserved = false;
      signal?.removeEventListener("abort", abort);
      changed();
    }
  }

  const register = () => {
    if (!pi) return;
    const agents = (config?.agents ?? []).map((spec) => resolveSubagentSpec(spec, currentMode(), executionPolicy()));
    const definition = { name: SUBAGENT_TOOL_NAME, label: SUBAGENT_TOOL_LABEL, description: subagentDescription(agents), promptSnippet: SUBAGENT_PROMPT_SNIPPET, promptGuidelines: subagentGuidelines(config?.trigger ?? "on_request", isReadOnlyPlanning(currentMode(), executionPolicy())), parameters: subagentParams(agents.map((agent) => agent.name)) };
    const key = JSON.stringify(definition);
    if (key === registeredKey) return;
    registeredKey = key;
    pi.registerTool({ ...definition, parameters: definition.parameters as never, executionMode: "sequential", execute: (id, params: unknown, signal, onUpdate) => execute(id, params, signal, onUpdate as never) as never });
  };
  const factory = (bound: ExtensionAPI) => {
    pi = bound;
    registeredKey = undefined;
    register();
    pi.registerTool({ name: SUBAGENT_JOB_TOOL_NAME, label: "Sub-agent jobs", description: "Inspect, wait for, or stop background sub-agents in this chat. wait returns once every targeted child finishes, fails or is stopped. Omit jobIds for all outstanding children at invocation time. Jobs never restart automatically.", parameters: JOB_PARAMS, executionMode: "sequential", execute: (_id, params: unknown, signal) => control(params, signal) as never });
    bound.on("tool_result", (event) => event.toolName === SUBAGENT_TOOL_NAME && failedCalls.delete(event.toolCallId) ? { isError: true } : undefined);
    bound.on("session_shutdown", () => stopAll(true));
  };
  const controller: SubagentsController = {
    refresh: register,
    async configure(next) {
      if (!next) await stopAll();
      config = next;
      if (next) pool.configure(next.maxConcurrency);
      register();
    },
    inactiveTools: () => config && config.agents.length ? [] : [SUBAGENT_TOOL_NAME, SUBAGENT_JOB_TOOL_NAME],
    stopAll,
    activeCount: () => [...jobs.values()].filter((job) => !job.outcome).length,
    pendingCount: () => [...jobs.values()].filter((job) => job.background && job.outcome && !job.delivered).length,
    hasWork: () => [...jobs.values()].some((job) => !job.outcome || (job.background && !job.delivered)),
    waiting: () => waiting,
    calls: () => [...calls.values()].map((call) => {
      const details = snapshotDetails(call.details);
      const json = JSON.stringify(details);
      const cached = cardCache.get(call);
      if (cached?.json === json) return cached.card;
      const card = { ...call, details };
      cardCache.set(call, { json, card });
      return card;
    }),
    takeResults() {
      if (stopped) return undefined;
      const targets = [...jobs.values()].filter((job) => job.background && job.outcome && !job.delivered && !job.reserved);
      if (!targets.length) return undefined;
      for (const job of targets) job.delivered = true;
      return { jobIds: targets.map((job) => job.id), text: textFor(targets) };
    },
  };
  return { factory, controller };
}
