import { DUCK_PATH } from "../gravity-well";

/**
 * WackCode's rubber duck (quack, wack), the same silhouette as the app icon (`src-tauri/icons/icon.svg`).
 * The shape lives in `gravity-well.ts` as `DUCK_PATH` so the compaction stage's duck cameo can fill the
 * real silhouette as a `Path2D`. One continuous shape with the eye as a real hole, so it takes any
 * `currentColor`: on the accent tile it follows the user's theme. Decorative: text beside it names the app.
 */
export function DuckMark({ className = "", ...place }: { className?: string } & Pick<React.SVGProps<SVGSVGElement>, "x" | "y" | "width" | "height">) {
  return (
    // `place` positions it when nested inside another SVG, such as a Settings hero's stage.
    <svg viewBox="0 0 256 256" fill="currentColor" aria-hidden="true" className={`duck-mark ${className}`} {...place}>
      <path fillRule="evenodd" d={DUCK_PATH} />
    </svg>
  );
}
