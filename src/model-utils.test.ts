import { describe, expect, it } from "vitest";
import { applyBuiltinModelSuggestion, autoTitleModelIssue, chatModelGone, defaultModelChoice, formatTokens, mergeDiscoveredModels, modelDisplayName, pickThinkingLevel, searchBuiltinModels, subagentModelIssue } from "./model-utils";
import type { BuiltinModelSuggestion, ModelRecord, ProviderRecord } from "./types";

const flash: BuiltinModelSuggestion = {
  sourceProvider: "deepseek", sourceApi: "openai-completions", id: "deepseek-flash", name: "DeepSeek V4.1 Flash",
  contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true,
  thinkingLevels: ["off", "low", "high", "max"],
  thinkingLevelMap: { off: null, low: "low", high: "high", max: "max" }, vision: true
};

describe("defaultModelChoice", () => {
  const provider: ProviderRecord = {
    id: "p", name: "Provider", kind: "custom", baseUrl: "", apiFormat: "openai-completions",
    createdAt: "", updatedAt: "", hasApiKey: true, connected: true,
    models: [{ id: "m", name: "Model", contextWindow: 10, maxTokens: 5, reasoning: true,
      thinkingLevels: ["low", "medium", "high"], thinkingLevelMap: {}, vision: false }]
  };
  const remembered = { providerId: "p", modelId: "m", thinkingLevel: "high" as const };
  const fallback = { ...provider, id: "fallback" };

  it("prefers the remembered model over provider ordering", () => {
    expect(defaultModelChoice([fallback, provider], remembered)).toEqual(remembered);
  });

  it("falls back when the model or provider is removed, incomplete, disabled or signed out", () => {
    for (const unavailable of [
      [], [{ ...provider, models: [] }],
      [{ ...provider, models: [{ ...provider.models[0], contextWindow: null }] }],
      [{ ...provider, enabled: false }], [{ ...provider, connected: false }]
    ]) {
      expect(defaultModelChoice([...unavailable, fallback], remembered)?.providerId).toBe("fallback");
    }
    expect(defaultModelChoice([provider], { ...remembered, modelId: "removed" })).toEqual({ ...remembered, thinkingLevel: "medium" });
  });

  it("adjusts unsupported reasoning and handles models with no reasoning levels", () => {
    expect(defaultModelChoice([provider], { ...remembered, thinkingLevel: "max" })?.thinkingLevel).toBe("medium");
    expect(defaultModelChoice([{ ...provider, models: [{ ...provider.models[0], thinkingLevels: [] }] }], remembered)?.thinkingLevel).toBe("off");
  });

  it("returns no choice when no model can run", () => {
    expect(defaultModelChoice([], remembered)).toBeUndefined();
    expect(defaultModelChoice([{ ...provider, connected: false }])).toBeUndefined();
  });
});

describe("mergeDiscoveredModels", () => {
  it("keeps configured models and adds unknown IDs without inventing limits", () => {
    const result = mergeDiscoveredModels([
      { id: "same", name: "Configured", contextWindow: 10, maxTokens: 5, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: true }
    ], ["same", "new"]);
    expect(result).toHaveLength(2);
    expect(result[0].name).toBe("Configured");
    expect(result[1].contextWindow).toBeNull();
    expect(result[1].maxTokens).toBeNull();
    // Vision is a capability the user confirms, like limits — discovery never assumes it.
    expect(result[0].vision).toBe(true);
    expect(result[1].vision).toBe(false);
  });
});

describe("Pi catalogue suggestions", () => {
  it("finds a model by partial, punctuation-insensitive name or ID", () => {
    expect(searchBuiltinModels([flash], "Deepseek V4 Flash")).toEqual([flash]);
    expect(searchBuiltinModels([flash], "deepseek_flash")).toEqual([flash]);
    expect(searchBuiltinModels([flash], "unknown")).toEqual([]);
  });

  it("keeps the direct provider visible when gateways have many closer name matches", () => {
    const gateways = Array.from({ length: 12 }, (_, index) => ({
      ...flash, sourceProvider: `gateway-${index}`, id: `gateway-${index}/deepseek-v4-flash`, name: "DeepSeek V4 Flash"
    }));
    expect(searchBuiltinModels([...gateways, flash], "Deepseek V4 Flash")).toContain(flash);
  });

  it("fills settings only when selected and preserves a provider's nonblank ID", () => {
    const model = {
      id: "gateway/deepseek-flash", name: "Custom", contextWindow: null, maxTokens: null,
      reasoning: false, thinkingLevels: ["off" as const], thinkingLevelMap: { off: null }, vision: false
    };
    expect(applyBuiltinModelSuggestion(model, flash)).toMatchObject({
      id: "gateway/deepseek-flash", name: flash.name, contextWindow: flash.contextWindow,
      maxTokens: flash.maxTokens, thinkingLevels: flash.thinkingLevels, vision: true
    });
    expect(applyBuiltinModelSuggestion({ ...model, id: "" }, flash).id).toBe("deepseek-flash");
    expect(model.contextWindow).toBeNull();
  });
});

describe("pickThinkingLevel", () => {
  const model = (thinkingLevels: ModelRecord["thinkingLevels"]): ModelRecord => ({
    id: "m", name: "M", contextWindow: 1, maxTokens: 1, reasoning: true, thinkingLevels, thinkingLevelMap: {}, vision: false
  });

  it("keeps the requested level, then the current one, then medium, then the first", () => {
    expect(pickThinkingLevel(model(["off", "low", "high"]), "high", "low")).toBe("high");
    expect(pickThinkingLevel(model(["off", "low", "high"]), "max", "low")).toBe("low");
    expect(pickThinkingLevel(model(["off", "medium"]), undefined, "max")).toBe("medium");
    expect(pickThinkingLevel(model(["low", "high"]))).toBe("low");
    expect(pickThinkingLevel(undefined, "high")).toBe("off");
  });
});

describe("modelDisplayName", () => {
  const provider: ProviderRecord = {
    id: "p", name: "Provider", kind: "custom", baseUrl: "", apiFormat: "openai-completions", createdAt: "", updatedAt: "", hasApiKey: true, connected: true,
    models: [{ id: "gpt-5", name: "GPT 5", contextWindow: 10, maxTokens: 5, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {}, vision: false }]
  };

  it("uses configured names and falls back to the durable model id", () => {
    expect(modelDisplayName([provider], { providerId: "p", modelId: "gpt-5" })).toBe("GPT 5");
    expect(modelDisplayName([provider], { providerId: "gone", modelId: "claude-sonnet-4" })).toBe("claude-sonnet-4");
  });
});

describe("subagentModelIssue", () => {
  const provider: ProviderRecord = {
    id: "p", name: "Cheap", kind: "subscription", baseUrl: "", apiFormat: "", createdAt: "", updatedAt: "", hasApiKey: false, connected: true,
    models: [{ id: "m", name: "Mini", contextWindow: 10, maxTokens: 5, reasoning: true, thinkingLevels: ["off", "low"], thinkingLevelMap: {}, vision: false }]
  };
  const choice = { providerId: "p", modelId: "m", thinkingLevel: "low" as const };

  it("explains why an agent's own model can't run, and is silent for the chat's model", () => {
    expect(subagentModelIssue(null, [provider])).toBeUndefined();
    expect(subagentModelIssue(choice, [provider])).toBeUndefined();
    expect(subagentModelIssue(choice, [])).toBe("Its connection no longer exists.");
    expect(subagentModelIssue(choice, [{ ...provider, enabled: false }])).toBe("Its connection is turned off.");
    expect(subagentModelIssue(choice, [{ ...provider, connected: false }])).toBe("Cheap is signed out.");
    expect(subagentModelIssue({ ...choice, modelId: "gone" }, [provider])).toBe("Its model is no longer configured.");
    expect(subagentModelIssue({ ...choice, thinkingLevel: "high" }, [provider])).toBe("Mini doesn't support high reasoning.");
  });
});

describe("autoTitleModelIssue", () => {
  const provider: ProviderRecord = {
    id: "p", name: "Cheap", kind: "custom", baseUrl: "", apiFormat: "openai-completions", createdAt: "", updatedAt: "", hasApiKey: true, connected: true,
    models: [{ id: "m", name: "Mini", contextWindow: 10, maxTokens: 5, reasoning: true, thinkingLevels: ["off", "low"], thinkingLevelMap: {}, vision: false }]
  };
  const config = { enabled: true, providerId: "p", modelId: "m" };

  it("explains why the titles model can't run, and stays quiet while unset", () => {
    expect(autoTitleModelIssue({ ...config, providerId: null, modelId: null }, [provider])).toBeUndefined();
    expect(autoTitleModelIssue(config, [provider])).toBeUndefined();
    expect(autoTitleModelIssue(config, [])).toBe("Its connection no longer exists.");
    expect(autoTitleModelIssue(config, [{ ...provider, enabled: false }])).toBe("Cheap is turned off.");
    expect(autoTitleModelIssue(config, [{ ...provider, connected: false }])).toBe("Cheap has no API key.");
  });
});

describe("chatModelGone", () => {
  const provider: ProviderRecord = {
    id: "p", name: "Custom", kind: "custom", baseUrl: "", apiFormat: "openai-completions", createdAt: "", updatedAt: "", hasApiKey: true, connected: true,
    models: [{ id: "m", name: "Mini", contextWindow: 10, maxTokens: 5, reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: {}, vision: false }]
  };
  const task = { providerId: "p", modelId: "m" };

  it("spots a model removed from its connection, and reports the worker's view of a subscription drop", () => {
    expect(chatModelGone([provider], task)).toBe(false);
    expect(chatModelGone([provider], { ...task, modelId: "removed" })).toBe(true);
    // A subscription can stop serving a model while it stays configured: only the snapshot knows.
    expect(chatModelGone([provider], task, { modelMissing: true })).toBe(true);
  });

  it("stays quiet with no chat open, and leaves a missing connection to its own error", () => {
    expect(chatModelGone([provider], undefined, { modelMissing: true })).toBe(false);
    expect(chatModelGone([], { ...task, providerId: "gone" })).toBe(false);
  });
});

describe("formatTokens", () => {
  it("shortens token counts the way model rows show them", () => {
    expect(formatTokens(512)).toBe("512");
    expect(formatTokens(8_000)).toBe("8K");
    expect(formatTokens(131_072)).toBe("131K");
    expect(formatTokens(1_000_000)).toBe("1M");
    expect(formatTokens(1_500_000)).toBe("1.5M");
  });
});
