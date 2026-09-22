import type { BuiltinModelSuggestion, ModelRecord } from "./types";

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
