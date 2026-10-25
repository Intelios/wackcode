import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { motion, useReducedMotion } from "motion/react";
import { useFrameTheme, useStagedVisual } from "../hooks/useStagedVisual";
import { MAX_VISUAL_SEND_CHARS, type VisualSendResult } from "../visual-action";
import { parseVisualArgs } from "../tool-utils";
import type { NormalizedBlock } from "../types";
import { Icon } from "./Icons";
import { CopyButton } from "./ui/CopyButton";
import { useNavigationDismiss } from "./ui/NavigationScope";

/** A visual grows to its content between these, then scrolls inside the frame. */
const MIN_HEIGHT = 96;
/** The cap in the transcript; the expanded view lifts it to the window. */
const CARD_MAX_HEIGHT = 640;
/** Frames report their height as soon as they lay out; past this a visual is not coming. */
const LOAD_TIMEOUT_MS = 4_000;
/** One message per visual per second, so a runaway loop cannot fill the queue. */
const SEND_INTERVAL_MS = 1_000;

const SEND_REFUSAL_NOTE: Record<string, string> = {
  stale: "From an earlier reply — ask again to use its actions.",
  busy: "Wait for the reply to finish before using this.",
  throttled: "Give it a moment between messages.",
  invalid: "That message was empty, so nothing was sent.",
  failed: "That message couldn’t be sent."
};

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

interface FrameControls {
  /** Nothing may send: the visual is not on the latest turn, or a reply is running. */
  refuse: "stale" | "busy" | null;
  onSend?: (text: string) => Promise<VisualSendResult>;
  onOpenLink: (href: string) => void;
}

interface VisualFrameProps {
  html: string;
  title: string;
  maxHeight: number;
  controls: FrameControls;
  onRefusal?: (reason: string) => void;
}

/**
 * One visual's document, staged through Rust and drawn in a sandboxed frame.
 *
 * The frame is opaque-origin (`sandbox="allow-scripts"`), so it can neither reach this DOM nor
 * invoke anything in the app; `visuals.rs` serves it under `default-src 'none'`, so it has no
 * network. Everything it says comes back as a `postMessage` this component verifies against the
 * frame's own `contentWindow`: a height, a link the user clicked, and a message it wants to send.
 */
function VisualFrame({ html, title, maxHeight, controls, onRefusal }: VisualFrameProps) {
  const reduce = useReducedMotion();
  const frameRef = useRef<HTMLIFrameElement>(null);
  const { src, failed, fail, retry } = useStagedVisual(html);
  const [height, setHeight] = useState<number>();
  const lastSend = useRef(0);
  // The listener is registered once; these are read at message time.
  const live = useRef(controls);
  useEffect(() => {
    live.current = controls;
  }, [controls]);

  /** Tell the frame how its request ended, so its own button can say what happened. */
  const answer = useCallback((id: number, ok: boolean, reason?: string) => {
    frameRef.current?.contentWindow?.postMessage({ type: "wackcode-visual:send-result", id, ok, ...(reason ? { reason } : {}) }, "*");
  }, []);

  // A freshly staged document lays out again from the minimum.
  useEffect(() => { setHeight(undefined); }, [src]);

  // No height means the document never laid out: show the retry instead of an empty well.
  useEffect(() => {
    if (!src || height !== undefined) return;
    const timer = window.setTimeout(fail, LOAD_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [src, height]);

  useFrameTheme(frameRef, src);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const frame = frameRef.current;
      // Only the frame this card mounted may talk to it; everything else is ignored.
      if (!frame || event.source !== frame.contentWindow) return;
      const data = event.data as { type?: unknown; height?: unknown; id?: unknown; text?: unknown; href?: unknown } | null;
      if (!data || typeof data !== "object") return;
      if (data.type === "wackcode-visual:size" && typeof data.height === "number" && Number.isFinite(data.height)) {
        setHeight(clamp(Math.round(data.height), MIN_HEIGHT, maxHeight));
        return;
      }
      if (data.type === "wackcode-visual:link" && typeof data.href === "string") {
        if (/^https?:\/\//i.test(data.href)) live.current.onOpenLink(data.href);
        return;
      }
      if (data.type !== "wackcode-visual:send" || typeof data.id !== "number" || typeof data.text !== "string") return;
      const id = data.id;
      const text = data.text.trim().slice(0, MAX_VISUAL_SEND_CHARS);
      const refuse = (reason: string) => {
        answer(id, false, reason);
        if (reason !== "throttled") onRefusal?.(reason);
      };
      if (live.current.refuse) return refuse(live.current.refuse);
      if (!text) return refuse("invalid");
      if (Date.now() - lastSend.current < SEND_INTERVAL_MS) return refuse("throttled");
      lastSend.current = Date.now();
      const send = live.current.onSend;
      if (!send) return refuse("failed");
      void send(text)
        .then((result) => {
          if (result.ok) answer(id, true);
          else refuse(result.reason);
        })
        .catch(() => refuse("failed"));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [answer, maxHeight, onRefusal]);

  if (failed) {
    return (
      <div className="visual-frame visual-frame-state">
        <span>This visual couldn’t be shown.</span>
        <button type="button" className="secondary-button compact" onClick={retry}>Try again</button>
      </div>
    );
  }
  return (
    <motion.div
      className={`visual-frame${height === undefined ? " loading" : ""}`}
      initial={false}
      animate={{ height: height ?? MIN_HEIGHT }}
      transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 420, damping: 40 }}
    >
      {src && (
        <iframe
          ref={frameRef}
          className="visual-frame-doc"
          src={src}
          title={title}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
        />
      )}
    </motion.div>
  );
}

/** What a visual card needs from the transcript it is drawn in: whether its buttons may send,
 *  and where a click and a link go. Provided once per transcript (`Transcript`, `ChatArea`). */
export interface VisualScope {
  /** Tool-call ids on the newest answer. Anything older may not send. */
  currentIds: ReadonlySet<string>;
  /** A reply is running, so nothing may send right now. */
  busy: boolean;
  /** Sends the widget's words as the user; the title rides along in the message's tail. */
  onSend?: (text: string, title: string) => Promise<VisualSendResult>;
  onOpenLink?: (href: string) => void;
}

export const VisualCardScope = createContext<VisualScope>({ currentIds: new Set(), busy: false });

/** Settings › Appearance: the card's head band (title, Expand, Copy). Off, a visual is just
 *  its frame in the transcript's well — the actions go with the band. Provided by
 *  `ChatContexts`; the expanded overlay always keeps its own head. */
export const VisualBannerEnabled = createContext(true);

/** A `show_visual` call, drawn as its card. The caller has already checked the result is settled
 *  and successful; arguments that do not parse fall back to the ordinary call row. */
export function VisualCallCard({ block }: { block: NormalizedBlock }) {
  const scope = useContext(VisualCardScope);
  const parsed = parseVisualArgs(block);
  if (!parsed) return null;
  return (
    <VisualCard
      title={parsed.title}
      html={parsed.html}
      current={block.toolCallId !== undefined && scope.currentIds.has(block.toolCallId)}
      busy={scope.busy}
      onSend={scope.onSend}
      onOpenLink={scope.onOpenLink}
    />
  );
}

/** Under a user's message: the words came from a visual they clicked, not from typing. */
export function VisualSentNote({ title }: { title: string }) {
  return (
    <p className="visual-sent">
      <Icon name="visual" aria-hidden="true" />
      <span>Sent from “{title}”</span>
    </p>
  );
}

interface VisualCardProps {
  title: string;
  html: string;
  /** This visual belongs to the latest turn, so its buttons may send. Older ones stay inert. */
  current: boolean;
  /** A reply is running, so nothing may send right now. */
  busy: boolean;
  onSend?: (text: string, title: string) => Promise<VisualSendResult>;
  onOpenLink?: (href: string) => void;
}

/**
 * A visual reply: the agent's small HTML answer — a chart, a comparison, a diagram, a mockup or
 * a tiny interactive widget — drawn where the call landed instead of described in prose.
 *
 * It wears the transcript's well and head band (like a code fence) so it reads as part of the
 * conversation, not an embedded page: the app's title strip, then the frame on a solid content
 * surface under the app's own theme. The document runs sandboxed and offline (`visuals.rs`);
 * the Expand action opens a second, larger frame rather than moving the one on screen, so the
 * card keeps whatever state the inline copy had.
 */
export function VisualCard({ title, html, current, busy, onSend, onOpenLink }: VisualCardProps) {
  const reduce = useReducedMotion();
  const banner = useContext(VisualBannerEnabled);
  const [expanded, setExpanded] = useState(false);
  const [note, setNote] = useState<string>();
  const openLink = useCallback((href: string) => { onOpenLink?.(href); }, [onOpenLink]);
  const controls: FrameControls = {
    refuse: current ? (busy ? "busy" : null) : "stale",
    onSend: onSend ? (text: string) => onSend(text, title) : undefined,
    onOpenLink: openLink
  };

  useEffect(() => {
    if (!note) return;
    const timer = window.setTimeout(() => setNote(undefined), 4_000);
    return () => window.clearTimeout(timer);
  }, [note]);

  const onRefusal = useCallback((reason: string) => {
    const copy = SEND_REFUSAL_NOTE[reason];
    if (copy) setNote(copy);
  }, []);

  return (
    <figure className="visual-card" aria-label={`Visual: ${title}`}>
      {banner && (
      <figcaption className="visual-card-head">
        <span className="visual-card-mark" aria-hidden="true"><Icon name="visual" /></span>
        <span className="visual-card-title">{title}</span>
        <span className="visual-card-actions">
          <button
            type="button"
            className="secondary-button compact visual-card-action"
            aria-label="Expand visual"
            onClick={() => setExpanded(true)}
          >
            <Icon name="expand" /><span>Expand</span>
          </button>
          <CopyButton text={html} label="Copy visual HTML" />
        </span>
      </figcaption>
      )}
      <div className="visual-card-body">
        <VisualFrame html={html} title={title} maxHeight={CARD_MAX_HEIGHT} controls={controls} onRefusal={onRefusal} />
      </div>
      {note && <p className="visual-card-note" role="status">{note}</p>}
      {expanded && (
        <VisualExpanded title={title} html={html} controls={controls} onRefusal={onRefusal} onClose={() => setExpanded(false)} reduce={reduce === true} />
      )}
    </figure>
  );
}

interface VisualExpandedProps {
  title: string;
  html: string;
  controls: FrameControls;
  onRefusal: (reason: string) => void;
  onClose: () => void;
  reduce: boolean;
}

/** The same visual, larger, over the app: a fresh frame at up to 90 % of the window. */
function VisualExpanded({ title, html, controls, onRefusal, onClose, reduce }: VisualExpandedProps) {
  useNavigationDismiss(onClose);
  const [maxHeight, setMaxHeight] = useState(() => Math.round(window.innerHeight * 0.85));

  useEffect(() => {
    const onResize = () => setMaxHeight(Math.round(window.innerHeight * 0.85));
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("resize", onResize);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("resize", onResize);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <motion.div
      className="modal-backdrop visual-expanded-backdrop"
      role="presentation"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: reduce ? 0 : 0.18 }}
      onMouseDown={(event) => event.button === 0 && event.target === event.currentTarget && onClose()}
    >
      <motion.div
        className="visual-expanded"
        role="dialog"
        aria-modal="true"
        aria-label={`Visual: ${title}`}
        initial={reduce ? false : { scale: 0.96, opacity: 0, y: 10 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="visual-card-head">
          <span className="visual-card-mark" aria-hidden="true"><Icon name="visual" /></span>
          <span className="visual-card-title">{title}</span>
          <span className="visual-card-actions">
            <CopyButton text={html} label="Copy visual HTML" />
            <button type="button" className="image-lightbox-close visual-expanded-close" aria-label="Close" onClick={onClose} autoFocus>
              <Icon name="close" />
            </button>
          </span>
        </div>
        <div className="visual-card-body">
          <VisualFrame html={html} title={title} maxHeight={maxHeight} controls={controls} onRefusal={onRefusal} />
        </div>
      </motion.div>
    </motion.div>,
    document.body
  );
}
