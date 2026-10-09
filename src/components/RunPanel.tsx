import { useEffect, useRef, useState } from "react";
import { useReducedMotion } from "motion/react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api } from "../api";
import type { AppearanceConfig, RunInfo } from "../types";
import { runIsActive, runStatusLabel } from "../run-state";
import { resolveTheme } from "../theme";
import { terminalTheme } from "../terminal-theme";
import { Icon } from "./Icons";
import { useContextClipboard, useContextMenu } from "./ui/ContextMenu";
import { Tooltip } from "./ui/Tooltip";

interface RunPanelProps {
  run?: RunInfo;
  appearance: AppearanceConfig;
  configured: boolean;
  onRun: () => void;
  onStop: () => void;
  onClose: () => void;
}

/** A self-contained PTY surface. Mounting attaches only; launching belongs to App. */
export function RunPanel({ run, appearance, configured, onRun, onStop, onClose }: RunPanelProps) {
  const reduce = useReducedMotion();
  const contextMenu = useContextMenu();
  const clipboard = useContextClipboard();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | undefined>(undefined);
  const [error, setError] = useState<string>();
  const [attachedSession, setAttachedSession] = useState<string>();
  const sessionId = run?.sessionId;
  const attached = Boolean(sessionId && attachedSession === sessionId);
  const live = run?.status === "running";

  useEffect(() => {
    setError(undefined);
    setAttachedSession(undefined);
    const host = hostRef.current;
    if (!host || !sessionId) return;
    const attachmentId = crypto.randomUUID();
    const term = new Terminal({
      fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace',
      fontSize: 12.5, lineHeight: 1.24, cursorBlink: !reduce, cursorStyle: "bar",
      scrollback: 5000, macOptionIsMeta: true, disableStdin: true,
      theme: terminalTheme(resolveTheme(appearance).variables)
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const key = event.key.toLowerCase();
      if (event.metaKey && event.altKey && (event.code === "Digit1" || event.code === "Digit2")) return false;
      return !((event.metaKey || event.ctrlKey) && (key === "n" || key === "o" || key === "," || (event.shiftKey && ["c", "t"].includes(key))));
    });
    term.open(host);
    fit.fit();
    termRef.current = term;
    let disposed = false;
    const data = term.onData((input) => {
      void api.writeRun(sessionId, input).catch((reason) => { if (!disposed) setError(String(reason)); });
    });
    void api.attachRun(sessionId, attachmentId, term.cols, term.rows, (frame) => {
      if (!disposed && frame.sessionId === sessionId && frame.attachmentId === attachmentId) term.write(frame.data);
    }).then((info) => {
      if (disposed) return;
      setAttachedSession(sessionId);
      if (info.status === "running") term.focus();
    }).catch((reason) => { if (!disposed) setError(String(reason)); });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      fit.fit();
      void api.resizeRun(sessionId, term.cols, term.rows).catch(() => undefined);
    });
    observer?.observe(host);
    return () => {
      disposed = true;
      observer?.disconnect();
      data.dispose();
      term.dispose();
      termRef.current = undefined;
      void api.detachRun(sessionId, attachmentId).catch(() => undefined);
    };
    // Attach identity follows the session, not its status or the selected chat.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = terminalTheme(resolveTheme(appearance).variables);
  }, [appearance]);
  useEffect(() => {
    if (termRef.current) {
      termRef.current.options.disableStdin = !attached || !live;
      termRef.current.options.cursorBlink = !reduce && live;
    }
  }, [attached, live, reduce, sessionId]);

  return (
    <div className="terminal-panel run-panel">
      <header className="panel-header terminal-header">
        <span className="terminal-glyph" aria-hidden="true"><Icon name="play" /></span>
        <div className="panel-title"><span className="panel-kicker">Run</span><h3 title={run?.command}>{run?.command ?? "Project command"}</h3></div>
        <div className="panel-header-actions">
          <Tooltip label="Clear screen" side="bottom">
            <button type="button" className="icon-button" aria-label="Clear Run terminal" disabled={!run} onClick={() => termRef.current?.clear()}><Icon name="erase" /></button>
          </Tooltip>
          {runIsActive(run) && <button type="button" className="icon-button" aria-label="Stop run" disabled={!live} onClick={onStop}><Icon name="stop" /></button>}
          <button type="button" className="icon-button" aria-label="Close Run panel" onClick={onClose}><Icon name="close" /></button>
        </div>
      </header>
      {run && <div className="run-panel-meta">
        <div className="run-panel-folder" title={run.cwd}><Icon name="folder" /><span>{run.cwd}</span></div>
        <span className={"terminal-pill " + (runIsActive(run) ? "running" : "stopped") + (run.status === "failed" ? " run-failed" : "")} role="status">
          <span className="terminal-pill-dot" aria-hidden="true" />{runStatusLabel(run)}
        </span>
      </div>}
      <div className="terminal-body">
        <div ref={hostRef} className="terminal-screen" onContextMenu={(event) => {
          const term = termRef.current;
          const selection = term?.getSelection() ?? "";
          contextMenu(event, [
            { label: "Copy selection", icon: <Icon name="copy" />, disabled: !clipboard || !selection, onSelect: () => clipboard?.copyText(selection) },
            { label: "Paste text", disabled: !clipboard || !live || !attached, onSelect: async () => {
              const text = await clipboard?.readText();
              if (text && termRef.current === term && !term?.options.disableStdin) term?.paste(text);
            } },
            { label: "Select all", hint: "⌘A", disabled: !term, onSelect: () => term?.selectAll() },
            { label: "Clear screen", icon: <Icon name="erase" />, disabled: !term, onSelect: () => term?.clear() }
          ], "Run terminal menu", false);
        }} />
        {!run && <div className="terminal-notice">
          <strong>No command running</strong><span>{configured ? "Run your project command to see its output here." : "Choose a project chat and set its command beside Run in the top bar."}</span>
          {configured && <button type="button" className="secondary-button compact" onClick={onRun}><Icon name="play" />Run</button>}
        </div>}
        {error && <div className="terminal-notice" role="alert"><strong>Run terminal error</strong><span>{error}</span></div>}
      </div>
    </div>
  );
}
