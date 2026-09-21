import { describe, expect, it } from "vitest";
import { displayPath, formatTokens, titleFromPrompt } from "./chat-utils";

describe("titleFromPrompt", () => {
  it("uses the first non-empty line", () => {
    expect(titleFromPrompt("Fix the login bug\n\nMore context here")).toBe("Fix the login bug");
  });

  it("skips leading blank lines", () => {
    expect(titleFromPrompt("\n\n  add tests  ")).toBe("add tests");
  });

  it("truncates long prompts", () => {
    const title = titleFromPrompt("x".repeat(100));
    expect(title.length).toBe(48);
    expect(title.endsWith("…")).toBe(true);
  });

  it("falls back to a default", () => {
    expect(titleFromPrompt("   \n")).toBe("New chat");
  });
});

describe("displayPath", () => {
  it("shows short paths in full", () => {
    expect(displayPath("src/App.tsx")).toBe("src/App.tsx");
  });

  it("trims absolute paths to the last segments", () => {
    expect(displayPath("/Users/jack/project/src/App.tsx")).toBe("…/src/App.tsx");
  });
});

describe("formatTokens", () => {
  it("formats magnitudes", () => {
    expect(formatTokens(500)).toBe("500");
    expect(formatTokens(12_300)).toBe("12.3k");
    expect(formatTokens(2_500_000)).toBe("2.5m");
  });
});
