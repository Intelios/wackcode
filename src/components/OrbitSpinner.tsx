import { useEffect, useRef, useState } from "react";
import { useReducedMotion } from "motion/react";

interface OrbitSpinnerProps {
  /** True while the tool is still running. */
  active: boolean;
  /** True when the run it belonged to failed; the landing beat turns danger-coloured. */
  failed?: boolean;
  className?: string;
}

/** The beads orbit an invisible 4.25px circle at 0°, -38° and -76°, each behind the last. */
const BEADS = [
  { cx: 10.25, cy: 6, r: 1.6, opacity: 1 },
  { cx: 9.35, cy: 3.38, r: 1.2, opacity: 0.6 },
  { cx: 7.03, cy: 1.88, r: 0.85, opacity: 0.32 },
] as const;

/** How long the CSS landing beat (`orbit-land` / `orbit-ping`) runs before the mark unmounts. */
const LAND_MS = 320;

/**
 * The tool row's working mark: three accent beads chasing each other round an orbit, decaying
 * like a comet broken into beads — a small echo of the composer's send comet. When `active`
 * drops the beads collapse to the centre while a ring pings outward, so the moment the tool
 * lands is visible; then the mark unmounts. A mark that first appears already finished never
 * pops (nothing landed while you were watching), and reduced motion skips the beat entirely.
 * While `active` it names itself "Running" (the transcript's live label); the landing beat is
 * decorative. Motion lives in styles.css, where the reduced-motion block leaves still beads.
 */
export function OrbitSpinner({ active, failed = false, className = "" }: OrbitSpinnerProps) {
  const reduced = useReducedMotion() ?? false;
  const [phase, setPhase] = useState<"live" | "landing" | "off">(active ? "live" : "off");
  const timer = useRef<number>(undefined);

  useEffect(() => {
    if (active) {
      window.clearTimeout(timer.current);
      setPhase("live");
    } else {
      // Only a mark that was live lands; one that mounted finished stays hidden.
      setPhase((was) => (was === "live" && !reduced ? "landing" : "off"));
    }
  }, [active, reduced]);

  useEffect(() => {
    if (phase !== "landing") return;
    timer.current = window.setTimeout(() => setPhase("off"), LAND_MS);
    return () => window.clearTimeout(timer.current);
  }, [phase]);

  if (phase === "off") return null;
  const live = phase === "live";
  return (
    <svg
      viewBox="0 0 12 12"
      className={`orbit-spinner ${live ? "live" : "landing"} ${failed ? "failed" : ""} ${className}`}
      role={live ? "img" : undefined}
      aria-label={live ? "Running" : undefined}
      aria-hidden={live ? undefined : true}
    >
      <g className="orbit-beads">
        <circle className="orbit-halo" cx={BEADS[0].cx} cy={BEADS[0].cy} r="2.6" />
        {BEADS.map((bead) => (
          <circle key={`${bead.cx},${bead.cy}`} className="orbit-bead" cx={bead.cx} cy={bead.cy} r={bead.r} opacity={bead.opacity} />
        ))}
      </g>
      <circle className="orbit-pop" cx="6" cy="6" r="4.25" />
    </svg>
  );
}
