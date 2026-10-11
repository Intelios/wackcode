import { useEffect, useRef, useState } from "react";
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

/** The page's host, for the header while a page has no title yet. */
function pageHost(url: string | undefined): string {
  if (!url) return "";
  try { return new URL(url).host; } catch { return ""; }
}

/** React owns the chrome; Rust places the untrusted native WKWebView over `browser-surface`. */
export function BrowserPanel({ taskId, state, visible, expanded, onState, onExpand, onReset, onClose }: BrowserPanelProps) {
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
    let frame: number | undefined;
    const hide = () => void api.browserPresent({ taskId, visible: false, x: 0, y: 0, width: 1, height: 1 }).catch(() => {});
    const place = () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      hide();
      if (!visible) return;
      // The drawer and page slides can move this fixed-size surface without resizing it.
      // Keep the native page parked until its full rectangle has stopped moving, rather than
      // measuring once after a delay that may expire partway through an animation.
      let bounds = element.getBoundingClientRect();
      let stillSince = performance.now();
      const settle = (now: number) => {
        const next = element.getBoundingClientRect();
        if (next.x !== bounds.x || next.y !== bounds.y || next.width !== bounds.width || next.height !== bounds.height) {
          bounds = next;
          stillSince = now;
        }
        if (now - stillSince < 140) {
          frame = window.requestAnimationFrame(settle);
          return;
        }
        frame = undefined;
        if (bounds.width < 1 || bounds.height < 1) return;
        void api.browserPresent({ taskId, visible: true, x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height })
          .then(onState).catch((reason) => setError(String(reason)));
      };
      frame = window.requestAnimationFrame(settle);
    };
    const observer = new ResizeObserver(place);
    observer.observe(element);
    // The outer drawer's width changes even when this surface keeps the final content width.
    const drawer = element.closest(".side-panel");
    if (drawer) observer.observe(drawer);
    window.addEventListener("resize", place);
    place();
    return () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
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
  const control = state?.userControl ? "Resume agent control" : "Take control";
  return (
    <section className="browser-panel">
      <header className="panel-header">
        <span className={`panel-mark ${state?.agentActive ? "live" : ""}`} aria-hidden="true"><Icon name="browser" /></span>
        <div className="panel-title" title={state?.url || undefined}>
          <span className="panel-kicker">Browser</span>
          <h3>{state?.title || pageHost(state?.url) || "No page open"}</h3>
        </div>
        {state?.agentActive ? (
          <span className="terminal-pill running" role="status"><span className="terminal-pill-dot" aria-hidden="true" />Agent is using the page</span>
        ) : state?.userControl ? (
          <span className="terminal-pill" role="status"><span className="terminal-pill-dot" aria-hidden="true" />You have control</span>
        ) : null}
        <div className="panel-header-actions">
          <button type="button" className="icon-button" title={expanded ? "Restore browser" : "Expand browser"} aria-label={expanded ? "Restore browser" : "Expand browser"} onClick={onExpand}><Icon name={expanded ? "collapse" : "expand"} /></button>
          <button type="button" className="icon-button" title="Close browser panel" aria-label="Close browser panel" onClick={onClose}><Icon name="close" /></button>
        </div>
        {state?.loading && <span className="panel-loading" aria-hidden="true" />}
      </header>
      <div className="browser-toolbar">
        <div className="browser-nav">
          <button type="button" className="icon-button" aria-label="Back" title="Back" disabled={!state?.canGoBack} onClick={() => void run(() => api.browserNavigation(taskId, "back"))}><Icon name="back" /></button>
          <button type="button" className="icon-button browser-forward" aria-label="Forward" title="Forward" disabled={!state?.canGoForward} onClick={() => void run(() => api.browserNavigation(taskId, "forward"))}><Icon name="back" /></button>
          <button type="button" className="icon-button" aria-label={state?.loading ? "Stop loading" : "Reload"} title={state?.loading ? "Stop loading" : "Reload"} disabled={!state?.exists} onClick={() => void run(() => api.browserNavigation(taskId, state?.loading ? "stop" : "reload"))}><Icon name={state?.loading ? "stop" : "refresh"} /></button>
        </div>
        <form className="browser-address" onSubmit={submit}>
          <Icon name={state?.url?.startsWith("https://") ? "lock" : "globe"} />
          <input aria-label="Browser address" value={address} placeholder="localhost:5173" spellCheck={false}
            onFocus={() => { addressFocused.current = true; }} onBlur={() => { addressFocused.current = false; }}
            onChange={(event) => setAddress(event.target.value)} />
        </form>
        <button type="button" className={`browser-control ${state?.userControl ? "active" : ""}`} disabled={!state?.exists}
          aria-label={control} title={control} aria-pressed={Boolean(state?.userControl)}
          onClick={() => void run(() => api.browserSetControl(taskId, !state?.userControl).then(onState))}>
          <Icon name="cursor" /><span>{control}</span>
        </button>
        <button type="button" className="icon-button" aria-label="Reset browser session" title="Reset browser session" disabled={!state?.exists} onClick={onReset}><Icon name="trash" /></button>
      </div>
      {state?.popup && <button type="button" className="browser-popup-return" onClick={() => void run(() => api.browserReturnFromPopup(taskId).then(onState))}><Icon name="back" /> Return to page</button>}
      {message && <div className="browser-error" role="alert"><Icon name="alert" /><span>{message}</span><button type="button" onClick={() => setError(undefined)}>Dismiss</button></div>}
      <div className="browser-surface" ref={surface}>
        {!state?.url && <div className="browser-empty"><span className="panel-empty-mark" aria-hidden="true"><Icon name="browser" /></span><h3>Preview a web app</h3><p>Enter a local or deployed HTTP address. The agent can open and use this browser too.</p></div>}
      </div>
    </section>
  );
}
