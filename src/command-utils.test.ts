import { describe, expect, it } from "vitest";
import { activeSlashCommand, APP_SLASH_COMMANDS, expandCommandPreview, validateCommandBody, validateCommandName } from "./command-utils";

describe("activeSlashCommand", () => {
  it("finds commands at the caret after prose and on later lines", () => {
    expect(activeSlashCommand("/he", 3)).toEqual({ start: 0, end: 3, query: "he" });
    expect(activeSlashCommand("Please /he world", 10)).toEqual({ start: 7, end: 10, query: "he" });
    expect(activeSlashCommand("First line\n\t/skill:pdf", 21)).toEqual({ start: 12, end: 22, query: "skill:pd" });
  });

  it("replaces the whole token when the caret is inside it", () => {
    expect(activeSlashCommand("Try /hello now", 7)).toEqual({ start: 4, end: 10, query: "he" });
    expect(activeSlashCommand("/hello world", 12)).toBeNull();
    expect(activeSlashCommand("Try /hello", 4)).toBeNull();
  });

  it("ignores URLs, path segments and double slashes", () => {
    for (const text of ["https://example.com/he", "src/hello", "@src/hello", "/tmp/hello", "//hello"]) {
      expect(activeSlashCommand(text, text.length)).toBeNull();
    }
    expect(activeSlashCommand("See /tmp/hello", 8)).toBeNull();
  });
});

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

describe("APP_SLASH_COMMANDS", () => {
  it("hints at exactly the app commands that accept arguments", () => {
    const hints = new Map(APP_SLASH_COMMANDS.map((command) => [command.name, command.argumentHint]));
    expect(hints.get("compact")).toBe("[instructions]");
    expect(hints.get("name")).toBe("<name>");
    expect(hints.get("goal")).toBe("<objective>");
    expect(hints.get("init")).toBeUndefined();
    expect(hints.get("new")).toBeUndefined();
    expect(hints.get("copy")).toBeUndefined();
  });
});
