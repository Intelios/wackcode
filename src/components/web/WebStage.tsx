import { DuckMark } from "../DuckMark";

/**
 * Settings › Web's stage: the duck on its tile, a magnifying glass sweeping over a stack of
 * drifting pages. Each page glows as the lens passes it, and a result lands in the stack below.
 * Asleep while search is off: the pages fade, the lens parks and a "z" floats up. Pure
 * decoration; the pill says the same in words. The loops live in styles.css, which parks the
 * lens over the middle page with every result laid out under reduced motion.
 */
export function WebStage({ live }: { live: boolean }) {
  const pages = [0, 1, 2];
  return (
    <svg className={`settings-stage web-stage ${live ? "live" : "asleep"}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="web-stage-tile" x="8" y="36" width="40" height="40" rx="11" />
      <DuckMark x="17" y="45" width="22" height="22" />
      <text className="web-stage-z" x="44" y="34">z</text>
      {pages.map((page) => {
        const x = 62 + page * 30;
        return (
          <g key={page} className={`web-stage-page ${page === 1 ? "middle" : ""}`} style={{ "--k": page } as React.CSSProperties}>
            <rect x={x} y="14" width="24" height="32" rx="4" />
            <path d={`M${x + 5} 22h14M${x + 5} 28h10M${x + 5} 34h12`} />
          </g>
        );
      })}
      <g className="web-stage-lens">
        <circle cx="74" cy="30" r="11" />
        <path d="M82 38l8 8" />
      </g>
      {pages.map((hit) => (
        <g key={hit} className="web-stage-hit" style={{ "--k": hit } as React.CSSProperties}>
          <circle cx="66" cy={62 + hit * 14} r="3.4" />
          <rect x="74" y={59 + hit * 14} width={[62, 48, 56][hit]} height="6" rx="3" />
        </g>
      ))}
    </svg>
  );
}
