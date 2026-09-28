import type { AutoTitleConfig, BuiltinModelSuggestion, ModelRecord, ModelRef, ProviderRecord, SubagentModel, ThinkingLevel } from "./types";

function words(value: string): string[] {
  return value.toLowerCase().normalize("NFKD").match(/[\p{L}\p{N}]+/gu) ?? [];
}

function matchScore(model: BuiltinModelSuggestion, query: string[]): number | null {
  const id = words(model.id);
  const name = words(model.name);
  const needle = query.join(" ");
  let score: number | null = null;
  if (id.join(" ") === needle) score = 0;
  else if (name.join(" ") === needle) score = 1;
  else if (name.join(" ").startsWith(needle)) score = 2;
  else if (id.join(" ").startsWith(needle)) score = 3;
  const all = [...name, ...id];
  if (score === null && query.every((part) => all.some((word) => word.startsWith(part)))) score = 4;
  if (score === null && query.every((part) => all.some((word) => word.includes(part)))) score = 5;
  if (score === null) return null;
  // Put the direct provider within reach when gateways repeat the same model name.
  const directProvider = words(model.sourceProvider).some((part) => part.length >= 4 && query.includes(part));
  return score - (directProvider ? 3.5 : 0);
}

export function searchBuiltinModels(catalog: BuiltinModelSuggestion[], text: string): BuiltinModelSuggestion[] {
  const query = words(text);
  if (!query.length) return [];
  return catalog
    .map((model) => ({ model, score: matchScore(model, query) }))
    .filter((match): match is { model: BuiltinModelSuggestion; score: number } => match.score !== null)
    .sort((left, right) => left.score - right.score || left.model.name.localeCompare(right.model.name)
      || left.model.sourceProvider.localeCompare(right.model.sourceProvider) || left.model.id.localeCompare(right.model.id))
    .slice(0, 8)
    .map(({ model }) => model);
}

export function applyBuiltinModelSuggestion(model: ModelRecord, suggestion: BuiltinModelSuggestion): ModelRecord {
  return {
    id: model.id.trim() ? model.id : suggestion.id,
    name: suggestion.name,
    contextWindow: suggestion.contextWindow,
    maxTokens: suggestion.maxTokens,
    reasoning: suggestion.reasoning,
    thinkingLevels: [...suggestion.thinkingLevels],
    thinkingLevelMap: { ...suggestion.thinkingLevelMap },
    vision: suggestion.vision
  };
}

export function mergeDiscoveredModels(existing: ModelRecord[], modelIds: string[]): ModelRecord[] {
  const known = new Map(existing.map((model) => [model.id, model]));
  for (const id of modelIds) {
    if (!known.has(id)) {
      known.set(id, {
        id,
        name: id,
        contextWindow: null,
        maxTokens: null,
        reasoning: false,
        thinkingLevels: ["off"],
        thinkingLevelMap: { off: null },
        vision: false
      });
    }
  }
  return [...known.values()];
}

export function modelIsReady(model: ModelRecord): boolean {
  return Boolean(model.id.trim() && model.contextWindow && model.maxTokens);
}

/** Current friendly name for a persisted model reference; old/deleted models fall back to ID. */
export function modelDisplayName(providers: ProviderRecord[], reference: ModelRef): string {
  return providers.find((provider) => provider.id === reference.providerId)
    ?.models.find((model) => model.id === reference.modelId)?.name || reference.modelId;
}

/**
 * The reasoning level to use after a model change: the requested one if the model has it, else
 * the current one, else "medium", else the model's first level.
 */
export function pickThinkingLevel(model: ModelRecord | undefined, requested?: ThinkingLevel, current?: ThinkingLevel): ThinkingLevel {
  const levels: ThinkingLevel[] = model?.thinkingLevels.length ? model.thinkingLevels : ["off"];
  if (requested && levels.includes(requested)) return requested;
  if (current && levels.includes(current)) return current;
  return levels.includes("medium") ? "medium" : levels[0];
}

/** Why a sub-agent's own model can't run right now, or undefined when it can (or it has none). */
export function subagentModelIssue(choice: SubagentModel | null | undefined, providers: ProviderRecord[]): string | undefined {
  if (!choice) return undefined;
  const provider = providers.find((item) => item.id === choice.providerId);
  if (!provider) return "Its connection no longer exists.";
  if (provider.enabled === false) return "Its connection is turned off.";
  if (!provider.connected) return provider.kind === "subscription" ? `${provider.name} is signed out.` : `${provider.name} has no API key.`;
  const model = provider.models.find((item) => item.id === choice.modelId);
  if (!model || !modelIsReady(model)) return "Its model is no longer configured.";
  if (!model.thinkingLevels.includes(choice.thinkingLevel)) return `${model.name} doesn't support ${choice.thinkingLevel} reasoning.`;
  return undefined;
}

/**
 * Why the auto-titles model can't run right now, or undefined when it can — including while
 * nothing is chosen yet, which is just unset rather than broken.
 */
export function autoTitleModelIssue(config: AutoTitleConfig, providers: ProviderRecord[]): string | undefined {
  if (!config.providerId || !config.modelId) return undefined;
  const provider = providers.find((item) => item.id === config.providerId);
  if (!provider) return "Its connection no longer exists.";
  if (provider.enabled === false) return `${provider.name} is turned off.`;
  if (!provider.connected) return provider.kind === "subscription" ? `${provider.name} is signed out.` : `${provider.name} has no API key.`;
  const model = provider.models.find((item) => item.id === config.modelId);
  if (!model || !modelIsReady(model)) return "Its model is no longer configured.";
  return undefined;
}
