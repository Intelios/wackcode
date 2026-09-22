import { describe, expect, it } from "vitest";
import { listBuiltinModelSuggestions } from "./catalog-models.js";

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
