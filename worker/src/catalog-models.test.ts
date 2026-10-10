import { describe, expect, it } from "vitest";
import { builtinCompat, listBuiltinModelSuggestions } from "./catalog-models.js";

describe("bundled Pi model suggestions", () => {
  it("projects DeepSeek V4.1 Flash limits and supported reasoning levels", () => {
    const models = listBuiltinModelSuggestions();
    const flash = models.find((model) => model.sourceProvider === "deepseek" && model.id === "deepseek-flash");
    expect(flash).toMatchObject({
      sourceApi: "openai-completions",
      name: "DeepSeek V4.1 Flash",
      contextWindow: 1_000_000,
      maxTokens: 384_000,
      reasoning: true,
      thinkingLevels: ["off", "low", "high", "max"],
      thinkingLevelMap: { off: null, low: "low", high: "high", max: "max" },
      vision: true
    });
    expect(flash?.thinkingLevelMap).not.toHaveProperty("medium");
  });
});

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
    // claude-haiku-5-5 predates the bundled catalogue's newest Haiku (4.5), yet every Claude
    // ≥ 4.6 requires adaptive thinking — the failure this lookup exists to prevent.
    for (const baseUrl of ["https://api.anthropic.com", "https://opencode.ai/zen", "https://mirror.example"]) {
      expect(builtinCompat("anthropic-messages", "claude-haiku-5-5", baseUrl))
        .toMatchObject({ forceAdaptiveThinking: true });
    }
  });

  it("leaves older or differently-named unknowns alone", () => {
    expect(builtinCompat("anthropic-messages", "claude-haiku-4-1", "https://api.anthropic.com")).toBeUndefined();
    expect(builtinCompat("anthropic-messages", "claude-3-7-sonnet", "https://api.anthropic.com")).toBeUndefined();
    expect(builtinCompat("openai-completions", "claude-haiku-5-5", "https://api.anthropic.com/v1")).toBeUndefined();
    expect(builtinCompat("anthropic-messages", "my-haiku-5-5", "https://mirror.example")).toBeUndefined();
  });
});
