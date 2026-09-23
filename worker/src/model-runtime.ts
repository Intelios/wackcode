/**
 * Pi model runtimes for one connection. The chat's own session builds one at `init`; sub-agents
 * whose model lives on another connection build their own with the same code, so both paths
 * write models.json and hand over credentials identically.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { THINKING_LEVELS, type WorkerModel, type WorkerProvider } from "./protocol.js";

type PiModule = typeof import("@earendil-works/pi-coding-agent");
export type ModelRuntime = Awaited<ReturnType<PiModule["ModelRuntime"]["create"]>>;
export type PiModel = NonNullable<ReturnType<ModelRuntime["getModel"]>>;
export type SettingsManager = ReturnType<PiModule["SettingsManager"]["inMemory"]>;

export function modelDefinition(model: WorkerModel): Record<string, unknown> {
  const thinkingLevelMap = Object.fromEntries(
    THINKING_LEVELS.map((level) => {
      if (!model.thinkingLevels.includes(level)) return [level, null];
      const mapped = model.thinkingLevelMap[level];
      return [level, mapped === undefined ? (level === "off" ? null : level) : mapped];
    })
  );
  return {
    id: model.id,
    name: model.name,
    reasoning: model.reasoning,
    // Pi's own capability flag. Without "image", Pi replaces images with an "image omitted"
    // placeholder before the request is built, and `read` stops returning image content.
    input: model.vision ? ["text", "image"] : ["text"],
    contextWindow: model.contextWindow,
    maxTokens: model.maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    ...(model.reasoning ? { thinkingLevelMap } : {})
  };
}

/**
 * A runtime for one connection whose files live in `dir`. A custom connection gets a private
 * models.json and its key set in memory only; a subscription reads Pi's own auth file, which
 * Pi refreshes in place during requests.
 */
export async function createModelRuntime(
  pi: PiModule,
  provider: WorkerProvider,
  dir: string,
  credential: { apiKey?: string; authPath?: string }
): Promise<ModelRuntime> {
  await mkdir(dir, { recursive: true });
  const subscription = provider.kind === "subscription";
  const modelsPath = join(dir, "models.json");
  if (!subscription) {
    const modelsConfig = {
      providers: {
        [provider.id]: {
          name: provider.name,
          baseUrl: provider.baseUrl,
          api: provider.api,
          models: provider.models.map(modelDefinition)
        }
      }
    };
    await writeFile(modelsPath, `${JSON.stringify(modelsConfig, null, 2)}\n`, { mode: 0o600 });
  } else if (!credential.authPath) {
    throw new Error(`${provider.name} is not signed in`);
  }
  const runtime = await pi.ModelRuntime.create({
    authPath: subscription ? credential.authPath : join(dir, "auth.json"),
    modelsPath: subscription ? null : modelsPath,
    modelsStorePath: join(dir, "models-store.json"),
    allowModelNetwork: false,
    refreshOnCreate: false
  });
  if (!subscription) {
    if (!credential.apiKey) throw new Error("This connection has no API key");
    await runtime.setRuntimeApiKey(provider.id, credential.apiKey);
  }
  return runtime;
}

/** Subscriptions list what the signed-in account may use; custom connections list their models. */
export async function findModel(runtime: ModelRuntime, provider: WorkerProvider, modelId: string): Promise<PiModel | undefined> {
  return provider.kind === "subscription"
    ? (await runtime.getAvailable(provider.id)).find((model) => model.id === modelId)
    : runtime.getModel(provider.id, modelId);
}

/** Every session in the worker, the chat's and each sub-agent's, runs with these settings. */
export function workerSettings(pi: PiModule): SettingsManager {
  return pi.SettingsManager.inMemory({
    enableInstallTelemetry: false,
    enableAnalytics: false,
    cacheWarming: "off",
    defaultProjectTrust: "never",
    compaction: { enabled: true },
    retry: { enabled: true }
  }, { projectTrusted: false });
}
