/** Reference metadata only. Applying a profile edits a draft; endpoint hints never leave here. */
import type { ApiFormat, ModelCatalogEntry, ModelCatalogProfile, ModelRecord } from "./types";

function words(value: string): string[] {
  return value.toLowerCase().normalize("NFKD").match(/[\p{L}\p{N}]+/gu) ?? [];
}

export function searchModelCatalog(catalog: ModelCatalogEntry[], text: string): ModelCatalogEntry[] {
  const query = words(text);
  if (!query.length) return [];
  const needle = query.join(" ");
  return catalog.map((entry) => {
    const ids = [entry.id, ...entry.profiles.map((profile) => profile.id)].map(words);
    const name = words(entry.name);
    const all = [...name, ...ids.flat()];
    const score = ids.some((id) => id.join(" ") === needle) ? 0
      : name.join(" ") === needle ? 1
      : name.join(" ").startsWith(needle) ? 2
      : ids.some((id) => id.join(" ").startsWith(needle)) ? 3
      : query.every((part) => all.some((word) => word.startsWith(part))) ? 4
      : query.every((part) => all.some((word) => word.includes(part))) ? 5 : null;
    return { entry, score };
  }).filter((match): match is { entry: ModelCatalogEntry; score: number } => match.score !== null)
    .sort((a, b) => a.score - b.score || a.entry.name.localeCompare(b.entry.name) || a.entry.id.localeCompare(b.entry.id))
    .slice(0, 8).map(({ entry }) => entry);
}

export function sortedCatalogProfiles(entry: ModelCatalogEntry): ModelCatalogProfile[] {
  return [...entry.profiles].sort((a, b) => Number(b.provider === entry.originalProvider) - Number(a.provider === entry.originalProvider)
    || a.providerName.localeCompare(b.providerName) || a.id.localeCompare(b.id));
}

function endpointKey(baseUrl: string, api: ApiFormat): string | undefined {
  try {
    const url = new URL(baseUrl);
    if (!["https:", "http:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) return;
    let path = url.pathname.replace(/\/+$/, "");
    if (api === "anthropic-messages") path = path.replace(/\/v1\/messages$/i, "").replace(/\/v1$/i, "");
    if (api === "google-generative-ai") path = path.replace(/(\/(?:v1|v1beta|v1alpha))\/models(?:\/[^/]+)?$/, "$1");
    return url.origin + path;
  } catch { return; }
}

/** Match the whole API base, never just its host: gateways can serve several plans on one host. */
export function matchingCatalogProfile(entry: ModelCatalogEntry, baseUrl: string, api: ApiFormat, modelId = ""): ModelCatalogProfile | undefined {
  const key = endpointKey(baseUrl, api);
  if (!key) return;
  const matches = entry.profiles.filter((profile) => profile.baseUrl && profile.apiFormats.includes(api)
    && endpointKey(profile.baseUrl, api) === key);
  const exact = matches.filter((profile) => profile.id === modelId.trim());
  return exact.length === 1 ? exact[0] : matches.length === 1 ? matches[0] : undefined;
}

export function applyCatalogProfile(model: ModelRecord, entry: ModelCatalogEntry, profile: ModelCatalogProfile): ModelRecord {
  return {
    ...model,
    id: model.id.trim() ? model.id : profile.id,
    name: entry.name,
    contextWindow: profile.contextWindow ?? model.contextWindow,
    maxTokens: profile.maxTokens ?? model.maxTokens,
    reasoning: profile.reasoning,
    // Missing controls preserve the user's values; the preview calls out manual setup.
    thinkingLevels: [...(profile.thinkingLevels ?? model.thinkingLevels)],
    thinkingLevelMap: { ...(profile.thinkingLevels ? profile.thinkingLevelMap : model.thinkingLevelMap) },
    vision: profile.vision
  };
}
