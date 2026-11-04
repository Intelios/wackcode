/**
 * Theme ⇄ image links (Settings › Appearance). A saved background image can be linked to one
 * theme preset: showing the image applies that preset's colours, and picking the preset shows
 * the linked image. Both directions resolve through pure helpers here so App and the settings
 * page agree.
 */

import { presetById, presetForColours, storedAccent, storedBackground } from "./theme";
import type { AppearanceConfig, BackgroundImageEntry } from "./types";

/** The library entry currently shown: `backgroundImage` must name one, or nothing is shown. */
export function shownEntry(config: AppearanceConfig): BackgroundImageEntry | undefined {
  return config.backgroundImages.find((image) => image.name === config.backgroundImage);
}

/** The image linked to a theme preset, if any. One image per preset is enforced when linking. */
export function imageLinkedTo(config: AppearanceConfig, presetId: string): BackgroundImageEntry | undefined {
  return config.backgroundImages.find((image) => image.theme === presetId);
}

/** Save a theme link onto one image, moving the preset off any other image that held it. */
export function linkImageToTheme(config: AppearanceConfig, name: string, presetId: string | null): AppearanceConfig {
  return {
    ...config,
    backgroundImages: config.backgroundImages.map((image) => ({
      ...image,
      theme: image.name === name ? presetId : presetId && image.theme === presetId ? null : image.theme
    }))
  };
}

/**
 * The config with the shown image's linked theme applied, or null when there is nothing to
 * apply: no shown image, no link, an id no preset has, or the colours already are that
 * preset. Accent and background save exactly as picking the preset tile would.
 */
export function linkedThemeConfig(config: AppearanceConfig): AppearanceConfig | null {
  const preset = presetById(shownEntry(config)?.theme);
  if (!preset || presetForColours(config.accent, config.background)?.id === preset.id) return null;
  return { ...config, accent: storedAccent(preset.accent), background: storedBackground(preset.background) };
}
