import { motion, useReducedMotion } from "motion/react";
import { AREAS, AREA_LABELS, type Area } from "../areas";
import { Icon, type IconName } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

const AREA_ICONS: Record<Area, IconName> = { code: "code", chat: "comment" };
const AREA_KEYS: Record<Area, string> = { code: "⌥⌘1", chat: "⌥⌘2" };

interface AreaSwitchProps {
  area: Area;
  /** The other area has a chat waiting on an answer; its card only shows once that chat is open. */
  attention?: Area;
  onSwitch: (area: Area) => void;
}

/**
 * The Code | Chat switch at the top of the sidebar. The pill is one element that glides between
 * the two segments (a shared `layoutId`), so the switch reads as moving rather than repainting.
 * A pair of pressed-state buttons, not a tablist: the chat tab bar is the window's tablist.
 */
export function AreaSwitch({ area, attention, onSwitch }: AreaSwitchProps) {
  const reduce = useReducedMotion() ?? false;
  return (
    <div className="area-switch" role="group" aria-label="Area">
      {AREAS.map((item) => {
        const waiting = attention === item;
        return (
          <Tooltip key={item} side="bottom" label={<>{AREA_LABELS[item]} <kbd>{AREA_KEYS[item]}</kbd></>}>
            <button
              type="button"
              aria-pressed={area === item}
              aria-label={waiting ? `${AREA_LABELS[item]}, a chat is waiting for your answer` : AREA_LABELS[item]}
              className={`area-tab${area === item ? " active" : ""}`}
              onClick={() => onSwitch(item)}
            >
              {area === item && (
                <motion.span layoutId="area-switch-pill" className="area-tab-pill" transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 480, damping: 34 }} />
              )}
              <span className="area-tab-label"><Icon name={AREA_ICONS[item]} />{AREA_LABELS[item]}</span>
              {waiting && <span className="area-tab-dot" aria-hidden="true" />}
            </button>
          </Tooltip>
        );
      })}
    </div>
  );
}
