import { createContext, useContext, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { BrowserState } from "../types";
import { Icon } from "./Icons";

interface BrowserPanelProps {
  taskId: string;
  state?: BrowserState;
  visible: boolean;
  expanded: boolean;
  onState: (state: BrowserState) => void;
  onExpand: () => void;
  onReset: () => void;
  onClose: () => void;
}

/**
 * True while a full-window view (Git mode) covers the side panel. A context rather than the
 * `visible` prop: when the panel closes, AnimatePresence keeps rendering the exiting element
 * with its old props, so only a context update reaches it in time to hide the native page,
 * which otherwise floats above everything until the drawer finishes closing.
 */
export const NativeOverlaysHidden = createContext(false);

/** React owns the chrome; Rust places the untrusted native WKWebView over `browser-surface`. */
export function BrowserPanel({ taskId, state, visible: shown, expanded, onState, onExpand, onReset, onClose }: BrowserPanelProps) {
  const covered = useContext(NativeOverlaysHidden);
  const visible = shown && !covered;
  const surface = useRef<HTMLDivElement>(null);
  const addressFocused = useRef(false);
  const [address, setAddress] = useState(state?.url ?? "");
  const [error, setError] = useState<string>();

  useEffect(() => {
    void api.browserState(taskId).then(onState).catch((reason) => setError(String(reason)));
  }, [taskId, onState]);

  useEffect(() => {
    if (!addressFocused.current) setAddress(state?.url ?? "");
  }, [state?.url]);

  useEffect(() => {
    const element = surface.current;
    if (!element) return;
    let timer: number | undefined;
    const hide = () => void api.browserPresent({ taskId, visible: false, x: 0, y: 0, width: 1, height: 1 }).catch(() => {});
    const place = () => {
      window.clearTimeout(timer);
      hide();
      if (!visible) return;
      // Keep the native page below moving React chrome while the drawer or resizer settles.
      timer = window.setTimeout(() => {
        const bounds = element.getBoundingClientRect();
        if (bounds.width < 1 || bounds.height < 1) return;
        void api.browserPresent({ taskId, visible: true, x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height })
          .then(onState).catch((reason) => setError(String(reason)));
      }, 140);
    };
    const observer = new ResizeObserver(place);
    observer.observe(element);
    window.addEventListener("resize", place);
    place();
    return () => {
      window.clearTimeout(timer);
      observer.disconnect();
      window.removeEventListener("resize", place);
      hide();
    };
  }, [taskId, visible, expanded, state?.url, onState]);

  async function run(action: () => Promise<unknown>) {
    setError(undefined);
    try { await action(); } catch (reason) { setError(String(reason)); }
  }

  function submit(event: React.FormEvent) {
    event.preventDefault();
    const url = address.trim();
    if (url) void run(() => api.browserOpen(taskId, url).then(onState));
  }

  const message = error ?? state?.error;
  return (
    <section className="browser-panel">
      <header className="browser-panel-header">
        <div className="browser-panel-title">
          <span className={`browser-status-dot ${state?.agentActive ? "agent" : state?.loading ? "loading" : ""}`} />
          <div><strong>{state?.title || "Browser"}</strong><span>{state?.agentActive ? "Agent is using the page" : state?.userControl ? "You have control" : "Shared browser"}</span></div>
        </div>
        <div className="browser-panel-actions">
          <button type="button" className="icon-button" title={expanded ? "Restore browser" : "Expand browser"} aria-label={expanded ? "Restore browser" : "Expand browser"} onClick={onExpand}><Icon name={expanded ? "collapse" : "expand"} /></button>
          <button type="button" className="icon-button" title="Close browser panel" aria-label="Close browser panel" onClick={onClose}><Icon name="close" /></button>
        </div>
      </header>
      <div className="browser-toolbar">
        <button type="button" className="icon-button" aria-label="Back" title="Back" disabled={!state?.canGoBack} onClick={() => void run(() => api.browserNavigation(taskId, "back"))}><Icon name="back" /></button>
        <button type="button" className="icon-button browser-forward" aria-label="Forward" title="Forward" disabled={!state?.canGoForward} onClick={() => void run(() => api.browserNavigation(taskId, "forward"))}><Icon name="back" /></button>
        <button type="button" className="icon-button" aria-label={state?.loading ? "Stop loading" : "Reload"} title={state?.loading ? "Stop loading" : "Reload"} disabled={!state?.exists} onClick={() => void run(() => api.browserNavigation(taskId, state?.loading ? "stop" : "reload"))}><Icon name={state?.loading ? "stop" : "refresh"} /></button>
        <form className="browser-address" onSubmit={submit}>
          <Icon name="browser" />
          <input aria-label="Browser address" value={address} placeholder="localhost:5173" spellCheck={false}
            onFocus={() => { addressFocused.current = true; }} onBlur={() => { addressFocused.current = false; }}
            onChange={(event) => setAddress(event.target.value)} />
        </form>
        <button type="button" className={`secondary-button browser-control ${state?.userControl ? "active" : ""}`} disabled={!state?.exists}
          onClick={() => void run(() => api.browserSetControl(taskId, !state?.userControl).then(onState))}>
          {state?.userControl ? "Resume agent control" : "Take control"}
        </button>
        <button type="button" className="icon-button" aria-label="Reset browser session" title="Reset browser session" disabled={!state?.exists} onClick={onReset}><Icon name="trash" /></button>
      </div>
      {state?.popup && <button type="button" className="browser-popup-return" onClick={() => void run(() => api.browserReturnFromPopup(taskId).then(onState))}><Icon name="back" /> Return to page</button>}
      {message && <div className="browser-error" role="alert"><span>{message}</span><button type="button" onClick={() => setError(undefined)}>Dismiss</button></div>}
      <div className="browser-surface" ref={surface}>
        {!state?.url && <div className="browser-empty"><Icon name="browser" /><h3>Preview a web app</h3><p>Enter a local or deployed HTTP address. The agent can open and use this browser too.</p></div>}
      </div>
    </section>
  );
}
