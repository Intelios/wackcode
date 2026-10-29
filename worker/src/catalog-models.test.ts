import { describe, expect, it } from "vitest";
import { builtinCompat } from "./catalog-models.js";

describe("custom-connection compat lookup", () => {
  it("copies adaptive thinking for a Claude model on Anthropic and on a gateway", () => {
    expect(builtinCompat("anthropic-messages", "claude-opus-4-6", "https://api.anthropic.com"))
      .toMatchObject({ forceAdaptiveThinking: true });
    expect(builtinCompat("anthropic-messages", "claude-opus-4-6", "https://opencode.ai/zen"))
      .toMatchObject({ forceAdaptiveThinking: true });
    expect(builtinCompat("anthropic-messages", "claude-opus-4-6", "https://mirror.example"))
      .toMatchObject({ forceAdaptiveThinking: true });
  });

  it("resolves snapshot-suffixed, dotted and vendor-prefixed spellings of one model", () => {
    expect(builtinCompat("anthropic-messages", "claude-opus-4.6", "https://api.anthropic.com"))
      .toMatchObject({ forceAdaptiveThinking: true });
    expect(builtinCompat("anthropic-messages", "anthropic/claude-opus-4.6", "https://opencode.ai/zen"))
      .toMatchObject({ forceAdaptiveThinking: true });
    expect(builtinCompat("anthropic-messages", "claude-opus-4-8-21000101", "https://api.anthropic.com"))
      .toMatchObject({ forceAdaptiveThinking: true });
  });

  it("never marks a dated budget-thinking snapshot adaptive", () => {
    const compat = builtinCompat("anthropic-messages", "claude-sonnet-4-5-20250929", "https://api.anthropic.com");
    expect(compat).toMatchObject({ supportsStrictTools: true });
    expect(compat?.forceAdaptiveThinking).not.toBe(true);
  });

  it("prefers the compat the catalogue wrote for the connection's own host", () => {
    const compat = builtinCompat("openai-completions", "deepseek-v4-flash", "https://opencode.ai/zen/go/v1");
    expect(compat).toMatchObject({ maxTokensField: "max_tokens", supportsStore: false });
  });

  it("treats a same-host catalogue entry with no compat as deliberate", () => {
    // OpenCode Go lists minimax-m3 without compat while its Fireworks upstream requires
    // adaptive thinking; the same-host listing wins over the foreign flags.
    expect(builtinCompat("anthropic-messages", "minimax-m3", "https://opencode.ai/zen/go")).toBeUndefined();
  });

  it("unions adaptive thinking across providers for a model on an unknown host", () => {
    expect(builtinCompat("anthropic-messages", "minimax-m3", "https://mirror.example"))
      .toMatchObject({ forceAdaptiveThinking: true });
  });

  it("only matches entries speaking the model's own API", () => {
    expect(builtinCompat("openai-completions", "claude-opus-4-6", "https://api.anthropic.com/v1")).toBeUndefined();
  });

  it("returns undefined for IDs outside the catalogue", () => {
    expect(builtinCompat("anthropic-messages", "totally-unknown-9", "https://api.anthropic.com")).toBeUndefined();
  });

  it("marks a Claude newer than the catalogue adaptive, wherever it is hosted", () => {
    // claude-haiku-9-9 is in no bundled catalogue (the test below fails if one ever lists it), yet
    // every Claude ≥ 4.6 requires adaptive thinking — the failure this lookup exists to prevent.
    for (const baseUrl of ["https://api.anthropic.com", "https://opencode.ai/zen", "https://mirror.example"]) {
      expect(builtinCompat("anthropic-messages", "claude-haiku-9-9", baseUrl))
        .toEqual({ forceAdaptiveThinking: true });
    }
  });

  it("leaves older or differently-named unknowns alone", () => {
    expect(builtinCompat("anthropic-messages", "claude-haiku-4-1", "https://api.anthropic.com")).toBeUndefined();
    expect(builtinCompat("anthropic-messages", "claude-3-7-sonnet", "https://api.anthropic.com")).toBeUndefined();
    expect(builtinCompat("openai-completions", "claude-haiku-9-9", "https://api.anthropic.com/v1")).toBeUndefined();
    expect(builtinCompat("anthropic-messages", "my-haiku-5-5", "https://mirror.example")).toBeUndefined();
  });

  it("shapes requests for Google Cloud's strict chat-completions endpoint", () => {
    // Unknown fields 400 there, so Pi's lenient-host defaults (store, developer role,
    // max_completion_tokens, stream_options) all have to be switched off or renamed.
    const vertex = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/endpoints/openapi";
    expect(builtinCompat("openai-completions", "gemini-3.8-flash", vertex)).toEqual({
      supportsStore: false,
      supportsDeveloperRole: false,
      supportsUsageInStreaming: false,
      supportsReasoningEffort: true,
      thinkingFormat: "openai",
      maxTokensField: "max_tokens",
      sendSessionAffinityHeaders: false,
      supportsLongCacheRetention: false
    });
    // The global Vertex host too.
    const global = vertex.replace("us-central1-", "");
    expect(builtinCompat("openai-completions", "gemini-2.5-flash", global)).toMatchObject({ supportsStore: false });
  });

  it("keeps streamed usage on the Gemini API's OpenAI route, which documents it", () => {
    const compat = builtinCompat("openai-completions", "gemini-3.8-flash", "https://generativelanguage.googleapis.com/v1beta/openai");
    expect(compat).toMatchObject({ supportsStore: false, maxTokensField: "max_tokens" });
    expect(compat).not.toHaveProperty("supportsUsageInStreaming");
  });

  it("overrides a matching catalogue entry: OpenRouter Gemini flags never reach Google's endpoint", () => {
    const compat = builtinCompat("openai-completions", "google/gemini-2.5-pro",
      "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/endpoints/openapi");
    expect(compat).not.toHaveProperty("supportsStrictMode");
    expect(compat).toMatchObject({ thinkingFormat: "openai", sendSessionAffinityHeaders: false, supportsStore: false });
  });

  it("leaves dedicated Model Garden deployments and other APIs on aiplatform alone", () => {
    // Outside …/endpoints/openapi the host runs vLLM/TGI containers, which keep Pi's
    // defaults and any catalogue match (here: OpenRouter's entry for the same ID).
    expect(builtinCompat("openai-completions", "meta/llama-4-maverick",
      "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/endpoints/99"))
      .toMatchObject({ thinkingFormat: "openrouter" });
    expect(builtinCompat("anthropic-messages", "claude-opus-4-6",
      "https://aiplatform.googleapis.com/v1beta1/projects/p/locations/g/endpoints/openapi"))
      .toMatchObject({ forceAdaptiveThinking: true });
  });
});
