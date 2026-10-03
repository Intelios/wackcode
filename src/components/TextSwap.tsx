import { useRef } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface TextSwapProps {
  text: string;
  /** Bumping this animates the swap; a `text`-only change updates in place (the span's key is
   *  unchanged, so React swaps the text node). Key the whole TextSwap by owner so switching
   *  owners remounts with `initial={false}` — the invariant is that an animated swap may only
   *  fire for a change the mounted component observed live, never for navigation between views. */
  swapKey: number | string;
  variant: "title" | "row" | "rise";
  className?: string;
  /** Marks only spans that entered through a real swap — remounts stay unmarked, so the
   *  title's glint never replays when a chat is revisited. */
  swappedClassName?: string;
}

/** One text line that swaps by key. `title` slides the old text out left while the new text
 *  enters from the right (the auto-title reveal); `row` is a plain crossfade (the sidebar echo);
 *  `rise` slides the old text up and out while the new text rises in from below (the new-chat
 *  hero's project name, echoing the project picker's trigger). */
export function TextSwap({ text, swapKey, variant, className, swappedClassName }: TextSwapProps) {
  const reduce = useReducedMotion() ?? false;
  const mountKey = useRef(swapKey);
  const swapped = swapKey !== mountKey.current;
  const horizontal = variant === "title";
  const vertical = variant === "rise";
  const distance = 16;
  const rise = 12;
  return (
    <span className={`text-swap ${vertical ? "rise" : ""} ${className ?? ""}`}>
      <AnimatePresence initial={false} mode="popLayout">
        <motion.span
          key={swapKey}
          className={`text-swap-text ${swapped && !reduce ? (swappedClassName ?? "") : ""}`}
          initial={reduce ? { opacity: 0 } : horizontal ? { opacity: 0, x: distance } : vertical ? { opacity: 0, y: rise } : { opacity: 0 }}
          animate={{ opacity: 1, x: 0, y: 0 }}
          exit={reduce
            ? { opacity: 0 }
            : horizontal
              ? { opacity: 0, x: -distance, transition: { duration: 0.18, ease: EASE } }
              : vertical
                ? { opacity: 0, y: -rise, transition: { duration: 0.18, ease: EASE } }
                : { opacity: 0 }}
          transition={(horizontal || vertical) && !reduce
            ? { type: "spring", stiffness: 420, damping: 32 }
            : { duration: 0.28, ease: EASE }}
        >{text}</motion.span>
      </AnimatePresence>
    </span>
  );
}
