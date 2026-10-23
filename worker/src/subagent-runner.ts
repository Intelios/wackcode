/**
 * Runs sub-agents as in-process Pi sessions inside the chat's worker: the only sub-agent code
 * that touches Pi or credentials. A child gets an in-memory session (never written to disk), a
 * resource loader that discovers nothing (the project's context files still load, like the
 * parent's), an explicit tool allowlist, and only the extensions the built-in hands it.
 *
 * Pi's own subagent example spawns `pi` processes instead; that cannot work here — the worker
 * has no `pi` binary, `process.argv[1]` is the worker itself, and provider keys are stripped
 * from the environment on purpose.
 *
 * Team chats keep finished children alive: a request carrying `jobId` retains its session in
 * `kept` (at most MAX_KEPT_CHILDREN, oldest evicted) instead of disposing it, so a request
 * carrying `continuation` reuses its in-memory history with the follow-up. Each round rebuilds
 * the Pi session from the current role, tools, policy and connection, so keeping context never
 * keeps access the user has since revoked. Continuations re-key the record to the new call's jobId, so
 * a jobId always names the child's latest round; dropped or missing ones fail the launch, and
 * the extension reports that as "session has ended".
 */
import { join } from "node:path";
import { trackSession } from "./usage.js";
import type { Usage } from "@earendil-works/pi-ai";
import type { SubagentOutcome, SubagentRunRequest } from "./builtin/host.js";
import { addUsage, emptyUsage } from "./builtin/subagents/details.js";
import { KeptSessions, type KeptChild } from "./builtin/subagents/kept.js";
import { BASH_JOB_TOOL_NAME, createBashJobsExtension } from "./builtin/bash-jobs.js";
import { createImageBudgetExtension } from "./builtin/image-budget.js";
import {
  type ModelRuntime,
  type PiModel,
  createModelRuntime,
  findModel,
  workerSettings,
} from "./model-runtime.js";
import type { SubagentProvider, ThinkingLevel, WorkerProvider } from "./protocol.js";

type PiModule = typeof import("@earendil-works/pi-coding-agent");
type AgentSession = Awaited<ReturnType<PiModule["createAgentSession"]>>["session"];

/** Retained finished children per chat (Team chats only); the least recently used goes first. */
const MAX_KEPT_CHILDREN = 8;

/** What a later round needs of a kept child, beyond the session itself. */
interface KeptContext {
  spec: SubagentRunRequest["spec"];
  /** Earlier rounds' raw history can still quote a credential after its connection changes. */
  providers: SubagentProvider[];
}

/** What the side panel's transcript stream (`subagent-stream.ts`) follows of a running child. */
export interface ChildTranscriptHooks {
  /** The child's session exists; `messages` reads what it holds so far. */
  started(messages: () => readonly unknown[]): void;
  /** One of the child's own session events. */
  event(event: Record<string, unknown>): void;
  /** The child's run is over: everything it said, read just before its session is disposed. */
  ended(messages: readonly unknown[]): void;
}

export interface SubagentRunnerContext {
  pi: PiModule;
  cwd: string;
  agentDir: string;
  /** The chat's own connection and runtime: reused so one process never runs two OAuth
   *  refreshers on the same auth file. */
  parentProvider: WorkerProvider;
  parentRuntime: ModelRuntime;
  parentModel(): PiModel | undefined;
  parentThinkingLevel(): ThinkingLevel;
  safeError(error: unknown): string;
}

interface ResolvedModel {
  runtime: ModelRuntime;
  model: PiModel;
  thinkingLevel: ThinkingLevel;
  label: string;
}

function lastAssistant(messages: readonly unknown[]): Record<string, unknown> | undefined {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as Record<string, unknown> | undefined;
    if (message?.role === "assistant") return message;
  }
  return undefined;
}

function directoryName(providerId: string): string {
  return providerId.replace(/[^A-Za-z0-9_-]/g, "_") || "provider";
}

export class SubagentRunner {
  private providers = new Map<string, SubagentProvider>();
  /** Runtimes for connections other than the chat's, built on first use. */
  private runtimes = new Map<string, Promise<ModelRuntime>>();
  private readonly live = new Set<AgentSession>();
  /** Queued/running children keep the credentials they captured redacted after Settings changes. */
  private readonly retained = new Map<SubagentRunRequest, SubagentProvider[]>();
  /** Finished Team children kept for `job` continuations (`builtin/subagents/kept.ts`). */
  private readonly kept = new KeptSessions<AgentSession>(MAX_KEPT_CHILDREN, (session) => session.dispose());
  /** A round already admitted may finish after Team is turned off, but must not keep a session. */
  private retentionGeneration = 0;

  constructor(private readonly context: SubagentRunnerContext) {}

  /**
   * Replace the connections sub-agents may use. Cached runtimes are dropped rather than
   * diffed: rebuilding is cheap, children already running keep the runtime they started with,
   * and a re-sign-in or new key is picked up without any change detection.
   */
  setProviders(providers: SubagentProvider[]): void {
    this.providers = new Map(providers.map((entry) => [entry.provider.id, entry]));
    this.runtimes.clear();
  }

  /** Every credential sub-agents hold, for redaction. */
  credentials(): { apiKeys: string[]; stored: { providerId: string; authPath: string }[] } {
    const apiKeys: string[] = [];
    const stored: { providerId: string; authPath: string }[] = [];
    const entries = [...this.providers.values(), ...[...this.retained.values()].flat()];
    for (const record of this.kept.records()) {
      entries.push(...(record.data as KeptContext).providers);
    }
    for (const entry of entries) {
      if (entry.apiKey) apiKeys.push(entry.apiKey);
      if (entry.authPath) stored.push({ providerId: entry.provider.id, authPath: entry.authPath });
    }
    return { apiKeys, stored };
  }

  /** Stop every running child on worker shutdown, in addition to the job manager's signals. */
  async abortAll(): Promise<void> {
    this.dropKept();
    await Promise.allSettled([...this.live].map((child) => child.abort()));
  }

  /** Dispose all retained children. Sessions are in-memory, so there is nothing to flush. */
  dropKept(): void {
    this.retentionGeneration += 1;
    this.kept.clear();
  }

  /** What a `job` continuation may resolve to. Aliases keep an older jobId working. */
  keptSubagent(jobId: string): { spec: KeptContext["spec"]; rounds: number; busy: boolean; jobId: string } | undefined {
    const record = this.kept.get(jobId);
    return record ? { spec: (record.data as KeptContext).spec, rounds: record.rounds, busy: record.busy, jobId: record.jobId } : undefined;
  }

  private runtimeFor(providerId: string): Promise<{ runtime: ModelRuntime; provider: WorkerProvider }> {
    const { context } = this;
    if (providerId === context.parentProvider.id) {
      return Promise.resolve({ runtime: context.parentRuntime, provider: context.parentProvider });
    }
    const entry = this.providers.get(providerId);
    if (!entry) return Promise.reject(new Error("That sub-agent's connection is no longer available. Check Settings → Sub-agents."));
    let runtime = this.runtimes.get(providerId);
    if (!runtime) {
      runtime = createModelRuntime(context.pi, entry.provider, join(context.agentDir, "subagents", directoryName(providerId)), {
        apiKey: entry.apiKey,
        authPath: entry.authPath,
      });
      // A failed build is retried next time rather than cached.
      runtime.catch(() => this.runtimes.delete(providerId));
      this.runtimes.set(providerId, runtime);
    }
    return runtime.then((built) => ({ runtime: built, provider: entry.provider }));
  }

  private async resolveModel(request: SubagentRunRequest): Promise<ResolvedModel> {
    const { context } = this;
    const choice = request.spec.model;
    if (!choice) {
      const model = context.parentModel();
      if (!model) throw new Error("The chat has no model selected.");
      return {
        runtime: context.parentRuntime,
        model,
        thinkingLevel: context.parentThinkingLevel(),
        label: `${context.parentProvider.name} · ${model.name ?? model.id}`,
      };
    }
    const { runtime, provider } = await this.runtimeFor(choice.providerId);
    const model = await findModel(runtime, provider, choice.modelId);
    if (!model) throw new Error(`${request.spec.name}'s model (${provider.name} / ${choice.modelId}) is not available.`);
    return {
      runtime,
      model,
      thinkingLevel: choice.thinkingLevel,
      label: `${provider.name} · ${model.name ?? model.id}`,
    };
  }

  prepare(request: SubagentRunRequest): { run(transcript?: ChildTranscriptHooks): Promise<SubagentOutcome>; dispose(): void } {
    // Continuations capture current credentials too, and keep them redacted until the host
    // finishes sanitizing the round, even if Team is switched off while it runs.
    this.retained.set(request, this.redactionProviders(request));
    const generation = this.retentionGeneration;
    const resolved = this.resolveModel(request);
    // It may reject while queued; observe it now, and report the same failure when scheduled.
    void resolved.catch(() => undefined);
    return {
      dispose: () => { this.retained.delete(request); },
      // The host releases AFTER redacting the output and capturing the final transcript.
      run: (transcript) => this.run(request, transcript, resolved, generation),
    };
  }

  private redactionProviders(request: SubagentRunRequest): SubagentProvider[] {
    const kept = request.continuation ? this.kept.get(request.continuation) : undefined;
    const previous = kept ? (kept.data as KeptContext).providers : [];
    const current = request.spec.model && this.providers.get(request.spec.model.providerId);
    const entries = current ? [...previous, current] : previous;
    return entries.filter((entry, index) => entries.findIndex((other) =>
      other.provider.id === entry.provider.id && other.apiKey === entry.apiKey && other.authPath === entry.authPath
    ) === index);
  }

  async run(request: SubagentRunRequest, transcript?: ChildTranscriptHooks, prepared?: Promise<ResolvedModel>, generation = this.retentionGeneration): Promise<SubagentOutcome> {
    let record: KeptChild<AgentSession> | undefined;
    if (request.continuation) {
      record = this.kept.get(request.continuation);
      if (!record) return { status: "failed", output: "", error: "That worker's session has ended. Launch a fresh worker and include the feedback in its brief.", usage: emptyUsage(), turns: 0 };
      if (record.busy) return { status: "failed", output: "", error: "That worker is still running. Wait for or stop it first.", usage: emptyUsage(), turns: 0 };
    }
    if (request.signal?.aborted) return { status: "aborted", output: "", usage: emptyUsage(), turns: 0 };
    // Pin the retained history while asynchronous model and resource setup runs as well.
    if (record) this.kept.begin(record, request.jobId);
    try {
      return await this.runRound(request, transcript, prepared, record, generation);
    } finally {
      if (record) this.kept.end(record);
    }
  }

  private async runRound(
    request: SubagentRunRequest,
    transcript: ChildTranscriptHooks | undefined,
    prepared: Promise<ResolvedModel> | undefined,
    record: KeptChild<AgentSession> | undefined,
    generation: number,
  ): Promise<SubagentOutcome> {
    const { pi, cwd, agentDir } = this.context;
    const resolved = await (prepared ?? this.resolveModel(request));
    const settingsManager = workerSettings(pi);
    // Private job ownership per child: one child can never inspect or stop another's commands.
    // bash_job accompanies an allowed bash, without widening the role's command allowlist.
    const bashJobs = createBashJobsExtension();
    const tools = request.tools.includes("bash") ? [...request.tools, BASH_JOB_TOOL_NAME] : request.tools;
    // Every no* flag stays on: a child loads nothing from settings, packages or the project's
    // own .pi/, only the extensions the built-in passed (e.g. the read-only guard). The role
    // prompt goes through the override hook, not `appendSystemPrompt`, which would read a file
    // if the prompt text happened to be a path.
    const resourceLoader = new pi.DefaultResourceLoader({
      cwd,
      agentDir,
      settingsManager,
      noExtensions: true,
      noSkills: true,
      noPromptTemplates: true,
      noThemes: true,
      noContextFiles: false,
      extensionFactories: [
        { name: "wackcode-bash-jobs", factory: bashJobs.factory, hidden: true },
        // A child reads files too; its images must not grow past a gateway's body cap either.
        { name: "wackcode-image-budget", factory: createImageBudgetExtension(), hidden: true },
        ...request.extensions
      ],
      appendSystemPromptOverride: (base) => [...base, request.spec.prompt],
    });
    await resourceLoader.reload();
    // `tools:` is a hard registry filter. That is exactly right for a child: it has no Settings
    // toggles to preserve, and nothing outside its role's list should exist for it at all.
    // Only the conversation survives a continuation. Rebuild the registry and extensions so
    // a disabled tool or restored read-only guard cannot remain active in an old Pi session.
    const sessionManager = record?.session.sessionManager ?? pi.SessionManager.inMemory(cwd);
    record?.session.dispose();
    const { session: child } = await pi.createAgentSession({
      cwd,
      agentDir,
      modelRuntime: resolved.runtime,
      model: resolved.model,
      thinkingLevel: resolved.thinkingLevel,
      tools,
      customTools: [bashJobs.controller.tool(cwd)],
      sessionManager,
      settingsManager,
      resourceLoader,
    });

    trackSession(child, `${request.toolCallId}:${request.index}`);
    // Keep old credentials in redaction for as long as the history that can quote them lives.
    const data: KeptContext = { spec: request.spec, providers: this.retained.get(request) ?? this.redactionProviders(request) };
    if (record) { record.session = child; record.data = data; }
    request.observer.started(resolved.label);
    try {
      const outcome = await this.prompt(child, request, bashJobs.controller.stopAll, transcript);
      if (request.jobId && generation === this.retentionGeneration) {
        if (!record) this.kept.put(request.jobId, { session: child, data });
      } else {
        child.dispose();
      }
      return outcome;
    } catch (error) {
      if (record) this.kept.drop(record);
      else child.dispose();
      throw error;
    }
  }

  /** One prompt round on a child session: launch or continuation. Owns abort wiring, the
   *  transcript hooks and the outcome classification. Never throws once the round starts:
   *  failures land in the outcome so usage still counts. */
  private async prompt(
    child: AgentSession,
    request: SubagentRunRequest,
    stopShell: () => Promise<void>,
    transcript?: ChildTranscriptHooks,
  ): Promise<SubagentOutcome> {
    const { signal, observer } = request;
    const usage: Usage = emptyUsage();
    let turns = 0;
    if (signal?.aborted) return { status: "aborted", output: "", usage, turns };

    this.live.add(child);
    transcript?.started(() => child.messages as unknown[]);
    const unsubscribe = child.subscribe((event) => {
      const value = event as unknown as Record<string, unknown>;
      transcript?.event(value);
      if (value.type === "tool_execution_start") {
        observer.tool(String(value.toolName ?? "tool"), value.args);
      } else if (value.type === "message_end") {
        const message = value.message as Record<string, unknown> | undefined;
        if (message?.role === "assistant") {
          addUsage(usage, message.usage as Partial<Usage> | undefined);
          turns += 1;
          observer.usage(usage, turns);
        }
      }
    });
    // Aborting rejects inside Pi as the request is torn down; that is the expected outcome, not
    // an error to surface (an unhandled rejection would become a worker_error banner).
    const onAbort = () => { child.abort().catch(() => undefined); };
    signal?.addEventListener("abort", onAbort, { once: true });

    let thrown: unknown;
    try {
      if (!signal?.aborted) await child.prompt(request.task, { expandPromptTemplates: false });
    } catch (error) {
      thrown = error;
    } finally {
      signal?.removeEventListener("abort", onAbort);
      await stopShell();
      unsubscribe();
      this.live.delete(child);
    }

    const output = child.getLastAssistantText() ?? "";
    const last = lastAssistant(child.messages as unknown[]);
    transcript?.ended([...(child.messages as unknown[])]);
    if (signal?.aborted || last?.stopReason === "aborted") {
      return { status: "aborted", output, usage, turns };
    }
    if (thrown !== undefined) {
      return { status: "failed", output, error: this.context.safeError(thrown), usage, turns };
    }
    if (last?.stopReason === "error") {
      const reason = typeof last.errorMessage === "string" && last.errorMessage ? last.errorMessage : "The model request failed.";
      return { status: "failed", output, error: this.context.safeError(reason), usage, turns };
    }
    return { status: "done", output, usage, turns };
  }

}
