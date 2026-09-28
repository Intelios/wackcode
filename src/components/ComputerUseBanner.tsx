import type { ComputerState } from "../types";
import { Icon } from "./Icons";

interface Props {
  /** The chat's computer-use state; renders nothing unless a run is using it. */
  computer?: ComputerState;
  onStop: () => void;
}

/**
 * Pinned above the composer while this chat's agent is using an app, so it's never unclear
 * that something else on the Mac is being driven — and how to stop it.
 */
export function ComputerUseBanner({ computer, onStop }: Props) {
  if (!computer?.active) return null;
  return (
    <div className="computer-banner-wrap">
      <section className="computer-banner" aria-label="Computer use" aria-live="polite">
        <span className="computer-banner-icon" aria-hidden="true"><Icon name="cursor" /></span>
        <span className="computer-banner-title">Computer use</span>
        <span className="computer-banner-detail" title={computer.app ?? undefined}>
          {computer.app ? `Using ${computer.app}` : "Using an app"}
        </span>
        {computer.hotkey && <kbd className="computer-banner-key" title="Stop shortcut">⌃⌥⌘.</kbd>}
        <button type="button" className="computer-banner-stop" onClick={onStop}>Stop</button>
      </section>
    </div>
  );
}
