/**
 * The colours a game draws with, read from the live theme so every accent, preset and backdrop
 * works (`theme.ts` owns them; a canvas can't use `var()`). Danger keeps its fixed meaning: it is
 * what hurts. Callers poll this about once a second, as `CompactingStage` does, rather than
 * observing theme mutations.
 */
export interface GamePalette {
  accent: string;
  onAccent: string;
  text: string;
  textSoft: string;
  textDim: string;
  well: string;
  surface: string;
  border: string;
  danger: string;
}

export const FALLBACK_PALETTE: GamePalette = {
  accent: "#c2ee4a", onAccent: "#172000", text: "#e8ece4", textSoft: "#adb5a7", textDim: "#757e70",
  well: "#0c0e0b", surface: "#161914", border: "#2b3028", danger: "#f08080"
};

const VARIABLES: Record<keyof GamePalette, string> = {
  accent: "--wc-accent", onAccent: "--wc-on-accent", text: "--text", textSoft: "--text-soft", textDim: "--text-dim",
  well: "--wc-well", surface: "--surface", border: "--border", danger: "--danger"
};

export function readPalette(previous: GamePalette = FALLBACK_PALETTE): GamePalette {
  if (typeof document === "undefined") return previous;
  const css = getComputedStyle(document.documentElement);
  const next = { ...previous };
  for (const key of Object.keys(VARIABLES) as (keyof GamePalette)[]) {
    const value = css.getPropertyValue(VARIABLES[key]).trim();
    if (value) next[key] = value;
  }
  return next;
}
