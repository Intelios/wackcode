/**
 * Foreground policy for exposed conversation text over an image. Samples are conservative
 * channel bounds from the displayed crop, already composited over the window's solid base.
 * A channel interval also bounds any blur of its pixels: do not replace it with an average,
 * which would hide small bright patches and complementary colours.
 *
 * These variables are deliberately separate from the theme. Bubbles, cards and visual frames
 * retain their solid palettes; a frame or semantic caption may instead supply fixed text
 * constraints. The extra veil is black, after the user's background-colour dimming, so even
 * the theme's quieter fixed light text can be protected without changing its colour.
 */
import { DEFAULT_ACCENT, DEFAULT_BACKGROUND, parseHex, resolveTheme, toHex } from "./theme";

export type ReadabilityRgb = [number, number, number];
export type ReadabilityPolarity = "light" | "dark";

export interface ReadabilitySample {
  min: ReadabilityRgb;
  max: ReadabilityRgb;
}

export interface FixedReadabilityText {
  colour: string;
  ratio: number;
}

export interface ImageReadabilityInput {
  samples: readonly ReadabilitySample[];
  /** Resolved, opaque theme background. */
  background: string;
  accent: string;
  /** The saved slider value, 0–90. Never changed by this policy. */
  dim: number;
  /** Animated/uninspectable images and transparent light-themed visuals stay light. */
  forceLight: boolean;
  fixedText?: readonly FixedReadabilityText[];
  previous?: ReadabilityPolarity;
}

export interface ImageReadabilityResult {
  polarity: ReadabilityPolarity;
  /** Black opacity, 0–1, applied after the saved background-colour dim. */
  extraDim: number;
  variables: Record<string, string>;
}

/** Covers every image colour, including changing animation frames and unavailable samples. */
export const FALLBACK_READABILITY_SAMPLES: readonly ReadabilitySample[] = [
  { min: [0, 0, 0], max: [255, 255, 255] }
];

/** Small margin covers hex rounding and fractional CSS alpha/compositing. */
const TEXT_CONTRAST = 4.55;
const MAX_TEXT_ADJUSTMENT = 0.99;
const POLARITY_TIE = 0.02;
const SEARCH_STEPS = 18;

const clamp = (value: number, minimum: number, maximum: number) => Math.min(maximum, Math.max(minimum, value));
const finite = (value: number, fallback: number) => Number.isFinite(value) ? value : fallback;

function mix(first: ReadabilityRgb, second: ReadabilityRgb, amount: number): ReadabilityRgb {
  return first.map((channel, index) => channel * (1 - amount) + second[index] * amount) as ReadabilityRgb;
}

/** Decode-free alpha compositing for the sampler; alpha uses ImageData's 0–255 convention. */
export function compositeReadabilityPixel(pixel: readonly [number, number, number, number], background: string): ReadabilityRgb {
  const base = parseHex(background) ?? parseHex(DEFAULT_BACKGROUND)!;
  const alpha = clamp(finite(pixel[3], 255), 0, 255) / 255;
  const rgb = pixel.slice(0, 3).map((value) => clamp(finite(value, 0), 0, 255)) as ReadabilityRgb;
  return mix(base, rgb, alpha);
}

function luminance(rgb: ReadabilityRgb): number {
  const channels = rgb.map((channel) => {
    const encoded = clamp(channel, 0, 255) / 255;
    return encoded <= 0.04045 ? encoded / 12.92 : ((encoded + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

interface BrightnessBounds { min: number; max: number }

/** A foreground within a background interval can disappear on some pixel in that interval. */
function intervalContrast(foreground: number, bounds: BrightnessBounds): number {
  if (foreground >= bounds.max) return (foreground + 0.05) / (bounds.max + 0.05);
  if (foreground <= bounds.min) return (bounds.min + 0.05) / (foreground + 0.05);
  return 1;
}

/** Bound every region before collapsing its luminance range; RGB channel extrema are not grey. */
function brightnessBounds(samples: readonly ReadabilitySample[], base: ReadabilityRgb, dim: number, extraDim: number): BrightnessBounds {
  let min = 1;
  let max = 0;
  for (const sample of samples) {
    const lower = sample.min.map((channel, index) => Math.min(channel, sample.max[index])) as ReadabilityRgb;
    const upper = sample.max.map((channel, index) => Math.max(channel, sample.min[index])) as ReadabilityRgb;
    const afterVeil = (rgb: ReadabilityRgb) => mix(rgb, base, dim).map((channel) => channel * (1 - extraDim)) as ReadabilityRgb;
    min = Math.min(min, luminance(afterVeil(lower)));
    max = Math.max(max, luminance(afterVeil(upper)));
  }
  return { min, max };
}

function validSamples(samples: readonly ReadabilitySample[]): readonly ReadabilitySample[] {
  if (!samples.length || samples.some((sample) =>
    sample.min.length !== 3 || sample.max.length !== 3 ||
    [...sample.min, ...sample.max].some((value) => !Number.isFinite(value) || value < 0 || value > 255))) {
    return FALLBACK_READABILITY_SAMPLES;
  }
  return samples;
}

const NEUTRAL_TOKENS = [
  ["--wc-transcript-text", "--text"],
  ["--wc-transcript-body", "--wc-text-body"],
  ["--wc-transcript-soft", "--text-soft"],
  ["--wc-transcript-dim", "--text-dim"],
  ["--wc-transcript-faint", "--text-faint"]
] as const;

export function resolveImageReadability(input: ImageReadabilityInput): ImageReadabilityResult {
  const theme = resolveTheme({ background: input.background, accent: input.accent });
  const base = parseHex(input.background) ?? parseHex(DEFAULT_BACKGROUND)!;
  const accent = parseHex(input.accent) ?? parseHex(DEFAULT_ACCENT)!;
  const samples = validSamples(input.samples);
  const dim = clamp(finite(input.dim, 0), 0, 90) / 100;
  const fixed = (input.fixedText ?? []).flatMap(({ colour, ratio }) => {
    const rgb = parseHex(colour);
    return rgb && Number.isFinite(ratio) && ratio > 1 ? [{ luminance: luminance(rgb), ratio }] : [];
  });
  const boundsAtDim = new Map<number, BrightnessBounds>();
  const boundsFor = (extraDim: number) => {
    let bounds = boundsAtDim.get(extraDim);
    if (!bounds) {
      bounds = brightnessBounds(samples, base, dim, extraDim);
      boundsAtDim.set(extraDim, bounds);
    }
    return bounds;
  };

  // Shared adjustment retains the ordering of the palette: quieter labels do not each
  // independently converge to white/black. Difficult pictures can compress the differences
  // before we dim the image; an ordinary dark background keeps the familiar hierarchy.
  function palette(polarity: ReadabilityPolarity, adjustment: number): Record<string, string> {
    const endpoint: ReadabilityRgb = polarity === "light" ? [255, 255, 255] : [0, 0, 0];
    const variables: Record<string, string> = {};
    for (const [target, source] of NEUTRAL_TOKENS) {
      const light = parseHex(theme.variables[source])!;
      const preferred = polarity === "light" ? light : light.map((channel) => 255 - channel) as ReadabilityRgb;
      variables[target] = toHex(mix(preferred, endpoint, adjustment));
    }
    variables["--wc-transcript-accent"] = toHex(mix(accent, endpoint, adjustment));
    return variables;
  }

  function readable(polarity: ReadabilityPolarity, adjustment: number, extraDim: number, includeFixed = true): boolean {
    const bounds = boundsFor(extraDim);
    if (includeFixed && fixed.some((text) => intervalContrast(text.luminance, bounds) < text.ratio)) return false;
    return Object.values(palette(polarity, adjustment)).every((hex) => {
      const ink = luminance(parseHex(hex)!);
      return (polarity === "light" ? ink > bounds.max : ink < bounds.min) && intervalContrast(ink, bounds) >= TEXT_CONTRAST;
    });
  }

  function candidate(polarity: ReadabilityPolarity): ImageReadabilityResult | undefined {
    let extraDim = 0;
    if (!readable(polarity, MAX_TEXT_ADJUSTMENT, 0)) {
      // Dark text cannot benefit from a darker backdrop. Fixed light foregrounds also favour
      // the light candidate; they never get recoloured merely to keep a bright picture.
      if (polarity === "dark" || !readable(polarity, MAX_TEXT_ADJUSTMENT, 1)) return undefined;
      let low = 0;
      let high = 1;
      for (let step = 0; step < SEARCH_STEPS; step += 1) {
        const middle = (low + high) / 2;
        if (readable(polarity, MAX_TEXT_ADJUSTMENT, middle)) high = middle;
        else low = middle;
      }
      extraDim = high;
    }
    let low = 0;
    let high = MAX_TEXT_ADJUSTMENT;
    if (readable(polarity, 0, extraDim, false)) high = 0;
    else for (let step = 0; step < SEARCH_STEPS; step += 1) {
      const middle = (low + high) / 2;
      if (readable(polarity, middle, extraDim, false)) high = middle;
      else low = middle;
    }
    return { polarity, extraDim, variables: palette(polarity, high) };
  }

  const light = candidate("light");
  const dark = input.forceLight ? undefined : candidate("dark");
  if (light && dark) {
    if (Math.abs(light.extraDim - dark.extraDim) <= POLARITY_TIE) return input.previous === "dark" ? dark : light;
    return light.extraDim < dark.extraDim ? light : dark;
  }
  // Only fixed colours too dark to meet their requested ratio even on black can make both
  // candidates impossible. Keep the app-owned light text protected in that malformed case.
  return light ?? dark ?? { polarity: "light", extraDim: 1, variables: palette("light", MAX_TEXT_ADJUSTMENT) };
}
