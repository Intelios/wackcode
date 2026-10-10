import { getSupportedThinkingLevels } from "@earendil-works/pi-ai";
import { getBuiltinModels, getBuiltinProviders } from "@earendil-works/pi-ai/providers/all";

export interface BuiltinModelSuggestion {
  sourceProvider: string;
  sourceApi: string;
  id: string;
  name: string;
  contextWindow: number;
  maxTokens: number;
  reasoning: boolean;
  thinkingLevels: string[];
  thinkingLevelMap: Record<string, string | null>;
  vision: boolean;
}

/** Project only the settings WackCode can represent from Pi's bundled, static catalogue. */
export function listBuiltinModelSuggestions(): BuiltinModelSuggestion[] {
  return getBuiltinProviders().flatMap((provider) => getBuiltinModels(provider).map((model) => {
    const thinkingLevels = getSupportedThinkingLevels(model);
    const thinkingLevelMap = Object.fromEntries(thinkingLevels.map((level) => [
      level,
      model.thinkingLevelMap?.[level] ?? (level === "off" ? null : level)
    ]));
    return {
      sourceProvider: model.provider,
      sourceApi: model.api,
      id: model.id,
      name: model.name,
      contextWindow: model.contextWindow,
      maxTokens: model.maxTokens,
      reasoning: model.reasoning,
      thinkingLevels,
      thinkingLevelMap,
      vision: model.input.includes("image")
    };
  }));
}

/**
 * `compat` lookup for custom connections. A custom connection's models.json re-declares its
 * models from scratch, and Pi's request shaping reads `compat` off the model itself
 * (`forceAdaptiveThinking`, `supportsStore`, `maxTokensField`, `supportsTemperature`…), so an
 * entry that declares none silently falls back to defaults — e.g. budget-based
 * `thinking.type:"enabled"`, which adaptive-only Claude models answer with a 400. When the
 * configured ID (or a spelling variant: a vendor prefix, a dotted version or a date/version
 * suffix) names a model in the bundled catalogue, its `compat` is copied into models.json.
 */
type CatalogEntry = { api: string; compat: Record<string, unknown>; baseUrl?: string };

/** Compat fields that are an account's request payload rather than a capability, never copied. */
const PAYLOAD_COMPAT_KEYS = new Set(["allowedFallbackModels", "openRouterRouting"]);

let compatIndex: Map<string, CatalogEntry[]> | undefined;

function normalizedModelId(id: string): string {
  return id.trim().toLowerCase().replace(/^~+/, "").replaceAll(".", "-");
}

/**
 * ID spellings that may refer to one model, most specific first: the full ID (which may carry a
 * vendor prefix like `anthropic/claude-opus-4.6`), then the same with a `-YYYYMMDD` snapshot or
 * `-v1`-style revision dropped, then each again as just the segment after the last slash (a
 * Vercel/OpenRouter-style `vendor/model` ID still matches the vendor's own catalogue entry).
 */
function compatIdCandidates(id: string): string[] {
  const normalized = normalizedModelId(id);
  const bases: string[] = [];
  for (const base of [normalized, normalized.split("/").pop() ?? normalized]) {
    for (const candidate of [base, base.replace(/-\d{8}$/, ""), base.replace(/-v\d+(?::\d+)?$/, "")]) {
      if (candidate && !bases.includes(candidate)) bases.push(candidate);
    }
  }
  return bases;
}

function catalogCompatIndex(): Map<string, CatalogEntry[]> {
  if (compatIndex) return compatIndex;
  compatIndex = new Map();
  for (const provider of getBuiltinProviders()) {
    for (const model of getBuiltinModels(provider)) {
      const compat = model.compat && typeof model.compat === "object"
        ? ({ ...(model.compat as object) } as Record<string, unknown>)
        : {};
      const entry: CatalogEntry = { api: model.api, compat, baseUrl: model.baseUrl };
      for (const key of compatIdCandidates(model.id)) {
        const list = compatIndex.get(key);
        if (list) list.push(entry);
        else compatIndex.set(key, [entry]);
      }
    }
  }
  return compatIndex;
}

/**
 * The catalogue `compat` for a configured model, or undefined when no entry matches — the model
 * then behaves exactly as before (Pi's defaults). `api` is the API the model will actually speak
 * (its own override, else the connection's), so e.g. a Completions entry never colours a Messages
 * model of the same ID. Between candidate entries for one ID a matching base-URL host wins, then
 * the leanest compat: a gateway that serves another vendor's model but supports only part of its
 * surface should send the smallest request shape rather than the vendor's quirks.
 */
export function builtinCompat(api: string, modelId: string, baseUrl?: string): Record<string, unknown> | undefined {
  const index = catalogCompatIndex();
  const host = hostOf(baseUrl);
  // Every catalogue entry the configured ID could name, tagged by how specific that spelling is.
  const candidates: { rank: number; entry: CatalogEntry }[] = [];
  compatIdCandidates(modelId).forEach((key, rank) => {
    for (const entry of index.get(key) ?? []) {
      if (entry.api === api) candidates.push({ rank, entry });
    }
  });
  if (!candidates.length) return adaptiveFallbackCompat(api, modelId);
  // A catalogue entry on the connection's own host is authoritative — including one that
  // deliberately lists the model with no compat — so it settles the question on its own.
  // Otherwise only compat-bearing entries shape the request: the most specific ID spelling wins,
  // then the leanest compat, since an unknown host should get the smallest request shape rather
  // than another vendor's quirks.
  const sameHost = candidates.filter(({ entry }) => host !== undefined && hostOf(entry.baseUrl) === host);
  const pool = sameHost.length
    ? sameHost
    : candidates.filter(({ entry }) => Object.keys(entry.compat).length > 0);
  const pick = pool.reduce<{ rank: number; entry: CatalogEntry } | undefined>((best, candidate) =>
    best === undefined
      || candidate.rank < best.rank
      || (candidate.rank === best.rank
        && Object.keys(candidate.entry.compat).length < Object.keys(best.entry.compat).length)
      ? candidate
      : best, undefined);
  if (!pick) return undefined;
  const compat = { ...pick.entry.compat };
  // Adaptive thinking describes the model, not the provider that listed it leanest: if the same
  // model ID requires it anywhere in the pool (e.g. a Fireworks-hosted model whose lean Vercel
  // mirror omits the flag), budget thinking would 400 on this connection too.
  if (pool.some(({ entry }) => entry.compat.forceAdaptiveThinking === true)) {
    compat.forceAdaptiveThinking = true;
  }
  for (const field of PAYLOAD_COMPAT_KEYS) delete compat[field];
  return Object.keys(compat).length ? compat : undefined;
}

/**
 * A Claude newer than anything the bundled catalogue knows still needs adaptive thinking: every
 * Claude ≥ 4.6 that Anthropic lists requires it, so an unlisted `claude-{family}-5-*` (or a later
 * 4.x minor) almost certainly does too — and budget thinking 400s. Only fires when no catalogue
 * entry at all matched (a listed model keeps its own verdict), and a wrong guess only affects an
 * ID that doesn't exist upstream anyway. Old-style `claude-3-7-sonnet` naming predates the
 * switch and is left alone.
 */
function adaptiveFallbackCompat(api: string, modelId: string): Record<string, unknown> | undefined {
  if (api !== "anthropic-messages") return undefined;
  const normalized = normalizedModelId(modelId);
  const bare = normalized.split("/").pop() ?? normalized;
  const match = bare.match(/^claude-(?:opus|sonnet|haiku|fable)-(\d+)(?:-(\d+))?/);
  if (!match) return undefined;
  const major = Number(match[1]);
  const minor = match[2] === undefined ? undefined : Number(match[2]);
  if (major > 4 || (major === 4 && minor !== undefined && minor >= 6)) {
    return { forceAdaptiveThinking: true };
  }
  return undefined;
}

function hostOf(baseUrl: string | undefined): string | undefined {
  if (!baseUrl) return undefined;
  try {
    return new URL(baseUrl).hostname;
  } catch {
    return undefined;
  }
}
