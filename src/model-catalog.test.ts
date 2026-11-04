import { describe, expect, it } from "vitest";
import { applyCatalogProfile, matchingCatalogProfile, searchModelCatalog, sortedCatalogProfiles } from "./model-catalog";
import type { ModelCatalogEntry, ModelCatalogProfile, ModelRecord } from "./types";

const native: ModelCatalogProfile = {
  key: "lab:model", provider: "lab", providerName: "Lab", id: "model", baseUrl: "https://example.test/v1",
  apiFormats: ["openai-completions"], contextWindow: 128_000, maxTokens: 16_000, reasoning: true,
  thinkingLevels: ["off", "low", "high"], thinkingLevelMap: { off: "none", low: "low", high: "high" }, vision: true
};
const gateway: ModelCatalogProfile = { ...native, key: "gateway:model", provider: "gateway", providerName: "Gateway", id: "vendor/model", baseUrl: "https://example.test/plan/v1", maxTokens: 8_000 };
const entry: ModelCatalogEntry = { id: "lab/model", name: "Model Four", originalProvider: "lab", profiles: [gateway, native] };

describe("model catalog lookup", () => {
  it("searches all profile IDs without repeating the underlying model", () => {
    for (const query of ["Model Four", "vendor_model", "lab/model"]) expect(searchModelCatalog([entry], query)).toEqual([entry]);
    expect(searchModelCatalog([entry], "")).toEqual([]);
    expect(searchModelCatalog([entry], "missing")).toEqual([]);
    expect(sortedCatalogProfiles(entry)).toEqual([native, gateway]);
  });

  it("matches full base paths, protocols and exact aliases rather than host alone", () => {
    expect(matchingCatalogProfile(entry, "https://example.test/plan/v1/", "openai-completions")).toEqual(gateway);
    expect(matchingCatalogProfile(entry, "https://example.test/v1", "openai-completions")).toEqual(native);
    expect(matchingCatalogProfile(entry, "https://example.test/other/v1", "openai-completions")).toBeUndefined();
    expect(matchingCatalogProfile(entry, "https://example.test/v1", "openai-responses")).toBeUndefined();
    const ambiguous = { ...entry, profiles: [native, { ...native, key: "second", id: "other-alias" }] };
    expect(matchingCatalogProfile(ambiguous, native.baseUrl!, "openai-completions")).toBeUndefined();
    expect(matchingCatalogProfile(ambiguous, native.baseUrl!, "openai-completions", "other-alias")?.key).toBe("second");
  });

  it("normalizes Messages and Google endpoint suffixes without matching queries or other hosts", () => {
    const messages = { ...native, baseUrl: "https://example.test/plan", apiFormats: ["anthropic-messages" as const] };
    const google = { ...native, baseUrl: "https://google.test/v1beta", apiFormats: ["google-generative-ai" as const] };
    const model = { ...entry, profiles: [messages, google] };
    expect(matchingCatalogProfile(model, "https://example.test/plan/v1/messages", "anthropic-messages")).toEqual(messages);
    expect(matchingCatalogProfile(model, "https://google.test/v1beta/models/gemini", "google-generative-ai")).toEqual(google);
    expect(matchingCatalogProfile(model, "https://google.test/v1beta?key=private", "google-generative-ai")).toBeUndefined();
    expect(matchingCatalogProfile(model, "https://google.test.attacker.test/v1beta", "google-generative-ai")).toBeUndefined();
  });

  it("matches Mistral's one profile URL from both its API surfaces", () => {
    // api.mistral.ai speaks the native chunk format and Chat Completions; its profile carries
    // the /v1 the Completions surface expects, and matching strips it for the native surface.
    const mistral = { ...native, baseUrl: "https://api.mistral.ai/v1", apiFormats: ["mistral-conversations" as const, "openai-completions" as const] };
    const model = { ...entry, profiles: [mistral] };
    expect(matchingCatalogProfile(model, "https://api.mistral.ai", "mistral-conversations")).toEqual(mistral);
    expect(matchingCatalogProfile(model, "https://api.mistral.ai/v1/chat/completions", "mistral-conversations")).toEqual(mistral);
    expect(matchingCatalogProfile(model, "https://api.mistral.ai/v1", "openai-completions")).toEqual(mistral);
  });

  it("applies only metadata to an immutable draft, preserving IDs and API overrides", () => {
    const model: ModelRecord = { id: "custom/alias", name: "Custom", contextWindow: null, maxTokens: null,
      reasoning: false, thinkingLevels: ["off"], thinkingLevelMap: { off: null }, vision: false, apiFormat: "openai-responses" };
    const applied = applyCatalogProfile(model, entry, native);
    expect(applied).toMatchObject({ id: model.id, name: entry.name, apiFormat: model.apiFormat, contextWindow: 128_000, vision: true });
    expect(model.contextWindow).toBeNull();
    expect(applied.thinkingLevels).not.toBe(native.thinkingLevels);
    expect(applyCatalogProfile({ ...model, id: "" }, entry, gateway).id).toBe("vendor/model");
    expect(applied).not.toHaveProperty("baseUrl");
    const partial = applyCatalogProfile(model, entry, { ...native, thinkingLevels: null, thinkingLevelMap: {}, contextWindow: null });
    expect(partial.thinkingLevels).toEqual(model.thinkingLevels);
    expect(partial.thinkingLevelMap).toEqual(model.thinkingLevelMap);
    expect(partial.contextWindow).toBeNull();
  });
});
