import type { ModelRecord } from "./types";

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
        thinkingLevelMap: { off: null }
      });
    }
  }
  return [...known.values()];
}

export function modelIsReady(model: ModelRecord): boolean {
  return Boolean(model.id.trim() && model.contextWindow && model.maxTokens);
}
