import { useId } from "react";

interface QuillMarkProps {
  live: boolean;
  className?: string;
}

const PAGE = "M14 77H43L65 99H36Z";
const FEATHER = "M18 86C22 75 26 65 31 57C25 48 31 33 41 23L39 36C45 21 60 10 86 3C77 14 72 27 69 34L59 40 68 37C65 48 59 56 53 60L44 62 52 63C48 69 40 73 33 74L18 86ZM23 78C32 59 49 32 69 15C48 39 36 61 23 78Z";
const INK = [
  "M23 82q2-1 3 0t3 0 3 0 3 0 3 0 4 0",
  "M29 88q2-1 3 0t3 0 3 0 3 0 3 0 4 0",
  "M35 94q2-1 3 0t3 0 3 0 3 0 3 0 4 0",
];

function Feather() {
  return <path d={FEATHER} fill="currentColor" fillRule="evenodd" stroke="none" />;
}

function Paper({ clipId }: { clipId?: string }) {
  return (
    <>
      <path className="quill-page" d={PAGE} fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinejoin="round" opacity=".45" />
      <g clipPath={clipId ? `url(#${clipId})` : undefined} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        {INK.map((d, index) => <path key={d} className={`quill-ink quill-ink-${index + 1}`} pathLength="1" d={d} />)}
      </g>
    </>
  );
}

/**
 * Reference-led quill silhouette: tapered shaft, asymmetric feather and cut barbs. The nib
 * writes three lines across and down a stationary sheet, lifting between lines and returning
 * to the first line before the loop repeats. Ink reveals in sync with the nib, stays on the
 * page, and fades only during the final lifted return. Start and end frames match.
 * Each instance owns its clip id (multiple transcript rows must not share SVG fragment ids).
 * Decorative: the adjacent verb conveys status. Reduced motion leaves a still written page.
 * Geometry and timing match the user-approved preview, on its original 100-unit grid.
 */
export function QuillMark({ live, className = "" }: QuillMarkProps) {
  const clipId = `quill-page-${useId().replace(/:/g, "")}`;
  return (
    <svg viewBox="0 0 100 100" aria-hidden="true" className={`quill-mark ${live ? "live" : ""} ${className}`}>
      <defs><clipPath id={clipId}><path d={PAGE} /></clipPath></defs>
      <Paper clipId={clipId} />
      <g className="quill-body"><Feather /></g>
    </svg>
  );
}

/** Icons.tsx's still mark uses the same geometry, scaled onto its 24-unit grid. */
export const quillIconPaths = <g transform="scale(.24)"><Paper /><Feather /></g>;
