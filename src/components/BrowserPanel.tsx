import { useContext, useEffect, useRef, useState } from "react";
import { motion, useIsPresent, useReducedMotion } from "motion/react";
import { api } from "../api";
import type { BrowserContextAttachment, BrowserState, BrowserViewportPreset } from "../types";
import { boundTargets, BROWSER_VIEWPORTS, flattenCapture } from "../browser-context";
import { AssistantNameContext } from "../agentName";
import { Icon } from "./Icons";
import { BrowserMarkup } from "./browser/BrowserMarkup";

interface BrowserPanelProps {
  taskId: string;
  state?: BrowserState;
  visible: boolean;
  expanded: boolean;
  onState: (state: BrowserState) => void;
  onExpand: () => void;
  onReset: () => void;
  onClose: () => void;
  onAttach?: (context: BrowserContextAttachment) => void;
}

/** The page's host, for the header while a page has no title yet. */
function pageHost(url: string | undefined): string {
  if (!url) return "";
  try { return new URL(url).host; } catch { return ""; }
}

const VIEWPORT_LABELS: Record<BrowserViewportPreset, string> = { desktop: "Desktop", laptop: "Laptop", tablet: "Tablet", phone: "Phone" };

/** React owns the chrome; Rust places the untrusted native WKWebView over `browser-surface`. */
export function BrowserPanel({ taskId, state, visible, expanded, onState, onExpand, onReset, onClose, onAttach }: BrowserPanelProps) {
  const surface = useRef<HTMLDivElement>(null);
  const addressFocused = useRef(false);
  const [address, setAddress] = useState(state?.url ?? "");
  const [error, setError] = useState<string>();
  const [markup, setMarkup] = useState<BrowserContextAttachment>();
  const [modalOpen, setModalOpen] = useState(false);
  const [busy, setBusy] = useState<"pick" | "capture">();
  const [displayScale, setDisplayScale] = useState(1);
  const presentationId = useRef(crypto.randomUUID());
  const captureId = useRef<string | undefined>(undefined);
  const isPresent = useIsPresent();
  const reduceMotion = useReducedMotion();
  const agent = useContext(AssistantNameContext);
  const captureAllowed = useRef(visible && isPresent);
  captureAllowed.current = visible && isPresent;
  const viewport = state?.viewport ?? BROWSER_VIEWPORTS.desktop;
  const presented = visible && isPresent && !modalOpen && !markup;

  useEffect(() => {
    const update = () => setModalOpen(Boolean(document.querySelector(".modal-backdrop, .popover")));
    const observer = new MutationObserver(update);
    observer.observe(document.body, { childList: true, subtree: true }); update();
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const owner = crypto.randomUUID();
    presentationId.current = owner;
    return () => {
      if (captureId.current) void api.browserCancelCapture(taskId, captureId.current).catch(() => {});
      captureId.current = undefined;
      void api.browserRetirePresentation(taskId, owner).catch(() => {});
    };
  }, [taskId]);
  useEffect(() => {
    if (visible && isPresent) return;
    const id = captureId.current;
    captureId.current = undefined;
    setBusy(undefined);
    if (id) void api.browserCancelCapture(taskId, id).catch(() => {});
    if (!isPresent) {
      setMarkup(undefined);
      void api.browserRetirePresentation(taskId, presentationId.current).catch(() => {});
    }
  }, [visible, isPresent, taskId]);

  useEffect(() => {
    void api.browserState(taskId).then(onState).catch((reason) => setError(String(reason)));
  }, [taskId, onState]);

  useEffect(() => {
    if (!addressFocused.current) setAddress(state?.url ?? "");
  }, [state?.url]);

  useEffect(() => {
    const element = surface.current;
    if (!element) return;
    const owner = presentationId.current;
    let frame: number | undefined;
    const hide = () => void api.browserPresent({ taskId, presentationId: owner, visible: false, x: 0, y: 0, width: 1, height: 1 }).catch(() => {});
    const place = () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      hide();
      if (!presented) return;
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
        setDisplayScale(Math.min(bounds.width / viewport.width, bounds.height / viewport.height));
        void api.browserPresent({ taskId, presentationId: owner, visible: true, x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height })
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
  }, [taskId, presented, expanded, state?.url, viewport.width, viewport.height, presentationId, onState]);

  async function capture(kind: "pick" | "capture") {
    const id = crypto.randomUUID(); captureId.current = id; setBusy(kind); setError(undefined);
    const theme = getComputedStyle(document.documentElement);
    try {
      const result = kind === "pick"
        ? await api.browserPick(taskId, id, presentationId.current, theme.getPropertyValue("--wc-accent").trim(), theme.getPropertyValue("--wc-well").trim())
        : await api.browserCapture(taskId, id, presentationId.current);
      if (captureId.current !== id || !captureAllowed.current || !result) return;
      if (kind === "capture") setMarkup(result);
      else {
        const target = result.targets[0];
        const marks = target ? [{ kind: "box" as const, points: [{ x: target.bounds.x, y: target.bounds.y }, { x: target.bounds.x + target.bounds.width, y: target.bounds.y + target.bounds.height }] }] : [];
        const image = await flattenCapture(result, marks, theme.getPropertyValue("--wc-accent"), theme.getPropertyValue("--wc-well"));
        if (captureId.current === id && captureAllowed.current) onAttach?.({ ...result, ...boundTargets(result.targets), image });
      }
    } catch (reason) { if (captureId.current === id) setError(String(reason)); }
    finally { if (captureId.current === id) { captureId.current = undefined; setBusy(undefined); } }
  }

  function cancelPick() {
    const id = captureId.current; captureId.current = undefined; setBusy(undefined);
    if (id) void api.browserCancelCapture(taskId, id).catch(() => {});
  }

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
  const hands = state?.userControl ? "user" : state?.agentActive ? "agent" : undefined;
  const readout = busy === "pick" ? "Pick · ⎋" : busy === "capture" ? "Capturing…" : state?.url ? `${Math.round(displayScale * 100)}%` : "";
  return (
    <section className="browser-panel">
      <header className="panel-header" data-tauri-drag-region>
        <span className={`panel-mark ${state?.agentActive ? "live" : ""}`} aria-hidden="true"><Icon name="browser" /></span>
        <div className="panel-title" title={state?.url || undefined} data-tauri-drag-region>
          <span className="panel-kicker" data-tauri-drag-region>Browser</span>
          <h3 data-tauri-drag-region>{state?.title || pageHost(state?.url) || "No page open"}</h3>
        </div>
        {state?.agentActive ? (
          <span className="terminal-pill running" role="status"><span className="terminal-pill-dot" aria-hidden="true" />Agent is using the page</span>
        ) : state?.userControl ? (
          <span className="terminal-pill" role="status"><span className="terminal-pill-dot" aria-hidden="true" />You have control</span>
        ) : null}
        <div className="panel-header-actions">
          <button type="button" className="icon-button" aria-label="Reset browser session" title="Reset browser session" disabled={!state?.exists} onClick={onReset}><Icon name="trash" /></button>
          <button type="button" className="icon-button" title={expanded ? "Restore browser" : "Expand browser"} aria-label={expanded ? "Restore browser" : "Expand browser"} onClick={onExpand}><Icon name={expanded ? "collapse" : "expand"} /></button>
          <button type="button" className="icon-button" title="Close browser panel" aria-label="Close browser panel" onClick={onClose}><Icon name="close" /></button>
        </div>
        {state?.loading && <span className="panel-loading" aria-hidden="true" />}
      </header>
      <div className="browser-toolbar">
        <div className="browser-nav">
          <button type="button" className="icon-button" aria-label="Back" title="Back" disabled={!state?.canGoBack} onClick={() => void run(() => api.browserNavigation(taskId, "back"))}><Icon name="back" /></button>
          <button type="button" className="icon-button browser-forward" aria-label="Forward" title="Forward" disabled={!state?.canGoForward} onClick={() => void run(() => api.browserNavigation(taskId, "forward"))}><Icon name="back" /></button>
          <button type="button" className={`icon-button browser-reload${state?.loading ? " loading" : ""}`} aria-label={state?.loading ? "Stop loading" : "Reload"} title={state?.loading ? "Stop loading" : "Reload"} disabled={!state?.exists} onClick={() => void run(() => api.browserNavigation(taskId, state?.loading ? "stop" : "reload"))}><Icon name={state?.loading ? "stop" : "refresh"} /></button>
        </div>
        <form className={`browser-address${state?.loading ? " loading" : ""}`} onSubmit={submit}>
          <Icon name={state?.url?.startsWith("https://") ? "lock" : "globe"} />
          <input aria-label="Browser address" value={address} placeholder="localhost:5173" spellCheck={false}
            onFocus={(event) => { addressFocused.current = true; event.target.select(); }} onBlur={() => { addressFocused.current = false; }}
            onChange={(event) => setAddress(event.target.value)} />
        </form>
        <button type="button" className={`browser-control ${state?.userControl ? "active" : ""}`} disabled={!state?.exists}
          aria-label={control} title={control} aria-pressed={Boolean(state?.userControl)}
          onClick={() => void run(() => api.browserSetControl(taskId, !state?.userControl).then(onState))}>
          <Icon name="cursor" /><span>{control}</span>
        </button>
      </div>
      <div className="browser-stagebar">
        <div className="browser-viewports" role="radiogroup" aria-label="Browser viewport">
          {Object.values(BROWSER_VIEWPORTS).map((preset) => {
            const selected = preset.preset === viewport.preset;
            const label = `${VIEWPORT_LABELS[preset.preset]}, ${preset.width} × ${preset.height}`;
            return (
              <button key={preset.preset} type="button" role="radio" aria-checked={selected} aria-label={label} title={label}
                className={`browser-viewport${selected ? " selected" : ""}`} disabled={Boolean(busy)}
                onClick={() => { if (!selected) void run(() => api.browserSetViewport(taskId, preset.preset).then(onState)); }}>
                {selected && <motion.span className="browser-viewport-pill" aria-hidden="true" layoutId={reduceMotion ? undefined : `browser-viewport-${taskId}`}
                  style={{ borderRadius: 999 }} transition={{ type: "spring", stiffness: 480, damping: 36 }} />}
                <Icon name={preset.preset} />
              </button>
            );
          })}
        </div>
        <span className="browser-readout"><span>{viewport.width} × {viewport.height}</span><small className={busy ? "busy" : undefined}>{readout}</small></span>
        <div className="browser-capture-actions">
          <button type="button" className={`icon-button${busy === "pick" ? " active" : ""}`} aria-label={busy === "pick" ? "Cancel element picking" : "Pick an element"} title={busy === "pick" ? "Cancel element picking (⎋)" : "Pick an element"}
            aria-pressed={busy === "pick"} disabled={!state?.url || busy === "capture" || !onAttach} onClick={() => busy === "pick" ? cancelPick() : void capture("pick")}><Icon name="viewfinder" /></button>
          <button type="button" className="icon-button" aria-label="Mark up screenshot" title="Mark up screenshot" disabled={!state?.url || Boolean(busy) || !onAttach} onClick={() => void capture("capture")}><Icon name="pencil" /></button>
        </div>
      </div>
      <div className="browser-stage">
        {state?.popup && <button type="button" className="browser-popup-return" onClick={() => void run(() => api.browserReturnFromPopup(taskId).then(onState))}><Icon name="back" /> Return to page</button>}
        {message && <div className="browser-error" role="alert"><Icon name="alert" /><span>{message}</span><button type="button" onClick={() => setError(undefined)}>Dismiss</button></div>}
        <div className="browser-surface" ref={surface} style={{ "--browser-aspect": viewport.width / viewport.height } as React.CSSProperties}>
          {state?.url ? (
            <span key={viewport.preset} className={`browser-frame${hands ? ` ${hands}` : ""}`} aria-hidden="true" />
          ) : (
            <div className="browser-empty">
              <span className="panel-empty-mark" aria-hidden="true"><Icon name="browser" /></span>
              <h3>Preview a web app</h3>
              <p>Enter a local or deployed HTTP address above. {agent} can open and use this browser too.</p>
            </div>
          )}
        </div>
      </div>
      {markup && isPresent && <BrowserMarkup capture={markup} onAdd={(context) => onAttach?.(context)} onClose={() => setMarkup(undefined)} />}
    </section>
  );
}
