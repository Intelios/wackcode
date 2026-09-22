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
