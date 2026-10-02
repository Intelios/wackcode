import { useCallback, useState } from "react";
import { defaultModelChoice, type ModelChoice } from "../model-utils";
import type { ProviderRecord } from "../types";

const LAST_MODEL_KEY = "wackcode:lastModelChoice";
const LEGACY_MODELS_KEY = "wackcode:lastModel";

function loadJSON<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) as T : fallback;
  } catch {
    return fallback;
  }
}

/**
 * New chats follow the latest explicit pick or sent chat, across projects and launches.
 * Until the first new pick, preserve the old per-project preference for existing installs.
 * Merely reading an old chat doesn't replace the preference.
 */
export function useModelMemory(providers: ProviderRecord[]) {
  const [lastModel, setLastModel] = useState<ModelChoice | undefined>(() => loadJSON(LAST_MODEL_KEY, undefined));
  const [legacyModels] = useState<Record<string, ModelChoice> | null>(() => loadJSON(LEGACY_MODELS_KEY, {}));

  const rememberModel = useCallback((choice: ModelChoice) => {
    const next = { providerId: choice.providerId, modelId: choice.modelId, thinkingLevel: choice.thinkingLevel };
    setLastModel(next);
    // A storage failure must not turn a successful model change or send into an error.
    try { localStorage.setItem(LAST_MODEL_KEY, JSON.stringify(next)); } catch { /* Keep the in-memory choice. */ }
  }, []);

  const defaultChoice = useCallback((projectId: string | null) =>
    defaultModelChoice(providers, lastModel ?? legacyModels?.[projectId ?? "none"]),
  [providers, lastModel, legacyModels]);

  return { rememberModel, defaultChoice };
}
