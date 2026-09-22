import { describe, expect, it } from "vitest";
import { applyBuiltinModelSuggestion, mergeDiscoveredModels, searchBuiltinModels } from "./model-utils";
import type { BuiltinModelSuggestion } from "./types";

const flash: BuiltinModelSuggestion = {
  sourceProvider: "deepseek", sourceApi: "openai-completions", id: "deepseek-flash", name: "DeepSeek V4.1 Flash",
  contextWindow: 1_000_000, maxTokens: 384_000, reasoning: true,
  thinkingLevels: ["off", "low", "high", "max"],
  thinkingLevelMap: { off: null, low: "low", high: "high", max: "max" }, vision: true
};

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
