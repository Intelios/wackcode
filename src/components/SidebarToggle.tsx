import { Icon } from "./Icons";
import { Tooltip } from "./ui/Tooltip";

interface SidebarToggleProps {
  /** The sidebar is on screen. */
  open: boolean;
  /** Open by choice but stepped aside for a wide side panel: the button wears a dot. */
  autoHidden: boolean;
  onToggle: () => void;
}

/**
 * The one sidebar toggle. It rides at the right end of the sidebar's title strip while the
 * sidebar is open and glides over beside the traffic lights when it closes, so it is always in
 * the same row the window drags from. It sits on `.app-shell` rather than inside `.sidebar`,
 * whose children the Settings rebuild animation addresses by position.
 */
export function SidebarToggle({ open, autoHidden, onToggle }: SidebarToggleProps) {
  const label = open ? "Hide sidebar" : "Show sidebar";
  return (
    <div className={`sidebar-toggle${open ? " open" : ""}`}>
      <Tooltip label={<>{label} <kbd>⌃⌘S</kbd></>} side="bottom">
        <button type="button" className="ghost-button sidebar-toggle-button" aria-label={label} aria-expanded={open} onClick={onToggle}>
          <Icon name="sidebar" />
          {autoHidden && <span className="sidebar-toggle-dot" aria-hidden="true" />}
        </button>
      </Tooltip>
    </div>
  );
}
