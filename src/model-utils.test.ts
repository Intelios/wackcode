import { describe, expect, it } from "vitest";
import { mergeDiscoveredModels } from "./model-utils";

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
