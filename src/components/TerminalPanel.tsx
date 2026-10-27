import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useContextClipboard, useContextMenu } from "./ui/ContextMenu";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api } from "../api";
import { normalizeTerminalSelection } from "../attachment-utils";
import { resolveTheme } from "../theme";
import { terminalTheme } from "../terminal-theme";
import type { AppearanceConfig, TerminalExit, TerminalFrame, TerminalInfo } from "../types";
import { Icon } from "./Icons";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { Tooltip } from "./ui/Tooltip";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

interface TerminalPanelProps {
  taskId: string;
  /** Appearance drives the xterm palette; the theme re-reads it live as it changes. */
  appearance: AppearanceConfig;
  /** Takes the tidied selection into the composer draft (App's handler), throwing a
   *  user-facing sentence when the draft refuses it — the same contract as BrowserPanel's
   *  `onAttach`. Omit to hide the add-to-draft affordances. */
  onAttach?: (text: string) => void;
}

function exitText(exit: TerminalExit): string {
  return exit.signal ? `Shell ended on ${exit.signal}` : exit.code === 0 ? "Shell exited" : `Shell exited (code ${exit.code})`;
}

function StatusPill({ exited, busy }: { exited: boolean; busy: boolean }) {
  // An idle shell says nothing; the pill appears only when it has something to tell.
  if (!exited && !busy) return null;
  return (
    <span className={`terminal-pill ${exited ? "stopped" : "running"}`} role="status">
      <span className="terminal-pill-dot" aria-hidden="true" />
      {exited ? "Exited" : "Working"}
    </span>
  );
}

/**
 * The side panel's Terminal view: the chat's own login shell on a PTY (`terminal.rs`). xterm
 * renders output straight off a Tauri channel — no React state in the hot path — and keystrokes
 * go back over `write_terminal`. The panel owns the PTY's lifetime only while attached: hiding
 * it detaches and leaves the shell running, and so does closing its tab.
 */
export function TerminalPanel({ taskId, appearance, onAttach }: TerminalPanelProps) {
  const contextMenu = useContextMenu();
  const clipboard = useContextClipboard();
  const reduce = useReducedMotion();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | undefined>(undefined);
  const fitRef = useRef<FitAddon | undefined>(undefined);
  const frameRef = useRef<(frame: TerminalFrame) => void>(() => undefined);
  const [info, setInfo] = useState<TerminalInfo>();
  const [exit, setExit] = useState<TerminalExit | null>(null);
  const [busy, setBusy] = useState(false);
  const [ended, setEnded] = useState(false);
  const [error, setError] = useState<string>();
  /** The add-to-draft pill's anchor over `.terminal-body`, while a selection rests. */
  const [pill, setPill] = useState<{ top: number; left: number } | null>(null);
  /** Why the composer refused a selection, if it did; not a shell problem. */
  const [notice, setNotice] = useState<string>();
  const draggingRef = useRef(false);
  const [confirmAction, setConfirmAction] = useState<"end" | "restart">();
  /** Set while a freshly spawned shell plays the power-on sweep; reattaches skip it. */
  const [booting, setBooting] = useState(false);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const term = new Terminal({
      fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace',
      fontSize: 12.5,
      lineHeight: 1.24,
      cursorBlink: !reduce,
      cursorStyle: "bar",
      scrollback: 5000,
      macOptionIsMeta: true,
      theme: terminalTheme(resolveTheme(appearance).variables)
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    // The app's own shortcuts (⌘N, ⌘O, ⌘,, ⌘⇧C, ⌘⇧T, ⌥⌘1, ⌥⌘2) keep working while the shell has focus;
    // everything else belongs to it. Mirrors the keydown list in App.tsx.
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      const key = event.key.toLowerCase();
      if (event.metaKey && event.altKey && (event.code === "Digit1" || event.code === "Digit2")) return false;
      if (event.metaKey || event.ctrlKey) {
        if (key === "n" || key === "o" || key === "," || (event.shiftKey && (key === "c" || key === "t"))) return false;
      }
      return true;
    });
    term.open(host);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;
    const dataDisposable = term.onData((data) => void api.writeTerminal(taskId, data).catch(() => undefined));
    // The pill follows the selection. A drag's intermediate states never show it — it would
    // steal the drag's mouse-up — so the mouse handlers on the host re-read on release.
    const selectionDisposable = term.onSelectionChange(() => { setNotice(undefined); readSelection(); });

    let disposed = false;
    frameRef.current = (frame: TerminalFrame) => {
      if (disposed) return;
      if (frame.type === "output") term.write(frame.data);
      else if (frame.type === "exit") {
        setExit({ code: frame.code ?? -1, signal: frame.signal });
        // A dead shell can't take input; keystrokes stop dead-ending into the writer.
        term.options.disableStdin = true;
      } else if (frame.type === "busy") setBusy(frame.busy);
    };

    api.openTerminal(taskId, term.cols, term.rows, (frame) => frameRef.current(frame))
      .then((result) => {
        if (disposed) return;
        setInfo(result);
        setExit(result.exit);
        setBusy(result.busy);
        setBooting(result.fresh);
        term.options.disableStdin = result.exit !== null;
        if (!result.exit) term.focus();
      })
      .catch((reason) => { if (!disposed) setError(String(reason)); });

    // The panel's resizer and the page animation both change the host; refit on either.
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      fit.fit();
      void api.resizeTerminal(taskId, term.cols, term.rows).catch(() => undefined);
    });
    observer?.observe(host);

    return () => {
      disposed = true;
      observer?.disconnect();
      dataDisposable.dispose();
      selectionDisposable.dispose();
      term.dispose();
      void api.detachTerminal(taskId).catch(() => undefined);
    };
    // The shell belongs to the chat, not this mount: one attach per taskId.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [taskId]);

  // Re-tint the palette when the theme changes; the shell session itself is untouched.
  useEffect(() => {
    const term = termRef.current;
    if (term) term.options.theme = terminalTheme(resolveTheme(appearance).variables);
  }, [appearance]);

  async function restart() {
    const term = termRef.current;
    if (!term) return;
    setError(undefined);
    try {
      term.reset();
      term.options.disableStdin = false;
      setExit(null);
      setEnded(false);
      const result = await api.restartTerminal(taskId, term.cols, term.rows, (frame) => frameRef.current(frame));
      setInfo(result);
      setBusy(result.busy);
      setBooting(true);
      term.focus();
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function open() {
    const term = termRef.current;
    if (!term) return;
    setError(undefined);
    // "New shell" after an explicit end starts clean; a retry after an error keeps the screen.
    if (ended) term.reset();
    term.options.disableStdin = false;
    try {
      const result = await api.openTerminal(taskId, term.cols, term.rows, (frame) => frameRef.current(frame));
      setInfo(result);
      setExit(result.exit);
      setBusy(result.busy);
      setBooting(result.fresh);
      setEnded(false);
      term.options.disableStdin = result.exit !== null;
      if (!result.exit) term.focus();
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function endSession() {
    try {
      await api.closeTerminal(taskId);
      setEnded(true);
    } catch (reason) {
      setError(String(reason));
    }
  }

  /** The pill's anchor: the selection's end cell as pixels over `.terminal-body` — above the
   *  last selected row (below it when there is no room) and clamped to the body's width.
   *  Cell metrics come from the `.xterm-screen` box xterm already measured. */
  function readSelection() {
    const term = termRef.current;
    const host = hostRef.current;
    if (!term || !host || draggingRef.current) return;
    const text = term.getSelection();
    const range = term.getSelectionPosition();
    if (!text || !range) { setPill(null); return; }
    const rect = host.querySelector(".xterm-screen")?.getBoundingClientRect();
    const body = host.getBoundingClientRect();
    const cellW = rect && rect.width > 0 ? rect.width / term.cols : 0;
    const cellH = rect && rect.height > 0 ? rect.height / term.rows : 0;
    const row = Math.min(Math.max(range.end.y, 0), term.rows - 1);
    const col = Math.min(Math.max(range.end.x, 0), term.cols - 1);
    const x = (rect?.left ?? body.left) - body.left + col * cellW;
    const y = (rect?.top ?? body.top) - body.top + row * cellH;
    setPill({ top: y >= 36 ? y - 32 : y + cellH + 6, left: Math.min(x, Math.max(0, host.clientWidth - 132)) });
  }

  /** The pill and the context menu's one action: hand the tidied selection to the composer. */
  function addSelection() {
    const text = termRef.current?.getSelection();
    if (!text || !onAttach) return;
    setNotice(undefined);
    try {
      onAttach(normalizeTerminalSelection(text));
    } catch (reason) {
      // The composer refused it (its caps): the sentence explains, the selection is untouched.
      setNotice(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const title = info ? `${info.shell} · ${info.cwd.split("/").filter(Boolean).pop() ?? info.cwd}` : "Shell";
  const live = !ended && !exit;

  return (
    <div className="terminal-panel">
      <header className="panel-header terminal-header">
        <div className="panel-title" title={info?.cwd}>
          <h3>{title}</h3>
        </div>
        <StatusPill exited={exit !== null || ended} busy={busy && live} />
        <div className="panel-header-actions">
          <Tooltip label="Clear screen" side="bottom">
            <button type="button" className="icon-button" aria-label="Clear terminal" onClick={() => termRef.current?.clear()} disabled={!live}>
              <Icon name="erase" />
            </button>
          </Tooltip>
          <Tooltip label="Restart shell" side="bottom">
            <button type="button" className="icon-button" aria-label="Restart shell" onClick={() => (busy && live ? setConfirmAction("restart") : void restart())}>
              <Icon name="refresh" />
            </button>
          </Tooltip>
          {!ended && (
            <Tooltip label="End session" side="bottom">
              <button type="button" className="icon-button" aria-label="End terminal session" onClick={() => (busy && live ? setConfirmAction("end") : void endSession())}>
                <Icon name="stop" />
              </button>
            </Tooltip>
          )}
        </div>
        {busy && live && <span className="panel-loading" aria-hidden="true" />}
      </header>

      <div className="terminal-body">
        <div
          ref={hostRef}
          onMouseDown={() => { draggingRef.current = true; setPill(null); }}
          onMouseUp={() => { draggingRef.current = false; readSelection(); }}
          onContextMenu={(event) => {
            const term = termRef.current;
            const selection = term?.getSelection() ?? "";
            contextMenu(event, [
              { label: "Copy selection", icon: <Icon name="copy" />, disabled: !clipboard || !selection, onSelect: () => clipboard?.copyText(selection) },
              { label: "Add selection to draft", icon: <Icon name="filePlus" />, disabled: !onAttach || !selection, onSelect: () => addSelection() },
              { label: "Paste text", disabled: !clipboard || !live || !info || Boolean(error), onSelect: async () => {
                const text = await clipboard?.readText();
                if (text && termRef.current === term && !term?.options.disableStdin) term?.paste(text);
              } },
              { label: "Select all", hint: "⌘A", disabled: !term, onSelect: () => term?.selectAll() },
              "separator",
              { label: "Clear screen", icon: <Icon name="erase" />, disabled: !live, onSelect: () => term?.clear() },
              { label: "Restart shell", icon: <Icon name="refresh" />, onSelect: () => (busy && live ? setConfirmAction("restart") : void restart()) },
              { label: "End session", icon: <Icon name="stop" />, disabled: ended, danger: true, onSelect: () => (busy && live ? setConfirmAction("end") : void endSession()) }
            ], "Terminal menu", false);
          }}
          className={`terminal-screen ${booting ? "power-on" : ""} ${exit || ended || error ? "settled" : ""}`}
          onAnimationEnd={(event) => { if (event.animationName === "terminal-sweep") setBooting(false); }}
        />
        {pill && onAttach && (
          <button
            type="button"
            className="terminal-add-pill"
            style={{ top: pill.top, left: pill.left }}
            // Keep the terminal's focus — and the selection that anchors this pill — alive.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => addSelection()}
          >
            <Icon name="filePlus" /> Add to draft
          </button>
        )}
        {notice && <div className="terminal-add-notice" role="status">{notice}</div>}
        <AnimatePresence>
          {error && !ended && (
            <motion.div
              key="terminal-error"
              className="terminal-notice"
              role="alert"
              initial={reduce ? false : { opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, transition: { duration: 0.15 } }}
              transition={{ duration: 0.3, ease: EASE }}
            >
              <strong>Couldn’t reach the shell</strong>
              <span>{error}</span>
              <button type="button" className="secondary-button compact" onClick={() => void open()}><Icon name="refresh" /> Try again</button>
            </motion.div>
          )}
          {exit && !ended && !error && (
            <motion.div
              key="terminal-exit"
              className="terminal-notice"
              role="status"
              initial={reduce ? false : { opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, transition: { duration: 0.15 } }}
              transition={{ duration: 0.3, ease: EASE }}
            >
              <strong>{exitText(exit)}</strong>
              <span>The terminal keeps the shell’s last screen above.</span>
              <button type="button" className="secondary-button compact" onClick={() => void restart()}><Icon name="refresh" /> Start a new shell</button>
            </motion.div>
          )}
          {ended && (
            <motion.div
              key="terminal-ended"
              className="terminal-notice"
              role="status"
              initial={reduce ? false : { opacity: 0, y: 14 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, transition: { duration: 0.15 } }}
              transition={{ duration: 0.3, ease: EASE }}
            >
              <strong>Session ended</strong>
              <span>Start a fresh login shell in this chat’s folder.</span>
              <button type="button" className="secondary-button compact" onClick={() => void open()}><Icon name="terminal" /> New shell</button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {confirmAction && (
        <ConfirmDialog
          title={confirmAction === "end" ? "End the terminal session?" : "Restart the shell?"}
          body="A command is still running in it and will be stopped."
          confirmLabel={confirmAction === "end" ? "End session" : "Restart"}
          danger
          onCancel={() => setConfirmAction(undefined)}
          onConfirm={async () => {
            setConfirmAction(undefined);
            if (confirmAction === "end") await endSession();
            else await restart();
          }}
        />
      )}
    </div>
  );
}
