import { useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

interface RollingNumberProps {
  value: number;
  prefix?: string;
  className?: string;
}

const ROLL = { type: "spring", stiffness: 520, damping: 40 } as const;
const DIGIT = {
  enter: (direction: number) => ({ y: `${direction * 105}%` }),
  settled: { y: 0 },
  exit: (direction: number) => ({ y: `${direction * -105}%` }),
};

/**
 * One drum per decimal place: increases roll up, decreases down, signs stay still. Changes
 * jump directly to their target, never queue every intermediate value. Columns key from the
 * units side so carries keep the same drum. Presence custom carries the current direction
 * to outgoing digits (their own props are frozen after removal). Mounts never roll.
 * `layoutDependency` scopes the columns' position animation to carries: without it every
 * rerender re-measures, and the transcript's follow-scroll shifts the measured box so the
 * digits slide as each token lands.
 * The signed value is the accessible label; all transient visual digits are hidden from AT.
 * Reduced motion bypasses the animation tree entirely.
 */
export function RollingNumber({ value, prefix = "", className = "" }: RollingNumberProps) {
  const reduced = useReducedMotion() ?? false;
  const [previous, setPrevious] = useState({ value, direction: 1 });
  const direction = value === previous.value ? previous.direction : value > previous.value ? 1 : -1;
  // Guarded render-time adjustment keeps direction stable across unrelated parent rerenders.
  if (previous.value !== value) setPrevious({ value, direction });

  if (reduced) return <span className={className}>{prefix}{value}</span>;

  const digits = String(value).split("");
  return (
    <span className={`rolling-number ${className}`} role="img" aria-label={`${prefix}${value}`}>
      {prefix && <span aria-hidden="true">{prefix}</span>}
      <AnimatePresence initial={false} mode="popLayout" custom={direction}>
        {digits.map((digit, index) => (
          <motion.span
            key={digits.length - index - 1}
            className="rolling-digit"
            aria-hidden="true"
            layout="position"
            layoutDependency={digits.length}
            custom={direction}
            variants={DIGIT}
            initial="enter"
            animate="settled"
            exit="exit"
            transition={ROLL}
          >
            <AnimatePresence initial={false} mode="popLayout" custom={direction}>
              <motion.span key={digit} custom={direction} variants={DIGIT} initial="enter" animate="settled" exit="exit" transition={ROLL}>
                {digit}
              </motion.span>
            </AnimatePresence>
          </motion.span>
        ))}
      </AnimatePresence>
    </span>
  );
}
