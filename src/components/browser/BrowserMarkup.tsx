/** Frozen pixels and captured geometry share CSS coordinates. Nothing here inspects the live page. */
import { useEffect, useMemo, useRef, useState, type PointerEvent, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { motion, useReducedMotion } from "motion/react";
import { boundTargets, drawMarks, flattenCapture, matchedTargets, type BrowserMark, type MarkPoint } from "../../browser-context";
import { imageDataUrl } from "../../attachment-utils";
import type { BrowserContextAttachment } from "../../types";
import { Icon, type IconName } from "../Icons";
import { useNavigationDismiss } from "../ui/NavigationScope";
import { trapFocus } from "./BrowserContextChip";

const TOOLS: { kind: BrowserMark["kind"]; label: string; icon: IconName }[] = [
  { kind: "pen", label: "Pen", icon: "pencil" }, { kind: "arrow", label: "Arrow", icon: "markupArrow" },
  { kind: "box", label: "Box", icon: "markupBox" }, { kind: "text", label: "Text", icon: "markupText" }
];

export function BrowserMarkup({ capture, onAdd, onClose }: { capture: BrowserContextAttachment; onAdd: (context: BrowserContextAttachment) => void; onClose: () => void }) {
  const reduce = useReducedMotion();
  const [tool, setTool] = useState<BrowserMark["kind"]>("pen");
  const [marks, setMarks] = useState<BrowserMark[]>([]);
  const [redo, setRedo] = useState<BrowserMark[]>([]);
  const [active, setActive] = useState<BrowserMark>();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const canvas = useRef<HTMLCanvasElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const live = useRef(true);
  const activeMark = useRef<BrowserMark | undefined>(undefined);
  const targets = useMemo(() => boundTargets(matchedTargets(marks, capture.targets)), [marks, capture.targets]);
  useNavigationDismiss(onClose);
  const undo = () => { const last = marks.at(-1); if (last) { setRedo([...redo, last]); setMarks(marks.slice(0, -1)); } };
  const redoMark = () => { const last = redo.at(-1); if (last) { setMarks([...marks, last]); setRedo(redo.slice(0, -1)); } };
  const actions = useRef({ undo, redoMark }); actions.current = { undo, redoMark };
  useEffect(() => {
    live.current = true;
    const previous = document.activeElement as HTMLElement | null;
    cancel.current?.focus();
    return () => { live.current = false; previous?.focus(); };
  }, []);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.stopPropagation(); if (!busy) onClose(); }
      if (event.metaKey && event.key.toLowerCase() === "z" && !(event.target instanceof HTMLInputElement)) { event.preventDefault(); if (event.shiftKey) actions.current.redoMark(); else actions.current.undo(); }
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [onClose, busy]);
  useEffect(() => {
    const context = canvas.current?.getContext("2d");
    if (!context) return;
    context.clearRect(0, 0, capture.viewport.width, capture.viewport.height);
    const theme = getComputedStyle(document.documentElement);
    drawMarks(context, active ? [...marks, active] : marks, theme.getPropertyValue("--wc-accent"), theme.getPropertyValue("--wc-well"));
  }, [marks, active, capture]);

  function point(event: PointerEvent<HTMLCanvasElement>): MarkPoint {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: Math.max(0, Math.min(capture.viewport.width, (event.clientX - rect.x) * capture.viewport.width / rect.width)),
      y: Math.max(0, Math.min(capture.viewport.height, (event.clientY - rect.y) * capture.viewport.height / rect.height)) };
  }
  function start(event: PointerEvent<HTMLCanvasElement>) {
    if (event.button !== 0 || busy) return;
    if (tool === "text") {
      if (!text.trim()) { setError("Write a label, then click where it belongs."); return; }
      // React clears currentTarget after dispatch; read geometry before scheduling an updater.
      const at = point(event), label = text.trim();
      setMarks((current) => [...current, { kind: "text", points: [at], text: label }]); setRedo([]); setError(undefined); return;
    }
    event.currentTarget.setPointerCapture(event.pointerId);
    activeMark.current = { kind: tool, points: [point(event)] }; setActive(activeMark.current);
  }
  function move(event: PointerEvent<HTMLCanvasElement>) {
    const mark = activeMark.current;
    if (!mark || mark.kind === "text") return;
    activeMark.current = { ...mark, points: mark.kind === "pen" ? [...mark.points, point(event)] : [mark.points[0], point(event)] };
    setActive(activeMark.current);
  }
  function finish() {
    const mark = activeMark.current;
    if (mark && mark.points.length > 1) { setMarks((current) => [...current, mark]); setRedo([]); }
    activeMark.current = undefined; setActive(undefined);
  }
  async function add() {
    setBusy(true); setError(undefined);
    try {
      const theme = getComputedStyle(document.documentElement);
      const image = await flattenCapture(capture, marks, theme.getPropertyValue("--wc-accent"), theme.getPropertyValue("--wc-well"));
      if (!live.current) return;
      onAdd({ ...capture, ...targets, truncated: capture.truncated || targets.truncated, image }); onClose();
    } catch (reason) { if (live.current) { setError(String(reason)); setBusy(false); } }
  }
  return createPortal(<motion.div className="modal-backdrop browser-context-backdrop" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: reduce ? 0 : 0.18 }}>
    <section className="browser-markup" role="dialog" aria-modal="true" aria-label="Mark up browser screenshot" onKeyDown={trapFocus}>
      <header className="panel-header"><span className="panel-mark"><Icon name="pencil" /></span><div className="panel-title"><span className="panel-kicker">Frozen screenshot</span><h3>{capture.title || "Mark up the page"}</h3></div><small>{capture.viewport.width} × {capture.viewport.height}</small></header>
      <div className="browser-markup-toolbar"><div className="segmented" role="group" aria-label="Markup tools">{TOOLS.map((item) => <button type="button" key={item.kind} aria-pressed={tool === item.kind} className={tool === item.kind ? "active" : ""} disabled={busy} onClick={() => setTool(item.kind)}><Icon name={item.icon} />{item.label}</button>)}</div>
        <button type="button" className="icon-button" aria-label="Undo markup" title="Undo (⌘Z)" disabled={!marks.length || busy} onClick={undo}><Icon name="undo" /></button>
        <button type="button" className="icon-button" aria-label="Redo markup" title="Redo (⌘⇧Z)" disabled={!redo.length || busy} onClick={redoMark}><Icon name="redo" /></button>
        {tool === "text" && <input aria-label="Markup text" placeholder="Write a label, then click the image" maxLength={300} value={text} onChange={(event) => setText(event.target.value)} />}
      </div>
      <div className="browser-markup-stage"><div className="browser-markup-image" style={{ aspectRatio: `${capture.viewport.width} / ${capture.viewport.height}`, "--markup-aspect": capture.viewport.width / capture.viewport.height } as CSSProperties}>
        <img src={imageDataUrl(capture.image)} alt="Frozen browser screenshot" draggable={false} />
        <canvas ref={canvas} width={capture.viewport.width} height={capture.viewport.height} aria-label="Draw on screenshot" onPointerDown={start} onPointerMove={move} onPointerUp={finish} onPointerCancel={() => { activeMark.current = undefined; setActive(undefined); }} />
      </div></div>
      <footer className="browser-markup-footer"><span role="status">{error || `${targets.targets.length} matched ${targets.targets.length === 1 ? "element" : "elements"}${capture.truncated || targets.truncated ? " · DOM context shortened" : ""}`}</span><button ref={cancel} type="button" className="secondary-button" disabled={busy} onClick={onClose}>Cancel</button><button type="button" className="primary-button" disabled={busy} onClick={() => void add()}>{busy ? "Preparing…" : "Add to draft"}</button></footer>
    </section>
  </motion.div>, document.body);
}
