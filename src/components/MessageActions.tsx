import type { MessageVersions } from "../types";
import { Icon, type IconName } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

export interface MessageActionItem {
  id: string;
  label: string;
  icon: IconName;
  onClick: () => void;
}

interface Props {
  items: MessageActionItem[];
  versions?: MessageVersions;
  /** Moving between versions is refused while the chat is busy. */
  switchDisabled?: boolean;
  onSwitch?: (entryId: string) => void;
  /** Aligns the row under a user bubble (end) or an answer (start). */
  align: "start" | "end";
}

/** The hover row under a message: its actions, and the ‹ n/m › switcher between its versions. */
export function MessageActions({ items, versions, switchDisabled, onSwitch, align }: Props) {
  if (items.length === 0 && !versions) return null;
  return (
    <div className={`message-actions ${align}`}>
      {versions && (
        <div className="version-switcher" role="group" aria-label="Message versions">
          <button
            type="button"
            className="ghost-button"
            aria-label="Previous version"
            disabled={switchDisabled || !versions.previous}
            onClick={() => versions.previous && onSwitch?.(versions.previous)}
          >
            <Icon name="back" />
          </button>
          <span aria-live="polite">{versions.index + 1}/{versions.total}</span>
          <button
            type="button"
            className="ghost-button"
            aria-label="Next version"
            disabled={switchDisabled || !versions.next}
            onClick={() => versions.next && onSwitch?.(versions.next)}
          >
            <Icon name="chevron" />
          </button>
        </div>
      )}
      {items.map((item) => (
        <Tooltip key={item.id} label={item.label}>
          <button type="button" className="ghost-button" aria-label={item.label} onClick={item.onClick}>
            <Icon name={item.icon} />
          </button>
        </Tooltip>
      ))}
    </div>
  );
}
