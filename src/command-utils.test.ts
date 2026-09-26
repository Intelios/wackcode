import { describe, expect, it } from "vitest";
import { expandCommandPreview, validateCommandBody, validateCommandName } from "./command-utils";

describe("validateCommandName", () => {
  it("accepts prompt-template style names and refuses the rest", () => {
    expect(validateCommandName("review-diff")).toBeUndefined();
    expect(validateCommandName("v2")).toBeUndefined();
    expect(validateCommandName("")).toMatch(/name/);
    expect(validateCommandName("Review")).toMatch(/lowercase/);
    expect(validateCommandName("my command")).toMatch(/lowercase/);
    expect(validateCommandName("-lead")).toMatch(/lowercase/);
    expect(validateCommandName("trail-")).toMatch(/lowercase/);
    expect(validateCommandName("a".repeat(65))).toMatch(/64/);
  });
});

describe("validateCommandBody", () => {
  it("needs instructions", () => {
    expect(validateCommandBody("  ")).toMatch(/instructions/);
    expect(validateCommandBody("Do it.")).toBeUndefined();
  });
});

describe("expandCommandPreview", () => {
  it("follows Pi's argument rules, like the worker's expandTemplate", () => {
    expect(expandCommandPreview("Check $1 and $2", "a.ts b.ts")).toBe("Check a.ts and b.ts");
    expect(expandCommandPreview("All: $ARGUMENTS", "a b c")).toBe("All: a b c");
    expect(expandCommandPreview("All: $@", "a b")).toBe("All: a b");
    expect(expandCommandPreview("${1:-none}", "")).toBe("none");
    expect(expandCommandPreview("${@:2}", "a b c")).toBe("b c");
    expect(expandCommandPreview("Say 'x y' $1", "a b")).toBe("Say 'x y' a");
    expect(expandCommandPreview("Split $2", "'two words' end")).toBe("Split end");
  });
});
