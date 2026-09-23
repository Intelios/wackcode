/**
 * Runs sub-agents as in-process Pi sessions inside the chat's worker: the only sub-agent code
 * that touches Pi or credentials. A child gets an in-memory session (never written to disk), a
 * resource loader that discovers nothing (the project's context files still load, like the
 * parent's), an explicit tool allowlist, and only the extensions the built-in hands it.
 *
 * Pi's own subagent example spawns `pi` processes instead; that cannot work here — the worker
 * has no `pi` binary, `process.argv[1]` is the worker itself, and provider keys are stripped
 * from the environment on purpose.
 */
import { join } from "node:path";
import type { Usage } from "@earendil-works/pi-ai";
import type { SubagentOutcome, SubagentRunRequest } from "./builtin/host.js";
import { addUsage, emptyUsage } from "./builtin/subagents/details.js";
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
    for (const entry of this.providers.values()) {
      if (entry.apiKey) apiKeys.push(entry.apiKey);
      if (entry.authPath) stored.push({ providerId: entry.provider.id, authPath: entry.authPath });
    }
    return { apiKeys, stored };
  }

  /** Stop every running child (worker shutdown). The parent's abort signal normally does this. */
  async abortAll(): Promise<void> {
    await Promise.allSettled([...this.live].map((child) => child.abort()));
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

  async run(request: SubagentRunRequest): Promise<SubagentOutcome> {
    const { pi, cwd, agentDir, safeError } = this.context;
    const { signal, observer } = request;
    const usage: Usage = emptyUsage();
    let turns = 0;
    if (signal?.aborted) return { status: "aborted", output: "", usage, turns };

    const resolved = await this.resolveModel(request);
    const settingsManager = workerSettings(pi);
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
      extensionFactories: request.extensions,
      appendSystemPromptOverride: (base) => [...base, request.spec.prompt],
    });
    await resourceLoader.reload();
    // `tools:` is a hard registry filter. That is exactly right for a child: it has no Settings
    // toggles to preserve, and nothing outside its role's list should exist for it at all.
    const { session: child } = await pi.createAgentSession({
      cwd,
      agentDir,
      modelRuntime: resolved.runtime,
      model: resolved.model,
      thinkingLevel: resolved.thinkingLevel,
      tools: request.tools,
      sessionManager: pi.SessionManager.inMemory(cwd),
      settingsManager,
      resourceLoader,
    });

    this.live.add(child);
    const unsubscribe = child.subscribe((event) => {
      const value = event as unknown as Record<string, unknown>;
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
    observer.started(resolved.label);

    let thrown: unknown;
    try {
      if (!signal?.aborted) await child.prompt(request.task, { expandPromptTemplates: false });
    } catch (error) {
      thrown = error;
    } finally {
      signal?.removeEventListener("abort", onAbort);
      unsubscribe();
      this.live.delete(child);
    }

    const output = child.getLastAssistantText() ?? "";
    const last = lastAssistant(child.messages as unknown[]);
    child.dispose();
    if (signal?.aborted || last?.stopReason === "aborted") {
      return { status: "aborted", output, usage, turns };
    }
    if (thrown !== undefined) {
      return { status: "failed", output, error: safeError(thrown), usage, turns };
    }
    if (last?.stopReason === "error") {
      const reason = typeof last.errorMessage === "string" && last.errorMessage ? last.errorMessage : "The model request failed.";
      return { status: "failed", output, error: safeError(reason), usage, turns };
    }
    return { status: "done", output, usage, turns };
  }
}
