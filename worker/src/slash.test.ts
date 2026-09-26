import { describe, expect, it } from "vitest";
import { commandKey, expandTemplate, resolveCommandNames, resolveInvocationNames } from "./slash.js";

describe("resolveCommandNames", () => {
  it("keeps the first of a name and renames the rest as <source>:<name>", () => {
    expect(resolveCommandNames([
      { source: "extension", invocation: "review" },
      { source: "custom", invocation: "review" },
      { source: "prompt", invocation: "review" }
    ])).toEqual([{ name: "review" }, { name: "custom:review", raw: "review" }, { name: "prompt:review", raw: "review" }]);
  });

  it("protects the app's own names", () => {
    expect(resolveCommandNames([{ source: "extension", invocation: "new" }])[0].name).toBe("extension:new");
    expect(resolveCommandNames([{ source: "extension", invocation: "goal" }])[0].name).toBe("extension:goal");
  });
});

describe("resolveInvocationNames", () => {
  it("mirrors the extension runner: shared names become name:<occurrence>", () => {
    const resolved = resolveInvocationNames([{ name: "a" }, { name: "a" }, { name: "b" }]);
    expect(resolved.map((command) => command.invocationName)).toEqual(["a:1", "a:2", "b"]);
  });
});

describe("commandKey", () => {
  it("keys extensions by file and registered name, files by path", () => {
    expect(commandKey("extension", "/pkg/x.ts", "hi")).toBe("extension:/pkg/x.ts#hi");
    expect(commandKey("custom", "/cmd/x.md")).toBe("custom:/cmd/x.md");
    expect(commandKey("prompt", "/pkg/x.md")).toBe("prompt:/pkg/x.md");
  });
});

describe("prompt-template arguments", () => {
  it("keeps Pi's quoted, positional, default, and slice substitutions", () => {
    expect(expandTemplate("$1 | $2 | $ARGUMENTS | ${3:-fallback} | ${@:2}", '"two words" next'))
      .toBe("two words | next | two words next | fallback | next");
  });

  it("does not expand placeholders introduced by an argument", () => {
    expect(expandTemplate("$1", "'$ARGUMENTS'")).toBe("$ARGUMENTS");
  });
});
