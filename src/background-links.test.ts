import { describe, expect, it } from "vitest";
import { DEFAULT_APPEARANCE, THEME_PRESETS, presetById, presetForColours } from "./theme";
import { imageLinkedTo, linkImageToTheme, linkedThemeConfig, shownEntry } from "./background-links";
import type { AppearanceConfig } from "./types";

const MONO = THEME_PRESETS.find((preset) => preset.id === "mono")!;

function library(entries: { name: string; theme?: string | null }[], shown?: string): AppearanceConfig {
  return {
    ...DEFAULT_APPEARANCE,
    backdrop: "image",
    backgroundImage: shown ?? entries[0]?.name ?? null,
    backgroundImages: entries.map((entry) => ({ name: entry.name, zoom: 100, x: 500, y: 500, theme: entry.theme ?? null }))
  };
}

describe("presetForColours", () => {
  it("matches a preset by its exact accent and background pair", () => {
    expect(presetForColours("#e6e6e6", "#111111")?.id).toBe("mono");
    expect(presetForColours(null, null)?.id).toBe("wackcode");
    expect(presetForColours(undefined, undefined)?.id).toBe("wackcode");
  });

  it("ignores hex case and shorthand", () => {
    expect(presetForColours("#E6E6E6", "#111111")?.id).toBe("mono");
  });

  it("matches nothing for a custom combination", () => {
    expect(presetForColours("#e6e6e6", "#111310")).toBeUndefined();
    expect(presetForColours("#123456", "#111111")).toBeUndefined();
    expect(presetForColours("#c2ee4a", "#abcdef")).toBeUndefined();
  });
});

describe("presetById", () => {
  it("resolves a preset id and nothing else", () => {
    expect(presetById("mono")).toEqual(MONO);
    expect(presetById("no-such-preset")).toBeUndefined();
    expect(presetById(null)).toBeUndefined();
    expect(presetById(undefined)).toBeUndefined();
  });
});

describe("shownEntry and imageLinkedTo", () => {
  it("finds the shown entry and the image linked to a preset", () => {
    const config = library([{ name: "a.png", theme: "mono" }, { name: "b.png" }], "b.png");
    expect(shownEntry(config)?.name).toBe("b.png");
    expect(imageLinkedTo(config, "mono")?.name).toBe("a.png");
    expect(imageLinkedTo(config, "rose")).toBeUndefined();
    expect(shownEntry(library([{ name: "a.png" }], "gone.png"))).toBeUndefined();
  });
});

describe("linkImageToTheme", () => {
  it("links one image and moves the preset off any other image", () => {
    const config = library([{ name: "a.png" }, { name: "b.png", theme: "mono" }, { name: "c.png", theme: "rose" }]);
    const next = linkImageToTheme(config, "a.png", "mono");
    expect(next.backgroundImages.map((image) => [image.name, image.theme])).toEqual([["a.png", "mono"], ["b.png", null], ["c.png", "rose"]]);
  });

  it("unlinks with null, touching only that image", () => {
    const config = library([{ name: "a.png", theme: "mono" }, { name: "b.png", theme: "mono" }]);
    const next = linkImageToTheme(config, "a.png", null);
    expect(next.backgroundImages.map((image) => [image.name, image.theme])).toEqual([["a.png", null], ["b.png", "mono"]]);
  });
});

describe("linkedThemeConfig", () => {
  it("applies the shown image's linked preset, saved the way the preset tile saves it", () => {
    const config = library([{ name: "a.png", theme: "mono" }, { name: "b.png" }], "a.png");
    expect(linkedThemeConfig(config)).toEqual({ ...config, accent: "#e6e6e6", background: "#111111" });
  });

  it("saves the default preset's colours as unset", () => {
    // Currently on Mono, showing the image linked to WackCode: back to the unset defaults.
    const config = { ...library([{ name: "a.png", theme: "wackcode" }], "a.png"), accent: "#e6e6e6", background: "#111111" };
    expect(linkedThemeConfig(config)).toEqual({ ...config, accent: null, background: null });
  });

  it("returns null when there is nothing to apply", () => {
    // Already on the linked preset.
    expect(linkedThemeConfig({ ...library([{ name: "a.png", theme: "mono" }], "a.png"), accent: "#e6e6e6", background: "#111111" })).toBeNull();
    // The shown image has no link.
    expect(linkedThemeConfig(library([{ name: "a.png", theme: "mono" }, { name: "b.png" }], "b.png"))).toBeNull();
    // No image shown at all.
    expect(linkedThemeConfig(library([{ name: "a.png", theme: "mono" }], "gone.png"))).toBeNull();
    // A hand-edited id no preset has.
    expect(linkedThemeConfig(library([{ name: "a.png", theme: "hacked" }], "a.png"))).toBeNull();
  });
});
