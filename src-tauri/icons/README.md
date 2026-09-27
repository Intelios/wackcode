# App icon

The rubber duck (quack, wack) in the app's ink `#111310` on its lime accent `#c2ee4a` (`DEFAULT_BACKGROUND` and
`DEFAULT_ACCENT` in `src/theme.ts`). The same silhouette is `src/components/DuckMark.tsx`.

- `icon.svg`: the master, on Apple's macOS grid (824 px tile in a 1024 px canvas, with its drop shadow).
- `icon-small.svg`: the 16 and 32 px cut: bigger duck and eye, flat tile, no shadow.

To regenerate after changing either SVG:

1. Render `icon.svg` to a 1024 px PNG, then run `pnpm tauri icon <that png>`. This rewrites every PNG, `icon.ico`
   and `icon.icns` here.
2. `pnpm tauri icon` shrinks the master for the small sizes, so rebuild `icon.icns` with `iconutil -c icns` from an
   `.iconset` whose `icon_16x16`, `icon_16x16@2x` and `icon_32x32` come from `icon-small.svg` (rendered at their
   pixel sizes) and whose other entries come from `icon.svg`. Copy the 32 px small cut over `32x32.png` too.
