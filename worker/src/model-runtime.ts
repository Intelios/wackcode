/**
 * Pi model runtimes for one connection. The chat's own session builds one at `init`; sub-agents
 * whose model lives on another connection build their own with the same code, so both paths
 * write models.json and hand over credentials identically.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { instrumentUsage } from "./usage.js";
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
    // Settings lets a name stay blank (the ID shows instead), but Pi rejects a blank name and
    // with it the whole models.json, so every model on the connection would vanish.
    name: model.name.trim() || model.id,
    reasoning: model.reasoning,
    // Pi's own capability flag. Without "image", Pi replaces images with an "image omitted"
    // placeholder before the request is built, and `read` stops returning image content.
    input: model.vision ? ["text", "image"] : ["text"],
    contextWindow: model.contextWindow,
    maxTokens: model.maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    ...(model.reasoning ? { thinkingLevelMap } : {}),
    // Pi takes a model's own `api` over the provider's, so one gateway can serve each model
    // over the API it actually speaks.
    ...(model.api ? { api: model.api } : {})
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
  instrumentUsage(runtime);
  return runtime;
}

/** Subscriptions list what the signed-in account may use; custom connections list their models. */
export async function findModel(runtime: ModelRuntime, provider: WorkerProvider, modelId: string): Promise<PiModel | undefined> {
  return provider.kind === "subscription"
    ? (await runtime.getAvailable(provider.id)).find((model) => model.id === modelId)
    : runtime.getModel(provider.id, modelId);
}

/** Shown wherever the user tries to make a model-less chat do model work. */
export const MODEL_MISSING_MESSAGE = "This chat's model is no longer configured. Pick another to continue.";

/**
 * Why `findModel` came back empty. Pi drops a connection whose configuration it rejects, which
 * makes every model on it look deleted; that case names the connection and Pi's reason instead
 * of blaming the model. The reason loses its file path and the connection's internal ID.
 */
export function missingModelMessage(runtime: ModelRuntime, provider: WorkerProvider): string {
  const error = runtime.getError();
  if (!error) return MODEL_MISSING_MESSAGE;
  const reason = error.split("\n")
    .map((line) => line.trim().replace(/^- /, ""))
    .filter((line) => line && !line.startsWith("File:"))
    .join(" ")
    .replaceAll(`providers.${provider.id}.`, "")
    .replaceAll(provider.id, provider.name)
    .slice(0, 300);
  return `${provider.name} couldn't be loaded (${reason}). Fix it in Settings › Providers, or pick another model.`;
}

/**
 * The stand-in for a model that left its connection, so a chat can still be opened and read:
 * the session restores on it, the snapshot says `modelMissing`, and every run is refused. A
 * zero context window keeps the context meter honestly absent instead of inventing a limit.
 */
export function missingModelPlaceholder(provider: WorkerProvider, modelId: string): PiModel {
  return {
    id: modelId,
    name: modelId,
    api: provider.api,
    provider: provider.id,
    baseUrl: provider.baseUrl,
    reasoning: false,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: 0,
    maxTokens: 0
  };
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
