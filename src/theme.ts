/**
 * Theme tokens: every themeable colour in `styles.css` is a CSS variable computed here from two
 * user choices, the accent and the background colour (Settings › Appearance).
 *
 * Each token is described by a recipe anchored on today's hand-picked hex (`ref`). A recipe
 * records how `ref` relates to the default accent/background, then applies the same relation
 * to the user's colours, so the default theme reproduces the original palette. `styles.css`
 * keeps those defaults in `:root` for the first paint; `theme.test.ts` pins both.
 *
 * Semantic colours (danger, warning, diff, success, Ultra Plan) are deliberately not here: an
 * accent recolours interaction, never meaning.
 */

import type { AppearanceConfig, BackdropMode } from "./types";

export const DEFAULT_ACCENT = "#c2ee4a";
/** Also `DEFAULT_BACKGROUND` in `src-tauri/src/glass.rs`, which paints the native window. */
export const DEFAULT_BACKGROUND = "#111310";

/** Mirrors `AppearanceConfig::default()` in models.rs. */
export const DEFAULT_APPEARANCE: AppearanceConfig = {
  thinkingPreview: true,
  messageBubbles: false,
  backdrop: "solid",
  imageDim: 65,
  imageBlur: 12,
  glassStyle: "frosted",
  glassTint: 40
};

/** One-click accent + background pairs (Settings › Appearance). The first is the default. */
export const THEME_PRESETS: { id: string; name: string; accent: string; background: string }[] = [
  { id: "wackcode", name: "WackCode", accent: DEFAULT_ACCENT, background: DEFAULT_BACKGROUND },
  { id: "midnight", name: "Midnight", accent: "#6cc4ff", background: "#0f1218" },
  { id: "grape", name: "Grape", accent: "#b69cff", background: "#14111b" },
  { id: "rose", name: "Rosé", accent: "#ff8fb8", background: "#181114" },
  { id: "ember", name: "Ember", accent: "#ff9a5c", background: "#17120f" },
  { id: "mono", name: "Mono", accent: "#e6e6e6", background: "#111111" }
];

type Rgb = [number, number, number];
type Lab = [number, number, number];
interface Lch {
  l: number;
  c: number;
  h: number;
}

// ── Colour maths ──────────────────────────────────────

/** `#rrggbb` (or `#rgb`) to 0–255 channels; undefined for anything else. */
export function parseHex(hex: string): Rgb | undefined {
  const match = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
  if (!match) return undefined;
  const digits = match[1].length === 3 ? [...match[1]].map((digit) => digit + digit).join("") : match[1];
  return [0, 2, 4].map((offset) => parseInt(digits.slice(offset, offset + 2), 16)) as Rgb;
}

export function toHex(rgb: Rgb): string {
  return `#${rgb.map((channel) => Math.round(Math.min(255, Math.max(0, channel))).toString(16).padStart(2, "0")).join("")}`;
}

function toLinear(channel: number): number {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function fromLinear(value: number): number {
  const encoded = value <= 0.0031308 ? value * 12.92 : 1.055 * value ** (1 / 2.4) - 0.055;
  return encoded * 255;
}

function rgbToOklab([red, green, blue]: Rgb): Lab {
  const r = toLinear(red), g = toLinear(green), b = toLinear(blue);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
  ];
}

/** Unclamped linear sRGB; channels outside 0–1 mean the colour is out of gamut. */
function oklabToLinear([lightness, a, b]: Lab): Rgb {
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s
  ];
}

// Ottosson's "toe": OKLab lightness is too compressed near black for offsets (a border one step
// above `#000` would vanish), so every lightness here is the toe-corrected Lr.
const K1 = 0.206, K2 = 0.03, K3 = (1 + K1) / (1 + K2);
function toe(l: number): number {
  return 0.5 * (K3 * l - K1 + Math.sqrt((K3 * l - K1) ** 2 + 4 * K2 * K3 * l));
}
function untoe(l: number): number {
  return (l * l + K1 * l) / (K3 * (l + K2));
}

function toLch(rgb: Rgb): Lch {
  const [l, a, b] = rgbToOklab(rgb);
  return { l: toe(l), c: Math.hypot(a, b), h: (Math.atan2(b, a) * 180) / Math.PI };
}

/** OKLCH (toe-corrected lightness) to sRGB, lowering chroma until the colour fits the gamut. */
function fromLch({ l, c, h }: Lch): Rgb {
  const lightness = untoe(Math.min(1, Math.max(0, l)));
  const radians = (h * Math.PI) / 180;
  let chroma = Math.max(0, c);
  for (let step = 0; step < 24; step += 1) {
    const linear = oklabToLinear([lightness, chroma * Math.cos(radians), chroma * Math.sin(radians)]);
    if (linear.every((value) => value >= -0.0005 && value <= 1.0005)) {
      return linear.map((value) => fromLinear(Math.min(1, Math.max(0, value)))) as Rgb;
    }
    chroma *= 0.85;
  }
  return oklabToLinear([lightness, 0, 0]).map((value) => fromLinear(Math.min(1, Math.max(0, value)))) as Rgb;
}

function luminance([red, green, blue]: Rgb): number {
  return 0.2126 * toLinear(red) + 0.7152 * toLinear(green) + 0.0722 * toLinear(blue);
}

/** WCAG 2 contrast ratio between two opaque colours. */
export function contrast(first: string, second: string): number {
  const a = parseHex(first), b = parseHex(second);
  if (!a || !b) return 1;
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

// ── Token recipes ─────────────────────────────────────

/**
 * - `surface`: keeps `ref`'s lightness offset from the background; hue and tint follow it.
 * - `text`: keeps `ref`'s absolute lightness (so a darker background never dims text).
 * - `accent`: sits between the background and the accent in lightness, in the accent's hue.
 * - `on-accent`: dark text drawn on the accent, swapped for near-white if it can't reach 4.5:1.
 */
type Recipe = "surface" | "text" | "accent" | "on-accent";

/** Every themeable token, with the hex it had before theming existed. */
export const TOKENS: Record<string, { recipe: Recipe; ref: string }> = {
  "--bg": { recipe: "surface", ref: "#111310" },
  "--wc-sidebar": { recipe: "surface", ref: "#0d0f0c" },
  "--wc-well": { recipe: "surface", ref: "#0c0e0b" },
  "--wc-panel": { recipe: "surface", ref: "#10120f" },
  "--wc-input": { recipe: "surface", ref: "#0e100d" },
  "--surface": { recipe: "surface", ref: "#161914" },
  "--wc-sidebar-hover": { recipe: "surface", ref: "#171a15" },
  "--wc-composer": { recipe: "surface", ref: "#181b16" },
  "--wc-elevated": { recipe: "surface", ref: "#1a1e17" },
  "--surface-raised": { recipe: "surface", ref: "#1b1f19" },
  "--wc-menu": { recipe: "surface", ref: "#20241c" },
  "--wc-code-bg": { recipe: "surface", ref: "#20251d" },
  "--wc-selected": { recipe: "surface", ref: "#21261d" },
  "--surface-hover": { recipe: "surface", ref: "#22271f" },
  "--wc-track": { recipe: "surface", ref: "#242a20" },
  "--wc-toggle": { recipe: "surface", ref: "#292d26" },
  "--border": { recipe: "surface", ref: "#2b3028" },
  "--wc-code-border": { recipe: "surface", ref: "#30382a" },
  "--border-bright": { recipe: "surface", ref: "#3a4135" },

  "--text": { recipe: "text", ref: "#e8ece4" },
  "--wc-text-body": { recipe: "text", ref: "#dbe0d7" },
  "--wc-code-heading": { recipe: "text", ref: "#d3dacd" },
  "--wc-code-text": { recipe: "text", ref: "#cad1c5" },
  "--text-soft": { recipe: "text", ref: "#adb5a7" },
  "--wc-knob": { recipe: "text", ref: "#7b8275" },
  "--text-dim": { recipe: "text", ref: "#757e70" },
  "--wc-placeholder": { recipe: "text", ref: "#6d7467" },
  "--text-faint": { recipe: "text", ref: "#5d6458" },

  "--wc-accent-hover": { recipe: "accent", ref: "#d0f664" },
  "--wc-accent-text": { recipe: "accent", ref: "#cce98a" },
  "--wc-accent-code": { recipe: "accent", ref: "#d2e4a3" },
  "--wc-accent-glint": { recipe: "accent", ref: "#f7ffcf" },
  "--wc-accent-wash": { recipe: "accent", ref: "#151a10" },
  "--wc-accent-tint": { recipe: "accent", ref: "#1b2114" },
  "--wc-accent-tint-strong": { recipe: "accent", ref: "#222b18" },
  "--wc-toggle-on": { recipe: "accent", ref: "#34451e" },
  "--wc-accent-ring": { recipe: "accent", ref: "#3c4b2d" },
  "--wc-accent-line-soft": { recipe: "accent", ref: "#43511f" },
  "--wc-accent-line-mid": { recipe: "accent", ref: "#506134" },
  "--wc-accent-line": { recipe: "accent", ref: "#58752d" },
  "--wc-focus-line": { recipe: "accent", ref: "#596649" },
  "--wc-input-focus": { recipe: "accent", ref: "#5f6d4e" },

  "--wc-on-accent": { recipe: "on-accent", ref: "#172000" }
};

/** Backgrounds are darkened until secondary (dim) text still reads on them at this ratio. */
export const MIN_DIM_TEXT_CONTRAST = 3.8;
/** The accent is also a text colour (links, labels), so it must read on the background. */
export const MIN_ACCENT_CONTRAST = 4.5;
const NEAR_WHITE = "#f7f8f5";

const DEFAULT_BG_LCH = toLch(parseHex(DEFAULT_BACKGROUND)!);
const DEFAULT_ACCENT_LCH = toLch(parseHex(DEFAULT_ACCENT)!);

function hueShift(ref: Lch, base: Lch): number {
  return ref.h - base.h;
}

function derive(recipe: Recipe, ref: Lch, background: Lch, accent: Lch): Lch {
  if (recipe === "surface" || recipe === "text") {
    // Tint beyond the background's own chroma fades out as the background approaches grey.
    const tint = Math.min(1, background.c / DEFAULT_BG_LCH.c);
    return {
      l: recipe === "text" ? ref.l : background.l + (ref.l - DEFAULT_BG_LCH.l),
      c: background.c + (ref.c - DEFAULT_BG_LCH.c) * tint,
      h: background.h + hueShift(ref, DEFAULT_BG_LCH)
    };
  }
  const chroma = DEFAULT_ACCENT_LCH.c > 0 ? ref.c / DEFAULT_ACCENT_LCH.c : 0;
  const hue = accent.h + hueShift(ref, DEFAULT_ACCENT_LCH);
  if (recipe === "on-accent") return { l: ref.l, c: accent.c * chroma, h: hue };
  const position = (ref.l - DEFAULT_BG_LCH.l) / (DEFAULT_ACCENT_LCH.l - DEFAULT_BG_LCH.l);
  return { l: background.l + (accent.l - background.l) * position, c: accent.c * chroma, h: hue };
}

/** The user's background, darkened if needed so the app's light text stays readable. */
export function clampBackground(hex: string): string {
  const rgb = parseHex(hex);
  if (!rgb) return DEFAULT_BACKGROUND;
  const lch = toLch(rgb);
  const dim = toLch(parseHex(TOKENS["--text-dim"].ref)!);
  let candidate = toHex(rgb);
  for (let lightness = lch.l; lightness > 0; lightness -= 0.005) {
    candidate = toHex(fromLch({ ...lch, l: lightness }));
    const derivedDim = toHex(fromLch(derive("text", dim, toLch(parseHex(candidate)!), DEFAULT_ACCENT_LCH)));
    // Dark theme only: the background must also be the darker of the two.
    if (luminance(parseHex(candidate)!) < luminance(parseHex(derivedDim)!) && contrast(derivedDim, candidate) >= MIN_DIM_TEXT_CONTRAST) break;
  }
  return lightnessUnchanged(candidate, rgb) ? toHex(rgb) : candidate;
}

/** The first loop step re-encodes the colour; treat a one-unit round-trip drift as unchanged. */
function lightnessUnchanged(candidate: string, original: Rgb): boolean {
  return parseHex(candidate)!.every((channel, index) => Math.abs(channel - original[index]) <= 1);
}

/** The user's accent, lightened if needed until it reads as text on `background`. */
export function readableAccent(hex: string, background: string): string {
  const rgb = parseHex(hex);
  if (!rgb) return DEFAULT_ACCENT;
  let candidate = toHex(rgb);
  const lch = toLch(rgb);
  for (let lightness = lch.l; contrast(candidate, background) < MIN_ACCENT_CONTRAST && lightness < 1; lightness += 0.01) {
    candidate = toHex(fromLch({ ...lch, l: lightness }));
  }
  return candidate;
}

/** What `resolveTheme` needs; a subset of `AppearanceConfig`. */
export interface ThemeInput {
  accent?: string | null;
  background?: string | null;
  backdrop?: BackdropMode;
  glassTint?: number;
  /** Drives `data-message-bubbles` on the root; the transcript styles from it. */
  messageBubbles?: boolean;
}

export interface ResolvedTheme {
  accent: string;
  background: string;
  /** The picked colour was changed to keep text readable. */
  accentAdjusted: boolean;
  backgroundAdjusted: boolean;
  variables: Record<string, string>;
}

export function resolveTheme(input: ThemeInput): ResolvedTheme {
  const pickedBackground = parseHex(input.background ?? "") ? toHex(parseHex(input.background!)!) : DEFAULT_BACKGROUND;
  const background = clampBackground(pickedBackground);
  const pickedAccent = parseHex(input.accent ?? "") ? toHex(parseHex(input.accent!)!) : DEFAULT_ACCENT;
  const accent = readableAccent(pickedAccent, background);
  const backgroundLch = toLch(parseHex(background)!);
  const accentLch = toLch(parseHex(accent)!);

  const variables: Record<string, string> = { "--wc-accent": accent };
  for (const [name, { recipe, ref }] of Object.entries(TOKENS)) {
    const refLch = toLch(parseHex(ref)!);
    variables[name] = toHex(fromLch(derive(recipe, refLch, backgroundLch, accentLch)));
  }
  // The background itself is exact, not a round trip through OKLCH.
  variables["--bg"] = background;
  // Mid-tone accents may not reach 4.5:1 with either; take whichever reads better.
  if (contrast(NEAR_WHITE, accent) > contrast(variables["--wc-on-accent"], accent)) variables["--wc-on-accent"] = NEAR_WHITE;
  Object.assign(variables, shellVariables(input.backdrop ?? "solid", variables, input.glassTint ?? DEFAULT_APPEARANCE.glassTint));

  return {
    accent,
    background,
    accentAdjusted: accent !== pickedAccent,
    backgroundAdjusted: background !== pickedBackground,
    variables
  };
}

/**
 * The shell (window background, sidebar, header, side panels) is what turns see-through over an
 * image or Liquid Glass. Content (bubbles, composer, cards, popovers, inputs) stays solid, so
 * chats stay legible whatever is behind them.
 */
function shellVariables(backdrop: BackdropMode, solid: Record<string, string>, glassTint: number): Record<string, string> {
  const alpha = (name: string, amount: number) => {
    const [red, green, blue] = parseHex(solid[name])!;
    return `rgb(${red} ${green} ${blue} / ${Math.round(Math.min(1, Math.max(0, amount)) * 100)}%)`;
  };
  if (backdrop === "image") {
    // The image is already dimmed by the backdrop layer; these only separate the panels.
    return {
      "--wc-shell": "transparent",
      "--wc-sidebar": alpha("--wc-sidebar", 0.5),
      "--wc-header": alpha("--bg", 0.3),
      "--wc-panel": alpha("--wc-panel", 0.72),
      "--wc-composer-fade": "transparent"
    };
  }
  if (backdrop === "glass") {
    const tint = glassTint / 100;
    return {
      "--wc-shell": alpha("--bg", tint),
      "--wc-sidebar": alpha("--wc-sidebar", tint + 0.15),
      "--wc-header": "transparent",
      "--wc-panel": alpha("--wc-panel", tint + 0.2),
      "--wc-composer-fade": "transparent"
    };
  }
  return {
    "--wc-shell": solid["--bg"],
    "--wc-header": alpha("--bg", 0.92),
    "--wc-composer-fade": solid["--bg"]
  };
}

/** Writes the theme onto `root` (the document element) as inline custom properties. */
export function applyTheme(input: ThemeInput, root: HTMLElement = document.documentElement): ResolvedTheme {
  const theme = resolveTheme(input);
  for (const [name, value] of Object.entries(theme.variables)) root.style.setProperty(name, value);
  root.dataset.backdrop = input.backdrop ?? "solid";
  // CSS-only switch, like the backdrop: the transcript styles from it without a re-render.
  root.dataset.messageBubbles = input.messageBubbles === true ? "on" : "off";
  return theme;
}

const THEME_CACHE_KEY = "wackcode:theme";

/**
 * Remembers the last applied theme so the next launch paints it before bootstrap answers. A
 * per-device hint only: `wackcode.json` stays the source of truth and overwrites it.
 */
export function cacheTheme(input: ThemeInput): void {
  try {
    const cached: ThemeInput = { accent: input.accent ?? null, background: input.background ?? null, backdrop: input.backdrop, glassTint: input.glassTint };
    localStorage.setItem(THEME_CACHE_KEY, JSON.stringify(cached));
  } catch {
    // Storage can be unavailable; the first frame then shows the default theme.
  }
}

/** Applies the cached theme, if any; called once before the first render. */
export function applyCachedTheme(): void {
  try {
    const cached = JSON.parse(localStorage.getItem(THEME_CACHE_KEY) ?? "null") as ThemeInput | null;
    if (cached) applyTheme(cached);
  } catch {
    // A missing or corrupt cache just means the defaults in styles.css paint first.
  }
}
