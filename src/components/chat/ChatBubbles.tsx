/**
 * Chat mode's speech: the user's accent bubbles, the agent's replies beside its duck, the
 * typing bubble that stands in for the Code transcript's "Working for" clock, the thought
 * caption, and the quiet system and compaction notes. Presentational; `ChatConversation`
 * decides what goes where. Every spring here goes still under reduced motion.
 */
import { memo, useCallback, useContext, useState, type ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { NormalizedBlock, NormalizedMessage } from "../../types";
import { AssistantNameContext } from "../../agentName";
import { splitFileSection, type FileAttachment } from "../../attachment-utils";
import { messageText } from "../../tree-utils";
import { useSmoothText } from "../../hooks/useSmoothText";
import { DuckMark } from "../DuckMark";
import { Icon, type IconName } from "../Icons";
import { Markdown } from "../Markdown";
import { MessageEditor } from "../MessageEditor";
import { PonderingDuck } from "../PonderingDuck";
import { ThinkingPreviewEnabled } from "../ThinkingRow";
import { ImageLightbox } from "../ui/ImageLightbox";
import { Tooltip } from "../ui/Tooltip";
import type { MessageAction, MessageImageLoader } from "../Transcript";
import { ActivityChips, ScratchpadChips } from "./ChatActivity";
import type { ScratchpadFile } from "./chat-scratchpad";

export const SPRING = { type: "spring", stiffness: 420, damping: 30, mass: 0.8 } as const;
export const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];

/** The agent's face: the duck in a disc, pondering while it works. */
export function ChatAvatar({ live = false, thinking = false, className = "" }: { live?: boolean; thinking?: boolean; className?: string }) {
  return (
    <span className={`chat-avatar${live ? " live" : ""}${thinking ? " thinking" : ""} ${className}`} aria-hidden="true">
      {thinking ? <PonderingDuck live /> : <DuckMark />}
    </span>
  );
}

function TrayButton({ icon, label, onClick }: { icon: IconName; label: string; onClick: () => void }) {
  return (
    <Tooltip label={label}>
      <button type="button" className="chat-tray-button" aria-label={label} onClick={onClick}><Icon name={icon} /></button>
    </Tooltip>
  );
}

function CopyTrayButton({ onCopy }: { onCopy: () => Promise<unknown> | void }) {
  const [copied, setCopied] = useState(false);
  return (
    <Tooltip label={copied ? "Copied" : "Copy"}>
      <button type="button" className={`chat-tray-button${copied ? " done" : ""}`} aria-label="Copy"
        onClick={async () => { await onCopy(); setCopied(true); window.setTimeout(() => setCopied(false), 1200); }}>
        <Icon name={copied ? "check" : "copy"} />
      </button>
    </Tooltip>
  );
}

function Versions({ message, disabled, onSwitch }: { message: NormalizedMessage; disabled: boolean; onSwitch: (entryId: string) => void }) {
  const versions = message.versions;
  if (!versions) return null;
  return (
    <div className="chat-versions" role="group" aria-label="Message versions">
      <button type="button" aria-label="Previous version" disabled={disabled || !versions.previous} onClick={() => versions.previous && onSwitch(versions.previous)}><Icon name="back" /></button>
      <span aria-live="polite">{versions.index + 1}/{versions.total}</span>
      <button type="button" aria-label="Next version" disabled={disabled || !versions.next} onClick={() => versions.next && onSwitch(versions.next)}><Icon name="chevron" /></button>
    </div>
  );
}

function FileNote({ file }: { file: FileAttachment }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={`chat-attached-file${open ? " open" : ""}`}>
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <Icon name="file" /><strong>{file.name}</strong>
      </button>
      {open && <pre>{file.text}</pre>}
    </div>
  );
}

function BubbleImages({ images, entryId, loadImage }: { images: NormalizedBlock[]; entryId?: string; loadImage?: MessageImageLoader }) {
  const [shown, setShown] = useState<{ index: number; image: NormalizedBlock } | null>(null);
  const load = useCallback(() => (loadImage && entryId && shown ? loadImage(entryId, shown.index) : Promise.resolve(undefined)), [loadImage, entryId, shown]);
  return (
    <div className="chat-images">
      {images.map((image, index) => image.thumbnail ? (
        <button key={image.imageId ?? index} type="button" className="chat-image" aria-label={`Open attached image ${index + 1}`} onClick={() => setShown({ index, image })}>
          <img src={image.thumbnail} alt={`Attached image ${index + 1}`} />
        </button>
      ) : <span key={image.imageId ?? index} className="chat-image pending" role="img" aria-label={`Attached image ${index + 1}`} />)}
      {shown?.image.thumbnail && <ImageLightbox preview={shown.image.thumbnail} load={load} alt={`Attached image ${shown.index + 1}`} onClose={() => setShown(null)} />}
    </div>
  );
}

interface UserBubbleProps {
  message: NormalizedMessage;
  /** The opening bubble of a run of the user's messages: it gets the tail. */
  first: boolean;
  actionsEnabled: boolean;
  /** This is the latest turn's message: Retry asks again. */
  retry: boolean;
  editing: boolean;
  vision: boolean;
  modelName?: string;
  /** The reply was stopped before it finished. */
  stopped?: boolean;
  onAction: (action: MessageAction | { type: "start-edit"; id: string } | { type: "cancel-edit" }) => Promise<boolean> | void;
  loadImage?: MessageImageLoader;
}

export const UserBubble = memo(function UserBubble({ message, first, actionsEnabled, retry, editing, vision, modelName, stopped, onAction, loadImage }: UserBubbleProps) {
  const reduce = useReducedMotion() ?? false;
  const pending = message.id.startsWith("pending:");
  const images = message.blocks.filter((block) => block.type === "image");
  const { text, files } = splitFileSection(messageText(message));
  const anchor = `message:${message.id}`;
  if (editing) {
    return (
      <div className="chat-row user editing" data-transcript-anchor={anchor}>
        <MessageEditor
          text={text} images={images} files={files} vision={vision} modelName={modelName}
          loadImage={loadImage && message.entryId ? (index) => loadImage(message.entryId!, index) : undefined}
          onCancel={() => void onAction({ type: "cancel-edit" })}
          onSend={async (edited, keptFiles, removeImages) => (await onAction({ type: "edit", message, text: edited, files: keptFiles, removeImages })) === true}
        />
      </div>
    );
  }
  const canAct = actionsEnabled && Boolean(message.entryId);
  return (
    <motion.div
      className={`chat-row user${first ? " first" : ""}${pending ? " sending" : ""}`}
      data-transcript-anchor={anchor}
      // Sending springs the words up out of the composer into their bubble.
      initial={reduce || !pending ? false : { opacity: 0, y: 46, scale: 0.9 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      transition={SPRING}
      style={{ transformOrigin: "100% 100%" }}
    >
      {images.length > 0 && <BubbleImages images={images} entryId={message.entryId} loadImage={loadImage} />}
      {files.length > 0 && <div className="chat-attached">{files.map((file, index) => <FileNote key={index} file={file} />)}</div>}
      {text && <div className="chat-bubble user">{text}</div>}
      <div className="chat-tray end">
        {stopped && <span className="chat-stopped">Stopped</span>}
        <Versions message={message} disabled={!actionsEnabled} onSwitch={(entryId) => void onAction({ type: "switch", entryId })} />
        {text && <CopyTrayButton onCopy={() => onAction({ type: "copy", message })} />}
        {canAct && <TrayButton icon="pencil" label="Edit" onClick={() => void onAction({ type: "start-edit", id: message.id })} />}
        {canAct && retry && <TrayButton icon="refresh" label="Ask again" onClick={() => void onAction({ type: "retry", message })} />}
      </div>
    </motion.div>
  );
});

/** Text that types itself in while it streams. */
function StreamingProse({ text }: { text: string }) {
  const shown = useSmoothText(text, true);
  return <Markdown streaming>{shown}</Markdown>;
}

function ThoughtCaption({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion() ?? false;
  return (
    <div className={`chat-thought${open ? " open" : ""}`}>
      <button type="button" className="chat-thought-toggle" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <PonderingDuck live={false} className="chat-thought-duck" />
        <span>{open ? "Hide the thinking" : "Thought it over"}</span>
        <Icon name="chevron" className="chat-thought-chevron" />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div className="chat-thought-body" initial={reduce ? false : { opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, height: 0 }} transition={{ duration: 0.26, ease: EASE }}>
            <div className="chat-thought-text">{text}</div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** A reply's blocks laid out as speech: thoughts, chips for tool runs, then prose bubbles. */
type Piece =
  | { type: "thought"; key: string; text: string }
  | { type: "tools"; key: string; calls: NormalizedBlock[] }
  | { type: "text"; key: string; text: string; streaming: boolean }
  | { type: "error"; key: string; text: string };

export function replyPieces(messages: NormalizedMessage[], partial?: NormalizedMessage): Piece[] {
  const pieces: Piece[] = [];
  // Keys are ordinal within the reply, never message ids: the streamed partial and the message
  // the worker saves for it differ in id, and a bubble must not remount (and pop again) then.
  let ordinal = 0;
  const add = (message: NormalizedMessage, streaming: boolean) => {
    message.blocks.forEach((block) => {
      const key = `${block.type}:${ordinal++}`;
      if (block.type === "thinking") {
        // Only settled reasoning gets a caption; live reasoning shows in the typing bubble.
        if (block.text?.trim() && !(streaming && block.durationMs === undefined)) {
          const last = pieces[pieces.length - 1];
          if (last?.type === "thought") last.text += `\n\n${block.text.trim()}`;
          else pieces.push({ type: "thought", key, text: block.text.trim() });
        }
      } else if (block.type === "tool-call") {
        const last = pieces[pieces.length - 1];
        if (last?.type === "tools") last.calls.push(block);
        else pieces.push({ type: "tools", key, calls: [block] });
      } else if (block.type === "text" && block.text?.trim()) {
        pieces.push({ type: "text", key, text: block.text, streaming });
      }
    });
    if (message.stopReason === "error") pieces.push({ type: "error", key: `error:${ordinal++}`, text: message.errorMessage || "The provider turned the request down." });
  };
  for (const message of messages) if (message.role === "assistant") add(message, false);
  if (partial) add(partial, true);
  return pieces;
}

interface AgentReplyProps {
  pieces: Piece[];
  results: Map<string, NormalizedBlock>;
  /** The first reply of a run of the agent's: it carries the name. */
  first: boolean;
  live: boolean;
  files: ScratchpadFile[];
  copyText?: string;
  /** Offered on the latest reply: ask the same question again. */
  onRetry?: () => void;
  onCopy: (text: string) => Promise<unknown> | void;
  onReveal: (path: string) => void;
  /** The typing bubble, while the reply has more to say. */
  typing?: ReactNode;
  anchor: string;
}

export function AgentReply({ pieces, results, first, live, files, copyText, onRetry, onCopy, onReveal, typing, anchor }: AgentReplyProps) {
  const name = useContext(AssistantNameContext);
  const reduce = useReducedMotion() ?? false;
  const hasText = pieces.some((piece) => piece.type === "text");
  return (
    <div className={`chat-row agent${first ? " first" : ""}${live ? " live" : ""}`} data-transcript-anchor={anchor}>
      <ChatAvatar live={live} />
      <div className="chat-reply">
        {first && <div className="chat-name">{name}</div>}
        {pieces.map((piece) => {
          if (piece.type === "thought") return <ThoughtCaption key={piece.key} text={piece.text} />;
          if (piece.type === "tools") return <ActivityChips key={piece.key} calls={piece.calls} results={results} live={live} />;
          if (piece.type === "error") return <div key={piece.key} className="chat-bubble agent error" role="alert"><Icon name="close" /> {piece.text}</div>;
          return (
            <motion.div key={piece.key} className="chat-bubble agent"
              // A reply pops out of the duck's beak: from the avatar's corner.
              initial={reduce || !live ? false : { opacity: 0, scale: 0.86, x: -10 }}
              animate={{ opacity: 1, scale: 1, x: 0 }} transition={SPRING} style={{ transformOrigin: "0% 0%" }}>
              {piece.streaming ? <StreamingProse text={piece.text} /> : <Markdown>{piece.text}</Markdown>}
            </motion.div>
          );
        })}
        {typing}
        {!live && <ScratchpadChips files={files} onReveal={onReveal} />}
        {!live && (hasText || onRetry) && (
          <div className="chat-tray start">
            {copyText && <CopyTrayButton onCopy={() => onCopy(copyText)} />}
            {onRetry && <TrayButton icon="refresh" label="Try again" onClick={onRetry} />}
          </div>
        )}
      </div>
    </div>
  );
}

/** What the duck is up to while it works. */
export type TypingMood = "typing" | "thinking" | "tidying" | "using";

export function TypingBubble({ mood, thought }: { mood: TypingMood; thought?: string }) {
  const reduce = useReducedMotion() ?? false;
  const preview = useContext(ThinkingPreviewEnabled);
  const caption = mood === "thinking" ? "pondering…" : mood === "tidying" ? "tidying up the conversation…" : mood === "using" ? "on it…" : undefined;
  const tail = preview && mood === "thinking" && thought ? thought.replace(/\s+/g, " ").trim().slice(-90) : "";
  return (
    <motion.div
      className={`chat-typing ${mood}`}
      role="status"
      aria-label={caption ? caption.replace("…", "") : "Typing"}
      layout={!reduce}
      initial={reduce ? false : { opacity: 0, scale: 0.6, x: -8 }}
      animate={{ opacity: 1, scale: 1, x: 0 }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.7, transition: { duration: 0.14 } }}
      transition={SPRING}
      style={{ transformOrigin: "0% 50%" }}
    >
      <span className="chat-dots" aria-hidden="true"><i /><i /><i /></span>
      {caption && <span className="chat-typing-caption">{caption}</span>}
      {tail && <span className="chat-typing-tail" aria-hidden="true">{tail}</span>}
    </motion.div>
  );
}

/** A standalone typing row: the duck has started but hasn't said anything yet. */
export function TypingRow({ mood, thought, first }: { mood: TypingMood; thought?: string; first: boolean }) {
  const name = useContext(AssistantNameContext);
  return (
    <div className={`chat-row agent live${first ? " first" : ""}`} data-transcript-anchor="typing">
      <ChatAvatar live thinking={mood === "thinking"} />
      <div className="chat-reply">
        {first && <div className="chat-name">{name}</div>}
        <TypingBubble mood={mood} thought={thought} />
      </div>
    </div>
  );
}

export function TimeDivider({ label }: { label: string }) {
  return <div className="chat-divider" role="separator"><span>{label}</span></div>;
}

export function CompactionNote({ message }: { message: NormalizedMessage }) {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion() ?? false;
  return (
    <div className="chat-compaction" data-transcript-anchor={`message:${message.id}`}>
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        <Icon name="spark" /> Tidied up the conversation <Icon name="chevron" className="chat-thought-chevron" />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div className="chat-compaction-body" initial={reduce ? false : { opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, height: 0 }} transition={{ duration: 0.24, ease: EASE }}>
            <p>Earlier messages were folded into this summary so the chat can keep going.</p>
            <Markdown>{message.compaction?.summary ?? ""}</Markdown>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

export interface SystemNote {
  key: string;
  tone: "error" | "warning" | "info";
  text: string;
  action?: { label: string; run: () => void };
  onDismiss?: () => void;
}

export function SystemBubble({ note }: { note: SystemNote }) {
  const reduce = useReducedMotion() ?? false;
  return (
    <motion.div className={`chat-system ${note.tone}`} role={note.tone === "error" ? "alert" : "status"}
      initial={reduce ? false : { opacity: 0, y: 8, scale: 0.96 }} animate={{ opacity: 1, y: 0, scale: 1 }} transition={SPRING}>
      <span className="chat-system-mark" aria-hidden="true">{note.tone === "info" ? "i" : "!"}</span>
      <span className="chat-system-text">{note.text}</span>
      {note.action && <button type="button" className="secondary-button compact" onClick={note.action.run}>{note.action.label}</button>}
      {note.onDismiss && <button type="button" className="ghost-button chat-system-dismiss" aria-label="Dismiss" onClick={note.onDismiss}><Icon name="close" /></button>}
    </motion.div>
  );
}
