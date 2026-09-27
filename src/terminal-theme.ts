/**
 * Builds the terminal's colour theme from the app's resolved theme variables, so the shell
 * follows the user's accent, background and presets like every other surface (`theme.ts`). The
 * eight `--wc-term-*` tokens carry the ANSI base colours (same `syntax` recipe as code
 * highlighting, so they lean towards the accent too); bright variants are derived here by
 * lifting each base towards the theme's bright text colour, the way a real terminal's
 * "bright" range reads.
 */
import type { ITheme } from "@xterm/xterm";
import { parseHex, toHex } from "./theme";

const ANSI_NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;
type AnsiName = (typeof ANSI_NAMES)[number];

/** A themeable colour's resolved value, or `fallback` when the variable is unset or unparseable. */
function readColor(variables: Record<string, string>, name: string, fallback: string): string {
  const value = variables[name]?.trim() ?? "";
  if (parseHex(value)) return toHex(parseHex(value)!);
  // Some shell variables resolve to `rgb(r g b / a)` rather than a hex.
  const rgb = value.match(/^rgba?\(\s*(\d+)\s+(\d+)\s+(\d+)/);
  return rgb ? toHex([Number(rgb[1]), Number(rgb[2]), Number(rgb[3])]) : fallback;
}

/** `colour` moved `amount` of the way towards `target` — the bright-variant lift. */
function lift(colour: string, target: string, amount: number): string {
  const from = parseHex(colour)!;
  const to = parseHex(target)!;
  return toHex(from.map((channel, index) => Math.round(channel + (to[index] - channel) * amount)) as typeof from);
}

/** `colour` with an alpha channel, as the #rrggbbaa xterm accepts. */
function translucent(colour: string, alpha: number): string {
  return colour + Math.round(Math.min(1, Math.max(0, alpha)) * 255).toString(16).padStart(2, "0");
}

/** The xterm `ITheme` for a resolved theme (`resolveTheme(appearance).variables`). */
export function terminalTheme(variables: Record<string, string>): ITheme {
  const color = (name: string, fallback: string) => readColor(variables, name, fallback);
  const background = color("--wc-well", "#0c0e0b");
  const foreground = color("--wc-text-body", "#dbe0d7");
  const brightTarget = color("--wc-term-white", "#dbe0d7");
  const accent = color("--wc-accent", "#c2ee4a");
  const base = Object.fromEntries(ANSI_NAMES.map((name) => [name, color(`--wc-term-${name}`, "#808080")])) as Record<AnsiName, string>;
  const bright = Object.fromEntries(
    ANSI_NAMES.map((name) => [`bright${(name[0].toUpperCase() + name.slice(1)) as Capitalize<AnsiName>}`, lift(base[name], brightTarget, 0.34)])
  ) as Record<`bright${Capitalize<AnsiName>}`, string>;
  // "Bright black" is a dim grey, not a washed-out background tone, and "bright white" lifts
  // toward pure white rather than itself (a no-op) — real terminals read it as brighter.
  bright.brightBlack = lift(base.black, color("--text-soft", "#adb5a7"), 0.62);
  bright.brightWhite = lift(base.white, "#ffffff", 0.4);
  return {
    background,
    foreground,
    cursor: accent,
    cursorAccent: color("--wc-on-accent", "#172000"),
    selectionBackground: translucent(accent, 0.26),
    selectionInactiveBackground: translucent(accent, 0.14),
    ...base,
    ...bright
  };
}
