import { describe, expect, it } from "vitest";
import type { NormalizedBlock } from "./types";
import { completedImageComparison } from "./image-comparisons";

const details = { v: 1, before: "wackcode-image://entry/upload/0", after: "wackcode-image://entry/read/0", title: "New layout", beforeLabel: "Original", afterLabel: "Edited" };
const result = (extra: Partial<NormalizedBlock> = {}): NormalizedBlock => ({ type: "tool-result", details, ...extra });

describe("accepted image comparisons", () => {
  it("uses compact versioned details as the record", () => {
    expect(completedImageComparison(result())).toEqual(details);
    expect(completedImageComparison()).toBeUndefined();
    expect(completedImageComparison(result({ isError: true }))).toBeUndefined();
    for (const invalid of [null, [], { ...details, v: 2 }, { ...details, before: "/tmp/a.png" }, { ...details, after: "https://example.com/image.png" }, { ...details, title: "" }, { ...details, afterLabel: "a".repeat(33) }]) {
      expect(completedImageComparison(result({ details: invalid }))).toBeUndefined();
    }
  });
});
