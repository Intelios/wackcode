import { describe, expect, it } from "vitest";
import { expandTemplate } from "./slash.js";

describe("prompt-template arguments", () => {
  it("keeps Pi's quoted, positional, default, and slice substitutions", () => {
    expect(expandTemplate("$1 | $2 | $ARGUMENTS | ${3:-fallback} | ${@:2}", '"two words" next'))
      .toBe("two words | next | two words next | fallback | next");
  });

  it("does not expand placeholders introduced by an argument", () => {
    expect(expandTemplate("$1", "'$ARGUMENTS'")).toBe("$ARGUMENTS");
  });
});
