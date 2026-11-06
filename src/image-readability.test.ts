import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { contrast, DEFAULT_ACCENT, DEFAULT_BACKGROUND, parseHex, resolveTheme, THEME_PRESETS, toHex } from "./theme";
import {
  compositeReadabilityPixel,
  FALLBACK_READABILITY_SAMPLES,
  resolveImageReadability,
  type ImageReadabilityInput,
  type ImageReadabilityResult,
  type ReadabilityRgb,
  type ReadabilitySample
} from "./image-readability";

const point = (rgb: ReadabilityRgb): ReadabilitySample => ({ min: rgb, max: rgb });
const defaults: ImageReadabilityInput = {
  samples: [point([0, 0, 0])],
  background: DEFAULT_BACKGROUND,
  accent: DEFAULT_ACCENT,
  dim: 0,
  forceLight: false
};

function backgroundAfterDim(rgb: ReadabilityRgb, input: ImageReadabilityInput, result: ImageReadabilityResult): string {
  const base = parseHex(input.background)!;
  const alpha = input.dim / 100;
  return toHex(rgb.map((channel, index) => (channel * (1 - alpha) + base[index] * alpha) * (1 - result.extraDim)) as ReadabilityRgb);
}

/** Test the endpoints independently of the resolver's interval luminance implementation. */
function expectReadable(input: ImageReadabilityInput, result = resolveImageReadability(input)): void {
  for (const sample of input.samples) {
    // Every corner matters for a saturated interval; greys alone would miss channel maths.
    for (let mask = 0; mask < 8; mask += 1) {
      const rgb = sample.min.map((minimum, channel) => mask & (1 << channel) ? sample.max[channel] : minimum) as ReadabilityRgb;
      const background = backgroundAfterDim(rgb, input, result);
      for (const [token, colour] of Object.entries(result.variables)) {
        expect(contrast(colour, background), `${token} ${colour} over ${background}`).toBeGreaterThanOrEqual(4.5);
      }
      for (const fixed of input.fixedText ?? []) {
        // Rounding the simulated CSS colour to a byte can shift the fixed-colour threshold.
        expect(contrast(fixed.colour, background), `fixed ${fixed.colour} over ${background}`).toBeGreaterThanOrEqual(fixed.ratio - 0.04);
      }
    }
  }
}

describe("image readability", () => {
  it("uses dark text on an undimmed white image without hiding the image", () => {
    const input = { ...defaults, samples: [point([255, 255, 255])] };
    const result = resolveImageReadability(input);
    expect(result).toMatchObject({ polarity: "dark", extraDim: 0 });
    expectReadable(input, result);
    expect(new Set(Object.values(result.variables)).size).toBeGreaterThanOrEqual(5);
  });

  it("uses light text on a black image and lifts quieter labels before adding dimming", () => {
    const result = resolveImageReadability(defaults);
    expect(result).toMatchObject({ polarity: "light", extraDim: 0 });
    expectReadable(defaults, result);
    expect(result.variables["--wc-transcript-faint"]).not.toBe(resolveTheme({}).variables["--text-faint"]);
    expect(new Set(Object.values(result.variables)).size).toBe(6);
  });

  it("dims a mixed light/dark image rather than trusting its average brightness", () => {
    const input = { ...defaults, samples: FALLBACK_READABILITY_SAMPLES };
    const result = resolveImageReadability(input);
    expect(result.polarity).toBe("light");
    expect(result.extraDim).toBeGreaterThan(0);
    expectReadable(input, result);
    // An averaged checkerboard would incorrectly appear to be a uniformly grey background.
    const average = resolveImageReadability({ ...defaults, samples: [point([128, 128, 128])] });
    expect(result.extraDim).toBeGreaterThan(average.extraDim);
  });

  it("bounds complementary saturated colours and all their possible blur mixtures", () => {
    const samples: ReadabilitySample[] = [
      { min: [0, 0, 0], max: [255, 255, 0] },
      { min: [0, 20, 0], max: [40, 255, 255] }
    ];
    for (const preset of THEME_PRESETS) {
      const input = { ...defaults, background: preset.background, accent: preset.accent, samples };
      expectReadable(input);
    }
  });

  it("composites transparent and partially transparent pixels over the solid theme base", () => {
    expect(compositeReadabilityPixel([255, 255, 255, 0], "#102030")).toEqual([16, 32, 48]);
    expect(compositeReadabilityPixel([255, 0, 0, 255], "#102030")).toEqual([255, 0, 0]);
    expect(compositeReadabilityPixel([255, 0, 0, 128], "#102030")[0]).toBeCloseTo(135.9686);
    const transparent = point(compositeReadabilityPixel([255, 255, 255, 0], DEFAULT_BACKGROUND));
    const input = { ...defaults, samples: [transparent] };
    expect(resolveImageReadability(input).polarity).toBe("light");
    expectReadable(input);
  });

  it("keeps saved dimming as the first layer without rewriting or reducing it", () => {
    const samples = [point([255, 255, 255])];
    const manual = { ...defaults, samples, dim: 80 };
    const snapshot = structuredClone(manual);
    const result = resolveImageReadability(manual);
    expect(result.polarity).toBe("light");
    expect(result.extraDim).toBe(0);
    expect(manual).toEqual(snapshot);
    expectReadable(manual, result);
    const forced = resolveImageReadability({ ...defaults, samples, forceLight: true });
    expect(forced.extraDim).toBeGreaterThan(result.extraDim);
  });

  it("keeps animated or uninspectable images on a conservatively dimmed light palette", () => {
    const input = { ...defaults, samples: FALLBACK_READABILITY_SAMPLES, forceLight: true };
    const result = resolveImageReadability(input);
    expect(result.polarity).toBe("light");
    expect(result.extraDim).toBeGreaterThan(0.5);
    expectReadable(input, result);
    expect(resolveImageReadability({ ...input, samples: [] })).toEqual(result);
    expect(resolveImageReadability({ ...input, samples: [{ min: [NaN, 0, 0], max: [255, 255, 255] }] })).toEqual(result);
  });

  it("protects fixed visual colours without changing their palette", () => {
    const theme = resolveTheme({});
    const fixedText = [
      { colour: theme.variables["--text"], ratio: 4.5 },
      { colour: theme.variables["--text-soft"], ratio: 4.5 },
      { colour: theme.variables["--text-dim"], ratio: 3.8 },
      { colour: theme.accent, ratio: 4.5 }
    ];
    const before = structuredClone(fixedText);
    const input = { ...defaults, samples: [point([255, 255, 255])], forceLight: true, fixedText };
    const result = resolveImageReadability(input);
    expect(result.polarity).toBe("light");
    expect(result.extraDim).toBeGreaterThan(resolveImageReadability({ ...input, fixedText: [] }).extraDim);
    expect(fixedText).toEqual(before);
    expectReadable(input, result);
  });

  it("includes exposed semantic warning text in its contrast constraints", () => {
    const stylesheet = readFileSync(resolve(__dirname, "styles.css"), "utf8");
    const warning = /--warning:\s*(#[\da-f]{6})\b/i.exec(stylesheet)?.[1];
    expect(warning).toBeDefined();
    const input = {
      ...defaults,
      samples: [point([255, 255, 255])],
      fixedText: [{ colour: warning!, ratio: 4.5 }]
    };
    const result = resolveImageReadability(input);
    expect(result.polarity).toBe("light");
    expectReadable(input, result);
  });

  it("retains the current polarity for near ties but changes it when contrast requires it", () => {
    let nearTie: ImageReadabilityInput | undefined;
    for (let grey = 110; grey <= 150; grey += 1) {
      const input = { ...defaults, samples: [point([grey, grey, grey])] };
      const keptDark = resolveImageReadability({ ...input, previous: "dark" });
      const keptLight = resolveImageReadability({ ...input, previous: "light" });
      if (keptDark.polarity === "dark" && keptLight.polarity === "light") { nearTie = input; break; }
    }
    expect(nearTie).toBeDefined();
    expectReadable(nearTie!, resolveImageReadability({ ...nearTie!, previous: "dark" }));
    expectReadable(nearTie!, resolveImageReadability({ ...nearTie!, previous: "light" }));
    expect(resolveImageReadability({ ...defaults, previous: "dark" }).polarity).toBe("light");
    expect(resolveImageReadability({ ...defaults, samples: [point([255, 255, 255])], previous: "light" }).polarity).toBe("dark");
  });

  it("uses only dedicated transcript tokens and leaves global theme values alone", () => {
    const result = resolveImageReadability(defaults);
    expect(Object.keys(result.variables).sort()).toEqual([
      "--wc-transcript-accent", "--wc-transcript-body", "--wc-transcript-dim",
      "--wc-transcript-faint", "--wc-transcript-soft", "--wc-transcript-text"
    ]);
  });
});
