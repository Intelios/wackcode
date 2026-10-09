import { useRef, useState } from "react";
import { motion, useReducedMotion, type Transition } from "motion/react";
import { areaDirection, type Area } from "../areas";
import { DuckMark } from "./DuckMark";

/** Hop endpoints sit this far past the window edge, so the silhouette fully clears it. */
const OFFSCREEN_PX = 72;

/** About one switch in ten trades the hop for a somersault. */
const SOMERSAULT_CHANCE = 0.1;

/** The lights' beat as a share of the hop: quick to dim, hold across the stage, come back
    up on the arriving area while the duck hops off. */
const DIM_TIMES = [0, 0.14, 0.72, 1];

interface Hop {
  /** Bumped per switch; the overlay is keyed by it, so a rapid re-switch replays the duck
      mid-flight instead of stacking a second one. */
  key: number;
  direction: 1 | -1;
  special: boolean;
}

/**
 * The Code ↔ Chat switch's little show (docs/design.md › Motion): on every real change of
 * area — the switch, ⌥⌘1/⌥⌘2, the Go menu, or a cross-area open — the window dims for a beat
 * and a spotlit duck hops across the stage in the direction of travel; the lights come back
 * up on the arriving area as the duck hops off, so the swap reads as a reveal rather than a
 * repaint. About one switch in ten it somersaults instead, kin to the compaction stage's rare
 * cameo. The area's own swap (pill glide, sidebar and hero slides) plays out under the dim.
 *
 * Pure decoration: an `aria-hidden`, click-through overlay whose duck ends off-screen and
 * invisible, so nothing ever unmounts it. Under reduced motion it renders nothing at all; the
 * switch's fades stay the whole effect there.
 */
export function AreaHop({ area }: { area: Area }) {
  const reduce = useReducedMotion() ?? false;
  const shownArea = useRef(area);
  const hopCount = useRef(0);
  const [hop, setHop] = useState<Hop>();
  // Set-state-during-render, the documented "previous props" pattern (Sidebar's swap ref does
  // the same): the hop must spawn inside the very commit that swaps the areas on screen.
  if (shownArea.current !== area) {
    const direction = areaDirection(shownArea.current, area);
    shownArea.current = area;
    if (!reduce) setHop({ key: ++hopCount.current, direction, special: Math.random() < SOMERSAULT_CHANCE });
  }
  if (!hop) return null;

  const { key, direction, special } = hop;
  const width = window.innerWidth || 1024;
  const from = direction === 1 ? -OFFSCREEN_PX : width + OFFSCREEN_PX;
  const to = direction === 1 ? width + OFFSCREEN_PX : -OFFSCREEN_PX;
  const at = (t: number) => Math.round(from + (to - from) * t);
  const duration = special ? 1.05 : 0.8;
  // The standard hop bounces in two arcs with a wobble; the rare somersault takes one high,
  // spinning flight. Transforms and opacity only, and it exits as it fades off the far edge.
  const animate = special
    ? {
        x: [from, at(0.5), to],
        y: [0, -120, 0],
        rotate: [0, 180 * direction, 360 * direction],
        scale: [1, 1.12, 1],
        opacity: [1, 1, 0]
      }
    : {
        x: [from, at(0.25), at(0.5), at(0.75), to],
        y: [0, -36, 0, -22, 0],
        rotate: [0, -9 * direction, 7 * direction, -5 * direction, 0],
        opacity: [1, 1, 1, 1, 0]
      };
  // Gravity's easing: ease out of every launch, ease into every landing.
  const transition: Transition = special
    ? { duration, times: [0, 0.5, 1], ease: ["easeOut", "easeIn"] }
    : { duration, times: [0, 0.25, 0.5, 0.75, 1], ease: ["easeOut", "easeIn", "easeOut", "easeIn"] };
  const dim: Transition = { duration, times: DIM_TIMES, ease: "easeInOut" };

  return (
    <div key={key} className={`area-hop${special ? " special" : ""}`} data-direction={direction === 1 ? "right" : "left"} aria-hidden="true">
      <motion.div className="area-hop-scrim" initial={{ opacity: 0 }} animate={{ opacity: [0, 1, 1, 0] }} transition={dim} />
      <motion.div className="area-hop-duck" initial={{ x: from, y: 0, rotate: 0, scale: 1, opacity: 1 }} animate={animate} transition={transition}>
        {/* The spotlight rides inside the animated element, so it travels with the duck. */}
        <span className="area-hop-glow" />
        {/* DUCK_PATH faces right; heading back to Code wears the mirror on a wrapper, whose
            transform motion does not own. */}
        <span className="area-hop-facing"><DuckMark /></span>
      </motion.div>
    </div>
  );
}
