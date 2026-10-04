import { describe, expect, it } from "vitest";
import { computerCursorAppearance } from "./computer-cursor";
import { DEFAULT_APPEARANCE, resolveTheme } from "./theme";

describe("native computer cursor appearance", () => {
  it("uses the configured persona and resolved, readable accent", () => {
    const appearance = { ...DEFAULT_APPEARANCE, agentName: "  Nova  ", accent: "#000001" };
    const theme = resolveTheme(appearance);
    const cursor = computerCursorAppearance(appearance, theme);
    expect(cursor.agentName).toBe("Nova");
    expect(cursor.accent).toBe(theme.accent);
    expect(cursor.accent).not.toBe(appearance.accent);
    expect(cursor.outline).toBe(theme.variables["--wc-on-accent"]);
  });

  it("keeps the label solid in Glass and image modes and follows theme changes", () => {
    for (const backdrop of ["solid", "glass", "image"] as const) {
      const appearance = { ...DEFAULT_APPEARANCE, backdrop, accent: "#b69cff" };
      const theme = resolveTheme(appearance);
      const cursor = computerCursorAppearance(appearance, theme);
      expect(cursor.agentName).toBe("WackCode");
      expect(cursor.surface).toBe(theme.variables["--wc-elevated"]);
      expect(cursor.surface).toMatch(/^#[0-9a-f]{6}$/);
      expect(cursor.text).toBe(theme.variables["--text"]);
      expect(cursor.mutedText).toBe(theme.variables["--text-soft"]);
      expect(cursor.accent).toBe(theme.accent);
    }
  });
});
