import { agentName } from "./agentName";
import type { ResolvedTheme } from "./theme";
import type { AppearanceConfig, ComputerCursorAppearance } from "./types";

/** Use the solid content surface tokens even when the app's shell uses Glass or an image. */
export function computerCursorAppearance(appearance: AppearanceConfig, theme: ResolvedTheme): ComputerCursorAppearance {
  return {
    agentName: agentName(appearance),
    accent: theme.accent,
    outline: theme.variables["--wc-on-accent"],
    surface: theme.variables["--wc-elevated"],
    text: theme.variables["--text"],
    mutedText: theme.variables["--text-soft"]
  };
}
