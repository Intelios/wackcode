/// <reference types="node" />
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_ACCENT,
  DEFAULT_BACKGROUND,
  MIN_ACCENT_CONTRAST,
  MIN_DIM_TEXT_CONTRAST,
  THEME_PRESETS,
  TOKENS,
  applyTheme,
  contrast,
  parseHex,
  resolveTheme,
  swatchTint
} from "./theme";

const stylesheet = readFileSync(resolve(__dirname, "styles.css"), "utf8");
const rootBlock = stylesheet.slice(0, stylesheet.indexOf("}\n") + 1);
const belowRoot = stylesheet.slice(rootBlock.length);

/**
 * Colours that are allowed to stay literal below `:root`: they carry meaning (danger, warning,
 * diff, success, the stop button) or are pure black/white for masks and highlights. A themeable
 * colour must be a TOKENS entry instead.
 */
const SEMANTIC_COLOURS = new Set([
  "#000", "#fff",
  // danger
  "#5b3030", "#ebb1b1", "#e3a2a2", "#e99191", "#381f1f", "#5b3333", "#cf9191", "#301c1c", "#f0baba",
  // warning
  "#5c4a22", "#e6d2a6", "#574a2d", "#493b20", "#2c2515", "#d9c491", "#211805", "#221c0f", "#332a17",
  // diff and success
  "#a8d883", "#b7d9a1", "#accd72", "#1d301b", "#9aaed2", "#232c3f",
  // stop button
  "#2b201c", "#e8b8a5", "#594037", "#37251f"
]);

describe("theme tokens", () => {
  it("reproduce the original palette exactly for the default accent and background", () => {
    const { variables } = resolveTheme({});
    for (const [name, { ref }] of Object.entries(TOKENS)) expect(variables[name], name).toBe(ref);
    expect(variables["--wc-accent"]).toBe(DEFAULT_ACCENT);
  });

  it("match the :root defaults in styles.css, which paint before the theme is applied", () => {
    const declared = new Map([...rootBlock.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((match) => [match[1], match[2].trim()]));
    for (const [name, value] of Object.entries(resolveTheme({}).variables)) expect(declared.get(name), name).toBe(value);
  });

  it("leave no themeable colour hard-coded in styles.css", () => {
    const literals = [...belowRoot.matchAll(/#[0-9a-f]{3,8}\b/gi)].map((match) => match[0].toLowerCase());
    expect(literals.filter((hex) => !SEMANTIC_COLOURS.has(hex))).toEqual([]);
    // The old accent as an rgb() glow: use color-mix with var(--wc-accent).
    expect(belowRoot).not.toMatch(/rgb\(194 238 74/);
  });

  it("fall back to the defaults for missing or malformed colours", () => {
    expect(resolveTheme({ accent: null, background: undefined })).toMatchObject({ accent: DEFAULT_ACCENT, background: DEFAULT_BACKGROUND });
    expect(resolveTheme({ accent: "green", background: "#12345" })).toMatchObject({ accent: DEFAULT_ACCENT, background: DEFAULT_BACKGROUND });
  });

  it("keep every preset exactly as picked, with readable text on the accent", () => {
    for (const preset of THEME_PRESETS) {
      const theme = resolveTheme(preset);
      expect(theme, preset.name).toMatchObject({ accent: preset.accent, background: preset.background, accentAdjusted: false, backgroundAdjusted: false });
      expect(contrast(theme.variables["--wc-on-accent"], theme.accent), preset.name).toBeGreaterThanOrEqual(MIN_ACCENT_CONTRAST);
    }
  });

  it("darken backgrounds too light for the fixed light text", () => {
    for (const background of ["#ffffff", "#808080", "#2a2a2a", "#3a2050", "#ff0000", "#00ffee", "#445566"]) {
      const theme = resolveTheme({ background });
      const { variables } = theme;
      expect(theme.backgroundAdjusted, background).toBe(true);
      expect(contrast(variables["--text"], variables["--bg"]), background).toBeGreaterThanOrEqual(12);
      expect(contrast(variables["--text-dim"], variables["--bg"]), background).toBeGreaterThanOrEqual(MIN_DIM_TEXT_CONTRAST - 0.05);
    }
    for (const background of ["#000000", "#111111", "#0f1218", "#1e1e2e"]) {
      expect(resolveTheme({ background }).background, background).toBe(background);
    }
  });

  it("lighten accents that would not read as text on the background", () => {
    for (const accent of ["#1d4ed8", "#5b21b6", "#7f1d1d", "#000000"]) {
      const theme = resolveTheme({ accent });
      expect(theme.accentAdjusted, accent).toBe(true);
      expect(contrast(theme.accent, theme.background), accent).toBeGreaterThanOrEqual(MIN_ACCENT_CONTRAST);
    }
  });

  it("keep near-black backgrounds' surfaces distinguishable", () => {
    const { variables } = resolveTheme({ background: "#000000" });
    const lightness = (name: string) => parseHex(variables[name])!.reduce((sum, channel) => sum + channel, 0);
    expect(lightness("--border")).toBeGreaterThan(lightness("--surface-hover"));
    expect(lightness("--surface-hover")).toBeGreaterThan(lightness("--surface") + 20);
  });

  it("lift a swatch's near-black fill into a rim its hue can be seen in", () => {
    const sum = (rgb: number[]) => rgb[0] + rgb[1] + rgb[2];
    // The rim keeps the fill's hue: the two rank their channels the same way.
    const order = (rgb: number[]) => rgb.map((_, index) => index).sort((a, b) => rgb[b] - rgb[a]);
    for (const preset of THEME_PRESETS) {
      const tint = parseHex(swatchTint(preset.background))!;
      const fill = parseHex(preset.background)!;
      expect(sum(tint), preset.name).toBeGreaterThan(sum(fill) * 3);
      expect(order(tint), preset.name).toEqual(order(fill));
    }
  });

  it("tell the preset backgrounds' rims apart, which the fills alone never do", () => {
    const tints = THEME_PRESETS.map((preset) => parseHex(swatchTint(preset.background))!);
    for (let first = 0; first < tints.length; first += 1) {
      for (let second = first + 1; second < tints.length; second += 1) {
        const apart = Math.hypot(...tints[first].map((channel, index) => channel - tints[second][index]));
        expect(apart, `${THEME_PRESETS[first].name} / ${THEME_PRESETS[second].name}`).toBeGreaterThan(30);
      }
    }
  });

  it("leave a grey background grey, and an unparseable colour untouched", () => {
    const grey = parseHex(swatchTint("#111111"))!;
    expect(grey[0]).toBe(grey[1]);
    expect(grey[1]).toBe(grey[2]);
    expect(grey[0]).toBeGreaterThan(100);
    expect(swatchTint("not a colour")).toBe("not a colour");
  });

  it("turns only the shell see-through over an image or Liquid Glass", () => {
    const solid = resolveTheme({}).variables;
    const image = resolveTheme({ backdrop: "image" }).variables;
    const glass = resolveTheme({ backdrop: "glass", glassTint: 30 }).variables;
    expect(solid["--wc-shell"]).toBe(DEFAULT_BACKGROUND);
    expect(image["--wc-shell"]).toBe("transparent");
    expect(glass["--wc-shell"]).toBe("rgb(17 19 16 / 30%)");
    expect(image["--wc-sidebar"]).toMatch(/^rgb\(.+ \/ \d+%\)$/);
    // Content surfaces stay solid, so chats stay legible over any backdrop.
    for (const name of ["--surface", "--surface-raised", "--wc-composer", "--wc-elevated", "--wc-input", "--wc-well"]) {
      expect(image[name], name).toBe(solid[name]);
      expect(glass[name], name).toBe(solid[name]);
    }
  });

  it("writes every variable onto the root element", () => {
    const root = document.createElement("div");
    const theme = applyTheme({ accent: "#6cc4ff", background: "#0f1218" }, root);
    expect(root.style.getPropertyValue("--wc-accent")).toBe("#6cc4ff");
    expect(root.style.getPropertyValue("--surface")).toBe(theme.variables["--surface"]);
    expect(root.dataset.backdrop).toBe("solid");
    expect(root.dataset.messageBubbles).toBe("off");
    applyTheme({ backdrop: "glass", messageBubbles: true }, root);
    expect(root.dataset.backdrop).toBe("glass");
    expect(root.dataset.messageBubbles).toBe("on");
  });
});
