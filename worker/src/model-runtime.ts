/**
 * Pi model runtimes for one connection. The chat's own session builds one at `init`; sub-agents
 * whose model lives on another connection build their own with the same code, so both paths
 * write models.json and hand over credentials identically.
 */
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { builtinCompat } from "./catalog-models.js";
import { instrumentUsage } from "./usage.js";
import { instrumentOpenCodeHeaders } from "./opencode-headers.js";
import { INLINE_IMAGE_LIMITS } from "./image-limits.js";
import { THINKING_LEVELS, type WorkerModel, type WorkerProvider } from "./protocol.js";

type PiModule = typeof import("@earendil-works/pi-coding-agent");
export type ModelRuntime = Awaited<ReturnType<PiModule["ModelRuntime"]["create"]>>;
export type PiModel = NonNullable<ReturnType<ModelRuntime["getModel"]>>;
export type SettingsManager = ReturnType<PiModule["SettingsManager"]["inMemory"]>;

/**
 * The base URL a model's API actually needs. The Messages client appends `/v1/messages` itself
 * and the OpenAI clients append `/chat/completions` or `/responses`. Google adds
 * `/models/<id>:streamGenerateContent`. A Messages base leaves `/v1` off while the other
 * formats keep whatever version segment they were given
 * — `…/v1` usually, but endpoints like z.ai's `…/paas/v4` carry their own and must not gain a
 * second one. A model on its connection's own API uses the saved base URL as-is (Messages also
 * repairs records saved before canonicalization); only a model whose `api` overrides the
 * connection's needs a re-derived base: a Messages model on a Completions connection would
 * otherwise post `…/v1/v1/messages` and land on the gateway's HTML 404 page, and the reverse
 * gains `/v1` (OpenCode Go's convention). Returns undefined when the connection's base already
 * fits, leaving the provider's own `baseUrl` in force. Same-gateway base URLs are the
 * documented convention (see ProvidersSection), except OpenCode Go's native Google API:
 * its legacy /zen/go prefix must become /inference/go. A model hosted elsewhere can't be
 * expressed today.
 */
function resolveModelBaseUrl(provider: WorkerProvider, model: WorkerModel): string | undefined {
  const saved = provider.baseUrl.replace(/\/+$/, "");
  // Saved connections are already canonicalized by Rust. Repair a pre-canonicalized Google
  // URL too, but only its versioned endpoint path — never a query or an unrelated /models path.
  let base = saved;
  if (provider.api === "google-generative-ai") {
    try {
      const url = new URL(saved);
      const path = url.pathname.replace(/\/+$/, "");
      const match = path.match(/^(.*\/(?:v1|v1beta|v1alpha))\/models(?:\/[^/:]+(?::(?:generateContent|streamGenerateContent))?)?$/);
      if (match) {
        url.pathname = match[1];
        if (path.endsWith(":streamGenerateContent") && url.searchParams.getAll("alt").includes("sse")) {
          const query = [...url.searchParams].filter(([key, value]) => key !== "alt" || value !== "sse");
          url.search = new URLSearchParams(query).toString();
        }
        base = url.toString();
      }
    } catch { /* Pi reports invalid URLs. */ }
  }
  const api = model.api ?? provider.api;
  if (api === "google-generative-ai") {
    const url = new URL(base);
    // @google/genai concatenates baseUrl with its request path, so a query would swallow
    // /models/… into the query (and a fragment would hide it). Match save-time validation.
    if (url.search) throw new Error("Google base URLs cannot include query parameters. Remove the query and enter any API key in the API key field.");
    // OpenCode's legacy Go prefix still serves the other APIs, but its Google route is an
    // HTML 404. Native Gemini lives at the documented /inference/go/v1 prefix instead.
    // Only change this known path on this exact host; never reroute another provider or API.
    if (url.origin === "https://opencode.ai" && /^\/zen\/go(?:\/v1)?$/.test(url.pathname)) {
      url.pathname = "/inference/go/v1";
      base = url.toString();
    }
    if (url.hash) {
      url.hash = "";
      base = url.toString();
    }
  }
  const resolved = api === "anthropic-messages"
    ? base.replace(/\/v1\/messages$/i, "").replace(/\/v1$/i, "")
    : api !== provider.api && !/\/v\d[^/]*$/i.test(base) ? `${base}/v1` : base;
  return resolved === saved ? undefined : resolved;
}

export function modelDefinition(provider: WorkerProvider, model: WorkerModel): Record<string, unknown> {
  const baseUrl = resolveModelBaseUrl(provider, model);
  // models.json re-declares the model, so Pi's request shaping (`forceAdaptiveThinking`,
  // `maxTokensField`, `supportsStore`…) is lost unless the bundled catalogue's compat is copied
  // over: without it an adaptive-only Claude is asked for `thinking.type:"enabled"` and answers
  // 400, and a completions gateway is sent fields it rejects. Unknown IDs get Pi's defaults.
  const compat = builtinCompat(model.api ?? provider.api, model.id, baseUrl ?? provider.baseUrl);
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
    // Pi resizes prompt and tool-result images (`read`, screenshots, MCP) to these, so declaring
    // them here keeps every image small at the source instead of Pi's roomy defaults. Without
    // `inputLimits` a single screenshot can be megabytes, and a chat with a dozen of them grows
    // its request until a gateway's body cap answers 413 (see `image-limits.ts`).
    inputLimits: { images: { resize: INLINE_IMAGE_LIMITS } },
    contextWindow: model.contextWindow,
    maxTokens: model.maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    ...(model.reasoning ? { thinkingLevelMap } : {}),
    // Pi takes a model's own `api` over the provider's, so one gateway can serve each model
    // over the API it actually speaks — with the base URL that API needs.
    ...(model.api ? { api: model.api } : {}),
    ...(baseUrl ? { baseUrl } : {}),
    ...(compat ? { compat } : {})
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
          models: provider.models.map((model) => modelDefinition(provider, model))
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
  instrumentOpenCodeHeaders(runtime);
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
