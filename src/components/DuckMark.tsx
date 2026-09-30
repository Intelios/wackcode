/**
 * WackCode's rubber duck (quack, wack), the same silhouette as the app icon (`src-tauri/icons/icon.svg`).
 * One continuous shape with the eye as a real hole, so it takes any `currentColor`: on the accent tile it
 * follows the user's theme. Decorative: text beside it names the app.
 */
export function DuckMark({ className = "", ...place }: { className?: string } & Pick<React.SVGProps<SVGSVGElement>, "x" | "y" | "width" | "height">) {
  return (
    // `place` positions it when nested inside another SVG, such as a Settings hero's stage.
    <svg viewBox="0 0 256 256" fill="currentColor" aria-hidden="true" className={`duck-mark ${className}`} {...place}>
      <path
        fillRule="evenodd"
        d="M34 102C50 120 66 134 88 136C108 137 107.91 128.49 99.91 100.49A56 56 0 0 1 205.92 65.02C218 70 236 72 245 80C253 88 250 101 238 103C228 106 217 110 209 112C201 114 205 132 217 142C231 154 238 172 234 188C228 216 190 228 128 228C70 228 32 216 24 188C18 166 16 126 22 106C24 99 30 98 34 102ZM159 72A11 11 0 1 0 181 72A11 11 0 1 0 159 72Z"
      />
    </svg>
  );
}
