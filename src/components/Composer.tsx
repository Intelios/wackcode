import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import { motion } from "motion/react";
import type { ImageContent, ProviderRecord, SessionSnapshot, SlashCommand, TaskMode, TaskStatus, ThinkingLevel } from "../types";
import { ACCEPTED_IMAGE_TYPES, attachImages, imageDataUrl, imageFilesFrom } from "../attachment-utils";
import { formatTokens } from "../chat-utils";
import { Icon } from "./Icons";
import { ContextPanel } from "./ContextPanel";
import { ModelPicker, ReasoningToggle } from "./ModelPicker";
import { ModeToggle } from "./ModeToggle";
import { Tooltip } from "./ui/Tooltip";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];
/** Springy settle (same bezier as `.segmented`) — makes the hero→dock glide land with a hint of overshoot. */
const GLIDE_EASE: [number, number, number, number] = [0.34, 1.3, 0.64, 1];

interface ComposerProps {
  status: TaskStatus;
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ThinkingLevel;
  providers: ProviderRecord[];
  stats?: SessionSnapshot["stats"];
  header?: ReactNode;
  placeholder?: string;
  popoverSide?: "top" | "bottom";
  /** The Build/Plan toggle; omit onModeChange to hide it. */
  mode?: TaskMode;
  onModeChange?: (mode: TaskMode) => void;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  /** Resolves false when the send failed; the composer then restores the draft and images. */
  onSend: (message: string, images: ImageContent[]) => Promise<boolean>;
  commands?: SlashCommand[];
  commandsReady?: boolean;
  commandsLoading?: boolean;
  commandsError?: string;
  onRequestCommands?: () => void;
  onCommand?: (name: string, args: string, images: ImageContent[]) => Promise<boolean>;
  onLiteral?: (message: string, images: ImageContent[]) => Promise<boolean>;
  onDraftChange?: (text: string, images: ImageContent[]) => void;
  transfer?: { text: string; images: ImageContent[]; nonce: number };
  onStop: () => void;
  onOpenSettings: () => void;
  /** When true the composer is visually dimmed and non-interactive (e.g. a dialog needs attention). */
  disabled?: boolean;
  /** Replaces the draft whenever `nonce` changes, e.g. with the text of a rewound message. */
  seed?: { text: string; nonce: number };
  /** Traces an ambient accent line around the border; used on the draft hero only. */
  comet?: boolean;
  /** Shows this text read-only instead of the draft while the hero composer hands off to the docked one. */
  frozen?: string;
  /** Shared-layout id so the composer glides between the hero and docked positions. */
  layoutId?: string;
}

export function Composer({ status, providerId, modelId, thinkingLevel, providers, stats, header, placeholder, popoverSide = "top", mode, onModeChange, onConfigure, onSend, commands = [], commandsReady, commandsLoading, commandsError, onRequestCommands, onCommand, onLiteral, onDraftChange, transfer, onStop, onOpenSettings, disabled, seed, comet, frozen, layoutId }: ComposerProps) {
  const [draft, setDraft] = useState(transfer?.text ?? "");
  const [attachments, setAttachments] = useState<ImageContent[]>(transfer?.images ?? []);
  const [attachNotice, setAttachNotice] = useState<string>();
  const [slashNotice, setSlashNotice] = useState<string>();
  const [slashOpen, setSlashOpen] = useState(transfer?.text.startsWith("/") ?? false);
  const [slashIndex, setSlashIndex] = useState(0);
  const [caret, setCaret] = useState(transfer?.text.length ?? 0);
  const [dragging, setDragging] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const transferNonce = useRef(transfer?.nonce);
  const fileRef = useRef<HTMLInputElement>(null);
  const busy = status === "running" || status === "stopping";
  const model = providers.find((provider) => provider.id === providerId)?.models.find((item) => item.id === modelId);
  const vision = model?.vision === true;
  const noVisionMessage = `${model?.name || model?.id || "This model"} doesn't accept images. Turn on Vision for it in Settings.`;
  // Images picked for a vision model, then the model was switched to one without it.
  const blockedByModel = attachments.length > 0 && !vision;
  const tokenEnd = draft.search(/\s/);
  const commandEnd = tokenEnd < 0 ? draft.length : tokenEnd;
  const commandToken = draft.startsWith("/") ? draft.slice(1, commandEnd) : "";
  const suggestions = commands.filter((command) => command.name.toLowerCase().includes(commandToken.toLowerCase()));
  const showCommands = slashOpen && draft.startsWith("/") && caret <= commandEnd && !disabled;
  useEffect(() => {
    if (showCommands && commandsReady === false && !commandsLoading && !commandsError) onRequestCommands?.();
  }, [showCommands, commandsReady, commandsLoading, commandsError]);

  useEffect(() => { onDraftChange?.(draft, attachments); }, [draft, attachments, onDraftChange]);
  useEffect(() => {
    if (!transfer || transferNonce.current === transfer.nonce) return;
    transferNonce.current = transfer.nonce;
    setDraft(transfer.text);
    setAttachments(transfer.images);
    setSlashOpen(transfer.text.startsWith("/"));
  }, [transfer?.nonce]);
  useEffect(() => { if (commandsError && draft.startsWith("/") && !commandsLoading) setSlashNotice(commandsError); }, [commandsError, commandsLoading]);

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 200)}px`;
  }, [draft, frozen]);

  // Only a new nonce reseeds, so a re-render never clobbers what the user has typed since.
  const seedText = useRef(seed?.text);
  seedText.current = seed?.text;
  const seedNonce = seed?.nonce;
  useEffect(() => {
    if (seedNonce === undefined || seedText.current === undefined) return;
    setDraft(seedText.current);
    areaRef.current?.focus();
  }, [seedNonce]);

  async function send() {
    if (frozen !== undefined) return;
    const message = draft.trim();
    if (!message || busy || blockedByModel) return;
    const images = attachments;
    if (message.startsWith("/") && onCommand) {
      const match = /^\/([^\s]+)(?:\s+([\s\S]*))?$/.exec(message);
      const name = match?.[1] ?? "";
      const args = match?.[2] ?? "";
      const known = ["compact", "init", "new", "name", "copy", ...commands.map((command) => command.name)].includes(name);
      if (!known) {
        setSlashNotice(`Unknown command /${name}. You can send it as a message.`);
        setSlashOpen(false);
        return;
      }
      const selected = commands.find((command) => command.name === name);
      if (images.length && (!selected || selected.source === "extension" || selected.source === "app")) {
        setSlashNotice("Remove images before running this command.");
        return;
      }
      try {
        const ok = await onCommand(name, args, images);
        if (ok) { setDraft(""); setAttachments([]); setSlashNotice(undefined); setSlashOpen(false); }
        else setSlashNotice("That command could not run. Try again.");
      } catch (reason) { setSlashNotice(String(reason)); }
      return;
    }
    setDraft("");
    setAttachments([]);
    setAttachNotice(undefined);
    const ok = await onSend(message, images);
    if (!ok) {
      setDraft(message);
      setAttachments(images);
    }
  }

  function insertCommand(name: string) {
    const suffix = draft.slice(commandEnd).trimStart();
    const next = `/${name} ${suffix}`;
    setDraft(next);
    setSlashOpen(false);
    setSlashNotice(undefined);
    requestAnimationFrame(() => { areaRef.current?.focus(); areaRef.current?.setSelectionRange(name.length + 2, name.length + 2); });
  }

  async function sendLiteral() {
    if (!onLiteral || busy) return;
    const ok = await onLiteral(draft.trim(), attachments);
    if (ok) { setDraft(""); setAttachments([]); setSlashNotice(undefined); }
  }

  async function addFiles(files: File[]) {
    if (files.length === 0) return;
    if (!vision) {
      setAttachNotice(noVisionMessage);
      return;
    }
    const result = await attachImages(attachments, files);
    setAttachments(result.images);
    onDraftChange?.(draft, result.images);
    setAttachNotice(result.error);
  }

  function removeAttachment(index: number) {
    const next = attachments.filter((_, itemIndex) => itemIndex !== index);
    setAttachments(next);
    onDraftChange?.(draft, next);
    setAttachNotice(undefined);
  }

  const acceptsDrop = (event: DragEvent) => !disabled && providers.length > 0 && event.dataTransfer.types.includes("Files");

  const context = stats?.contextUsage;
  const statsLabel = stats
    ? `${formatTokens(stats.tokens.total)} tokens · ${context?.percent == null ? "?" : Math.round(context.percent)}% context${stats.cost ? ` · $${stats.cost.toFixed(3)}` : ""}`
    : undefined;

  return (
    <div
      className={`composer-wrap ${disabled ? "disabled" : ""} ${dragging ? "dropping" : ""}`}
      onDragOver={(event) => {
        if (!acceptsDrop(event)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = vision ? "copy" : "none";
        setDragging(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(event) => {
        if (!acceptsDrop(event)) return;
        event.preventDefault();
        setDragging(false);
        void addFiles(imageFilesFrom(event.dataTransfer));
      }}
    >
      {header && <motion.div className="composer-header" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0, y: -12, transition: { duration: 0.18, ease: EASE } }} transition={{ duration: 0.25, ease: EASE }}>{header}</motion.div>}
      <motion.div
        className={`composer${comet ? " comet" : ""}${frozen !== undefined ? " frozen" : ""}`}
        aria-disabled={disabled || undefined}
        layoutId={layoutId}
        layoutCrossfade={false}
        transition={layoutId ? { duration: 0.55, ease: GLIDE_EASE } : undefined}
        exit={{ opacity: 0, y: 24, transition: { duration: 0.22, ease: EASE } }}
      >
        {showCommands && <div id="slash-command-list" className="slash-picker" role="listbox" aria-label="Slash commands">
          {commandsLoading ? <div className="slash-picker-status">Loading commands…</div> : commandsError ? <div className="slash-picker-status">{commandsError} <button type="button" onClick={onRequestCommands}>Retry</button></div> : suggestions.length ? suggestions.map((command, index) =>
            <button id={`slash-option-${index}`} type="button" role="option" aria-selected={index === slashIndex} className={`slash-option ${index === slashIndex ? "selected" : ""}`} key={command.id} onMouseDown={(event) => event.preventDefault()} onClick={() => insertCommand(command.name)}>
              <strong>/{command.name}</strong><span>{command.description}</span><small>{command.sourceLabel}</small>
            </button>) : <div className="slash-picker-status">No matching commands</div>}
        </div>}
        {attachments.length > 0 && (
          <div className="composer-attachments">
            {attachments.map((image, index) => (
              <div className="attachment-thumb" key={index}>
                <img src={imageDataUrl(image)} alt={`Attached image ${index + 1}`} />
                <button type="button" className="attachment-remove" aria-label={`Remove image ${index + 1}`} onClick={() => removeAttachment(index)}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
          </div>
        )}
        {(attachNotice || blockedByModel) && (
          <div className="attachment-notice" role="status">{blockedByModel ? `${noVisionMessage} Or remove the images to send.` : attachNotice}</div>
        )}
        {slashNotice && <div className="attachment-notice" role="status">{slashNotice} <button type="button" onClick={() => void sendLiteral()}>Send as message</button></div>}
        <textarea
          ref={areaRef}
          aria-controls={showCommands ? "slash-command-list" : undefined}
          aria-expanded={showCommands}
          aria-activedescendant={showCommands && suggestions.length ? `slash-option-${Math.min(slashIndex, suggestions.length - 1)}` : undefined}
          value={frozen ?? draft}
          rows={1}
          readOnly={frozen !== undefined}
          onChange={(event) => {
            if (frozen !== undefined) return;
            const value = event.target.value;
            setDraft(value);
            onDraftChange?.(value, attachments);
            setCaret(event.target.selectionStart);
            setSlashIndex(0);
            setSlashNotice(undefined);
            setSlashOpen(value.startsWith("/"));
            if (value.startsWith("/") && !draft.startsWith("/")) onRequestCommands?.();
          }}
          onClick={(event) => setCaret(event.currentTarget.selectionStart)}
          onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
          onPaste={(event) => {
            // Rich-text apps put an image rendition next to copied text; that is a text paste.
            if (event.clipboardData.types.includes("text/plain")) return;
            const files = imageFilesFrom(event.clipboardData);
            if (files.length === 0) return;
            event.preventDefault();
            void addFiles(files);
          }}
          onKeyDown={(event) => {
            if (showCommands && !event.nativeEvent.isComposing) {
              if (event.key === "ArrowDown" && suggestions.length) { event.preventDefault(); setSlashIndex((index) => (index + 1) % suggestions.length); return; }
              if (event.key === "ArrowUp" && suggestions.length) { event.preventDefault(); setSlashIndex((index) => (index - 1 + suggestions.length) % suggestions.length); return; }
              if ((event.key === "Tab" || event.key === "Enter") && suggestions.length && !event.shiftKey) { event.preventDefault(); insertCommand(suggestions[Math.min(slashIndex, suggestions.length - 1)].name); return; }
              if (event.key === "Escape") { event.preventDefault(); setSlashOpen(false); return; }
            }
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={placeholder ?? (providers.length === 0 ? "Connect a provider to start…" : busy ? "Pi is working — queue your next message…" : mode === "plan" ? "Describe the work — in Plan mode Pi inspects and proposes a plan without changing files…" : "Ask Pi to inspect, change, or run something…")}
          disabled={disabled || providers.length === 0}
        />
        <div className="composer-toolbar">
          <div className="composer-left">
            {providers.length === 0 ? (
              <button type="button" className="model-pill callout" onClick={onOpenSettings}>
                <Icon name="key" /> Connect a provider
              </button>
            ) : (
              <>
                <Tooltip label={vision ? "Attach images" : noVisionMessage}>
                  <button
                    type="button"
                    className={`attach-button ${vision ? "" : "unavailable"}`}
                    aria-label="Attach images"
                    aria-disabled={!vision || undefined}
                    disabled={disabled}
                    onClick={() => vision ? fileRef.current?.click() : setAttachNotice(noVisionMessage)}
                  >
                    <Icon name="image" />
                  </button>
                </Tooltip>
                <input
                  ref={fileRef}
                  type="file"
                  accept={ACCEPTED_IMAGE_TYPES.join(",")}
                  multiple
                  hidden
                  data-testid="attach-input"
                  onChange={(event) => {
                    const files = imageFilesFrom(event.target.files);
                    event.target.value = "";
                    void addFiles(files);
                  }}
                />
                <ModelPicker
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                <ReasoningToggle
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  thinkingLevel={thinkingLevel}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                {onModeChange && <ModeToggle mode={mode ?? "build"} disabled={busy} onChange={onModeChange} />}
              </>
            )}
          </div>
          <div className="composer-right">
            {statsLabel && stats && <ContextPanel stats={stats} label={statsLabel} />}
            {busy ? (
              <Tooltip label={status === "stopping" ? "Stopping…" : "Stop"}>
                <button type="button" className="send-button stop" onClick={onStop} disabled={status === "stopping"} aria-label="Stop">
                  <Icon name="stop" />
                </button>
              </Tooltip>
            ) : (
              <Tooltip label="Send (⏎)">
                <button type="button" className="send-button" onClick={() => void send()} disabled={!draft.trim() || blockedByModel} aria-label="Send message">
                  <Icon name="send" />
                </button>
              </Tooltip>
            )}
          </div>
        </div>
      </motion.div>
    </div>
  );
}
