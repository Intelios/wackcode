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

/** Chat mode remembers its own model: what suits a coding agent is often not what suits a chat. */
export const LAST_CHAT_MODEL_KEY = "wackcode:lastChatModelChoice";

/**
 * New chats follow the latest explicit pick or sent chat, across projects and launches.
 * Until the first new pick, preserve the old per-project preference for existing installs.
 * Merely reading an old chat doesn't replace the preference.
 *
 * One instance per area, each with its own `storageKey`. Only the Code area's has the old
 * per-project preference to fall back on.
 */
export function useModelMemory(providers: ProviderRecord[], storageKey = LAST_MODEL_KEY) {
  const [lastModel, setLastModel] = useState<ModelChoice | undefined>(() => loadJSON(storageKey, undefined));
  const [legacyModels] = useState<Record<string, ModelChoice> | null>(() => storageKey === LAST_MODEL_KEY ? loadJSON(LEGACY_MODELS_KEY, {}) : null);

  const rememberModel = useCallback((choice: ModelChoice) => {
    const next = { providerId: choice.providerId, modelId: choice.modelId, thinkingLevel: choice.thinkingLevel };
    setLastModel(next);
    // A storage failure must not turn a successful model change or send into an error.
    try { localStorage.setItem(storageKey, JSON.stringify(next)); } catch { /* Keep the in-memory choice. */ }
  }, [storageKey]);

  const defaultChoice = useCallback((projectId: string | null) =>
    defaultModelChoice(providers, lastModel ?? legacyModels?.[projectId ?? "none"]),
  [providers, lastModel, legacyModels]);

  /** Whether a choice was ever remembered here, as opposed to the fallback `defaultChoice` gives. */
  return { rememberModel, defaultChoice, hasChoice: lastModel !== undefined };
}
