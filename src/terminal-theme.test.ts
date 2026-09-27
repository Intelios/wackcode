import { describe, expect, it } from "vitest";
import { parseHex, resolveTheme } from "./theme";
import { terminalTheme } from "./terminal-theme";

const variables = resolveTheme({}).variables;

/** How much `b` lightened `a`, as a shared brightness delta (lift moves all channels together-ish). */
function brightness(hex: string): number {
  return parseHex(hex)!.reduce((sum, channel) => sum + channel, 0);
}

describe("terminalTheme", () => {
  it("maps the ANSI palette from the --wc-term-* tokens", () => {
    const theme = terminalTheme(variables);
    for (const name of ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const) {
      expect(theme[name], name).toBe(variables[`--wc-term-${name}`]);
    }
    expect(theme.background).toBe(variables["--wc-well"]);
    expect(theme.foreground).toBe(variables["--wc-text-body"]);
  });

  it("derives bright variants lifted toward the bright white, and keeps bright black a grey", () => {
    const theme = terminalTheme(variables);
    for (const name of ["Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"] as const) {
      const bright = theme[`bright${name}`]!;
      expect(bright, name).not.toBe(theme[name.toLowerCase() as "red"]);
      expect(brightness(bright), name).toBeGreaterThan(brightness(variables[`--wc-term-${name.toLowerCase()}`]));
    }
    // Bright black stays a readable dim grey, not a washed-out panel tone.
    const brightBlack = parseHex(theme.brightBlack!)!;
    const spread = Math.max(...brightBlack) - Math.min(...brightBlack);
    expect(spread).toBeLessThan(40);
    expect(brightness(theme.brightBlack!)).toBeGreaterThan(brightness(variables["--wc-term-black"]));
  });

  it("follows the accent for cursor and selection, tinted for on-accent text", () => {
    const theme = terminalTheme(resolveTheme({ accent: "#6cc4ff" }).variables);
    expect(theme.cursor).toBe("#6cc4ff");
    expect(theme.cursorAccent).toMatch(/^#[0-9a-f]{6}$/i);
    expect(theme.selectionBackground).toMatch(/^#6cc4ff[0-9a-f]{2}$/i);
  });

  it("retints when the accent changes, without touching the ANSI tokens", () => {
    const theme = terminalTheme(resolveTheme({ accent: "#ff71a6" }).variables);
    expect(theme.cursor).toBe("#ff71a6");
    expect(theme.red).toBe(resolveTheme({ accent: "#ff71a6" }).variables["--wc-term-red"]);
  });

  it("falls back gracefully when variables are missing or unparseable", () => {
    const theme = terminalTheme({});
    expect(theme.background).toBe("#0c0e0b");
    expect(theme.foreground).toBe("#dbe0d7");
    expect(theme.red).toBe("#808080");
    const partial = terminalTheme({ "--wc-term-red": "not-a-colour", "--wc-term-blue": "#123456" });
    expect(partial.red).toBe("#808080");
    expect(partial.blue).toBe("#123456");
    expect(partial.brightBlue).toMatch(/^#[0-9a-f]{6}$/i);
  });
});
