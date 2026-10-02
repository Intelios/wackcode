import { DuckMark } from "./DuckMark";

interface PonderingDuckProps {
  /** True while the model is still producing this reasoning block. */
  live: boolean;
  className?: string;
}

/**
 * The reasoning row's mark: WackCode's duck doing rubber-duck debugging. While `live`, the duck
 * rocks its head as if puzzling and three thought bubbles trail up-right from its head and pop;
 * at rest the duck sits upright and the trail stays faint, so the icon never swaps and still
 * reads as thinking when the block is done.
 * Decorative: the text beside it says "Thinking…"/"Thought for …", so it stays `aria-hidden`.
 * Motion lives in styles.css, where the reduced-motion block shows a still duck-with-bubbles.
 */
export function PonderingDuck({ live, className = "" }: PonderingDuckProps) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className={`ponder-duck ${live ? "live" : ""} ${className}`}>
      <g className="ponder-body">
        <DuckMark x="0" y="7" width="17" height="17" />
      </g>
      <g className="ponder-bubbles">
        <circle className="ponder-bubble" cx="16.5" cy="8.5" r="1.1" />
        <circle className="ponder-bubble" cx="19" cy="5.5" r="1.5" />
        <circle className="ponder-bubble" cx="21.5" cy="2" r="2" />
      </g>
    </svg>
  );
}
