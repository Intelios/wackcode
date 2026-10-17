import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { motion, useReducedMotion } from "motion/react";
import { contextLabel } from "../../browser-context";
import type { BrowserContextRecord } from "../../types";
import { Icon } from "../Icons";
import { useNavigationDismiss } from "../ui/NavigationScope";

/** Also used for history: metadata and thumbnail stay small; the original loads on demand. */
export function BrowserContextChip({ context, preview, load, onRemove }: {
  context: Omit<BrowserContextRecord, "imageIndex">;
  preview?: string;
  load?: () => Promise<string | undefined>;
  onRemove?: () => void;
}) {
  const [open, setOpen] = useState(false);
  return <div className="browser-context-chip">
    <button type="button" className="browser-context-open" onClick={() => setOpen(true)} aria-label={`Inspect ${contextLabel(context)}`}>
      {preview ? <img src={preview} alt="" /> : <Icon name="viewfinder" />}
      <span><strong>{contextLabel(context)}</strong><small>{context.viewport.width} × {context.viewport.height} · {context.targets.length} {context.targets.length === 1 ? "element" : "elements"}</small></span>
    </button>
    {onRemove && <button type="button" className="icon-button" aria-label={`Remove ${contextLabel(context)}`} onClick={onRemove}><Icon name="close" /></button>}
    {open && <ContextInspector context={context} preview={preview} load={load} onClose={() => setOpen(false)} />}
  </div>;
}

function ContextInspector({ context, preview, load, onClose }: {
  context: Omit<BrowserContextRecord, "imageIndex">; preview?: string; load?: () => Promise<string | undefined>; onClose: () => void;
}) {
  const [image, setImage] = useState(preview);
  const reduce = useReducedMotion();
  const closeRef = useRef<HTMLButtonElement>(null);
  useNavigationDismiss(onClose);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    const key = (event: KeyboardEvent) => { if (event.key === "Escape") { event.stopPropagation(); onClose(); } };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); previous?.focus(); };
  }, [onClose]);
  useEffect(() => { let live = true; void load?.().then((url) => { if (live && url) setImage(url); }).catch(() => {}); return () => { live = false; }; }, [load]);
  return createPortal(<motion.div className="modal-backdrop browser-context-backdrop" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: reduce ? 0 : 0.15 }}
    onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="browser-context-inspector" role="dialog" aria-modal="true" aria-label="Browser context" onKeyDown={trapFocus}>
      <header className="panel-header"><span className="panel-mark"><Icon name="viewfinder" /></span><div className="panel-title"><span className="panel-kicker">Browser context</span><h3>{contextLabel(context)}</h3></div>
        <button ref={closeRef} type="button" className="icon-button" aria-label="Close browser context" onClick={onClose}><Icon name="close" /></button></header>
      <div className="browser-context-inspector-body">
        {image && <img className="browser-context-image" src={image} alt={contextLabel(context)} />}
        <div className="browser-context-details"><p>{context.title}</p><p className="browser-context-url">{context.url}</p><small>{context.viewport.width} × {context.viewport.height} · {new Date(context.capturedAt).toLocaleString()}</small>
          {(context.truncated || context.targets.some((target) => target.truncated)) && <p role="status">Some DOM context was shortened.</p>}
          {context.targets.map((target, index) => <details key={index} open={context.targets.length === 1}><summary>{target.label || target.selector}</summary><code>{target.path.map((part) => `${part.kind}: ${part.selector} → `).join("")}{target.selector}</code><pre>{target.dom}</pre></details>)}
          {!context.targets.length && <p>This screenshot carries visual context.</p>}
        </div>
      </div>
    </section>
  </motion.div>, document.body);
}

export function trapFocus(event: React.KeyboardEvent<HTMLElement>) {
  if (event.key !== "Tab") return;
  const nodes = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input, textarea, summary, [tabindex="0"]'));
  const first = nodes[0], last = nodes[nodes.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}
