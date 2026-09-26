import { useEffect, useRef, useState, type ReactNode } from "react";

/** A sub-agent as its chip and panel show it (a child left running when its call ended was stopped). */
export type RobotStatus = "running" | "queued" | "done" | "failed" | "stopped";

// Each face on the same 24px grid as Icons.tsx. Running eyes are filled dots so they can scan
// and blink; the others are strokes: closed (queued, stopped), happy (done), crossed (failed).
const EYES: Record<RobotStatus, ReactNode> = {
  running: <><circle cx="9.4" cy="12.3" r="1.35" /><circle cx="14.6" cy="12.3" r="1.35" /></>,
  queued: <path d="M8.4 12.5h2.2M13.4 12.5h2.2" />,
  done: <path d="M8.3 13l1.1-1.2 1.1 1.2M13.5 13l1.1-1.2 1.1 1.2" />,
  failed: <path d="M8.5 11.4l1.8 1.8M10.3 11.4l-1.8 1.8M13.7 11.4l1.8 1.8M15.5 11.4l-1.8 1.8" />,
  stopped: <path d="M8.4 12.5h2.2M13.4 12.5h2.2" />
};

const MOUTHS: Record<RobotStatus, string> = {
  running: "M10.7 15.7h2.6",
  queued: "M11.1 15.9h1.8",
  done: "M10.2 15.3c.9.9 2.7.9 3.6 0",
  failed: "M10.2 16.2c.9-.9 2.7-.9 3.6 0",
  stopped: "M10.7 15.8h2.6"
};

interface RobotMarkProps {
  status: RobotStatus;
  /** Open its eyes on mount: the panel's robot "boots" when it slides in. */
  boot?: boolean;
  className?: string;
}

/**
 * The sub-agent's robot. Its face follows the child: it scans and blinks with a pulsing antenna
 * while it works, dozes while queued, grins when it is done (with a one-shot pop when that
 * happens live), and shows crossed eyes when it failed. Decorative: the text beside it says the
 * same, so it stays hidden from assistive technology. Motion lives in styles.css, where the
 * reduced-motion block stills it.
 */
export function RobotMark({ status, boot = false, className = "" }: RobotMarkProps) {
  const previous = useRef(status);
  const [popping, setPopping] = useState(false);

  useEffect(() => {
    const was = previous.current;
    previous.current = status;
    if (was !== "running" || status === "running") return;
    setPopping(true);
    const timer = setTimeout(() => setPopping(false), 600);
    return () => clearTimeout(timer);
  }, [status]);

  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={`robot-mark ${status} ${boot ? "booting" : ""} ${popping ? "popping" : ""} ${className}`}
    >
      <g className="robot-body">
        <path d="M12 7.6V5.4" />
        <circle className="robot-light" cx="12" cy="3.7" r="1.55" />
        <rect x="4.6" y="7.6" width="14.8" height="11" rx="3.6" />
        <path d="M2.8 11.6v3M21.2 11.6v3" />
        <g className="robot-gaze">
          <g className={`robot-eyes ${status === "running" ? "filled" : ""}`}>{EYES[status]}</g>
        </g>
        <path className="robot-mouth" d={MOUTHS[status]} />
      </g>
    </svg>
  );
}
