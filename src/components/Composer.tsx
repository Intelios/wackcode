import { useEffect, useLayoutEffect, useMemo, useRef, useState, type DragEvent, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import type { ImageContent, ProviderRecord, SessionSnapshot, SlashCommand, TaskMode, TaskStatus, ThinkingLevel } from "../types";
import { ACCEPTED_IMAGE_TYPES, attachImages, imageDataUrl, imageFilesFrom } from "../attachment-utils";
import { formatTokens } from "../chat-utils";
import { activeMention, mentionValue, rankMentions, type MentionSuggestion } from "../mention-utils";
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
  /** Resolves false when the send failed; the composer then restores the draft and images.
   *  `queue` is set while a run is in progress: "steer" redirects it at the next boundary,
   *  "follow_up" queues for after it. */
  onSend: (message: string, images: ImageContent[], queue?: "steer" | "follow_up") => Promise<boolean>;
  commands?: SlashCommand[];
  commandsReady?: boolean;
  commandsLoading?: boolean;
  commandsError?: string;
  onRequestCommands?: () => void;
  onCommand?: (name: string, args: string, images: ImageContent[]) => Promise<boolean>;
  onLiteral?: (message: string, images: ImageContent[]) => Promise<boolean>;
  onDraftChange?: (text: string, images: ImageContent[]) => void;
  /** Workspace files for `@` mentions, relative to it; undefined until loaded. */
  mentionFiles?: string[];
  mentionsLoading?: boolean;
  mentionsError?: string;
  mentionsTruncated?: boolean;
  /** Called whenever a new `@` token opens, so the list is fresh. Omit to turn mentions off. */
  onRequestMentions?: () => void;
  transfer?: { text: string; images: ImageContent[]; nonce: number };
  /** Messages queued on the running prompt, shown between the transcript and the draft. */
  queuedMessages?: { steer: string[]; followUp: string[] };
  /** Takes the queued messages back out of Pi; resolves with their texts for the draft. */
  onDequeue?: () => Promise<string[] | undefined>;
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
  /** The persona name in the built-in placeholder and busy copy; absent keeps the historical "Pi". */
  agentName?: string;
}

export function Composer({ status, providerId, modelId, thinkingLevel, providers, stats, header, placeholder, popoverSide = "top", mode, onModeChange, onConfigure, onSend, commands = [], commandsReady, commandsLoading, commandsError, onRequestCommands, onCommand, onLiteral, onDraftChange, mentionFiles, mentionsLoading, mentionsError, mentionsTruncated, onRequestMentions, transfer, queuedMessages, onDequeue, onStop, onOpenSettings, disabled, seed, comet, frozen, agentName = "Pi" }: ComposerProps) {
  const [draft, setDraft] = useState(transfer?.text ?? "");
  const [attachments, setAttachments] = useState<ImageContent[]>(transfer?.images ?? []);
  const [attachNotice, setAttachNotice] = useState<string>();
  const [slashNotice, setSlashNotice] = useState<string>();
  const [slashOpen, setSlashOpen] = useState(transfer?.text.startsWith("/") ?? false);
  const [slashIndex, setSlashIndex] = useState(0);
  const [caret, setCaret] = useState(transfer?.text.length ?? 0);
  const [mentionIndex, setMentionIndex] = useState(0);
  /** Where the `@` token Escape closed starts; it stays closed until that `@` goes away. */
  const [mentionDismissedAt, setMentionDismissedAt] = useState<number>();
  const [dragging, setDragging] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
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
  const mention = onRequestMentions && !showCommands && !disabled && frozen === undefined ? activeMention(draft, caret) : null;
  const showMentions = mention !== null && mention.start !== mentionDismissedAt;
  const mentionQuery = showMentions ? mention.query : undefined;
  const mentionSuggestions = useMemo(
    () => mentionQuery !== undefined && mentionFiles ? rankMentions(mentionFiles, mentionQuery) : [],
    [mentionFiles, mentionQuery]
  );
  const mentionStart = showMentions ? mention.start : undefined;
  useEffect(() => { if (mentionStart !== undefined) onRequestMentions?.(); }, [mentionStart]);
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

  // The composer lives in an overlay layer; the chat view reserves space for it
  // through --composer-h on .workspace.
  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const workspace = wrap?.closest(".workspace");
    if (!wrap || !(workspace instanceof HTMLElement)) return;
    const update = () => workspace.style.setProperty("--composer-h", `${wrap.offsetHeight}px`);
    update();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(update);
    observer?.observe(wrap);
    return () => { observer?.disconnect(); workspace.style.removeProperty("--composer-h"); };
  }, []);

  // Only a new nonce reseeds, so a re-render never clobbers what the user has typed since.
  const seedText = useRef(seed?.text);
  seedText.current = seed?.text;
  const seedNonce = seed?.nonce;
  useEffect(() => {
    if (seedNonce === undefined || seedText.current === undefined) return;
    setDraft(seedText.current);
    areaRef.current?.focus();
  }, [seedNonce]);

  /** Queue the draft on the running prompt. Enter steers (delivered at the run's next
   *  boundary); ⌥Enter and "Send as message" queue it for after the run, raw or expanded. */
  async function queueDraft(behavior: "steer" | "follow_up", literal = false) {
    const message = draft.trim();
    if (!message || status !== "running" || blockedByModel) return;
    const images = attachments;
    setDraft("");
    setAttachments([]);
    setAttachNotice(undefined);
    const ok = literal && onLiteral ? await onLiteral(message, images) : await onSend(message, images, behavior);
    if (!ok) {
      setDraft(message);
      setAttachments(images);
    }
  }

  async function send() {
    if (frozen !== undefined) return;
    const message = draft.trim();
    if (!message || blockedByModel) return;
    if (busy) {
      // Pi is working: queue instead of sending. Slash commands that act on the app itself
      // cannot queue (they are not messages); skills, templates and unknown text can — Pi
      // expands the first two and refuses extension commands.
      if (message.startsWith("/") && commands.some((command) => command.name === /^\/([^\s]+)/.exec(message)?.[1] && command.source === "app")) {
        setSlashNotice(`Wait for ${agentName} to finish before running /${/^\/([^\s]+)/.exec(message)?.[1]}.`);
        setSlashOpen(false);
        return;
      }
      await queueDraft("steer");
      return;
    }
    const images = attachments;
    if (message.startsWith("/") && onCommand) {
      const match = /^\/([^\s]+)(?:\s+([\s\S]*))?$/.exec(message);
      const name = match?.[1] ?? "";
      const args = match?.[2] ?? "";
      // Every command `/` may offer is in `commands` — WackCode's own filtered by Settings, so a
      // switched-off `/copy` reads as unknown like any other name.
      const known = commands.some((command) => command.name === name);
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

  function insertMention(entry: MentionSuggestion) {
    if (!mention) return;
    const value = mentionValue(entry.path, entry.directory);
    const rest = draft.slice(mention.end);
    // A file ends the token; a folder stays open so its contents are suggested next.
    const gap = entry.directory || /^\s/.test(rest) ? "" : " ";
    const next = draft.slice(0, mention.start) + value + gap + rest;
    const position = mention.start + value.length + (entry.directory ? 0 : 1);
    setDraft(next);
    setCaret(position);
    setMentionIndex(0);
    requestAnimationFrame(() => { areaRef.current?.focus(); areaRef.current?.setSelectionRange(position, position); });
  }

  async function sendLiteral() {
    if (!onLiteral) return;
    if (busy) return void queueDraft("steer", true);
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

  /** Take the queued messages back out of Pi and into the draft, keeping anything typed since. */
  async function restoreQueued() {
    const texts = await onDequeue?.();
    if (!texts || texts.length === 0) return;
    setSlashNotice(undefined);
    setDraft((current) => {
      const restored = texts.join("\n\n");
      return current.trim() ? `${current}\n\n${restored}` : restored;
    });
    requestAnimationFrame(() => { areaRef.current?.focus(); });
  }

  const acceptsDrop = (event: DragEvent) => !disabled && providers.length > 0 && event.dataTransfer.types.includes("Files");

  const queuedEntries = [
    ...(queuedMessages?.steer ?? []).map((text) => ({ kind: "steer" as const, text })),
    ...(queuedMessages?.followUp ?? []).map((text) => ({ kind: "followUp" as const, text }))
  ];

  const context = stats?.contextUsage;
  const statsLabel = stats
    ? `${formatTokens(stats.tokens.total)} tokens · ${context?.percent == null ? "?" : Math.round(context.percent)}% context${stats.cost ? ` · $${stats.cost.toFixed(3)}` : ""}`
    : undefined;

  return (
    <div
      ref={wrapRef}
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
      <AnimatePresence initial={false} mode="popLayout">
        {header && <motion.div key="composer-header" className="composer-header" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0, y: -12, transition: { duration: 0.18, ease: EASE } }} transition={{ duration: 0.25, ease: EASE }}>{header}</motion.div>}
      </AnimatePresence>
      <motion.div
        className={`composer${comet ? " comet" : ""}${frozen !== undefined ? " frozen" : ""}`}
        aria-disabled={disabled || undefined}
        layout="position"
        transition={{ duration: 0.55, ease: GLIDE_EASE }}
      >
        {showCommands && <div id="slash-command-list" className="slash-picker" role="listbox" aria-label="Slash commands">
          {commandsLoading ? <div className="slash-picker-status">Loading commands…</div> : commandsError ? <div className="slash-picker-status">{commandsError} <button type="button" onClick={onRequestCommands}>Retry</button></div> : suggestions.length ? suggestions.map((command, index) =>
            <button id={`slash-option-${index}`} type="button" role="option" aria-selected={index === slashIndex} className={`slash-option ${index === slashIndex ? "selected" : ""}`} key={command.id} onMouseDown={(event) => event.preventDefault()} onClick={() => insertCommand(command.name)}>
              <strong>/{command.name}{command.argumentHint ? ` ${command.argumentHint}` : ""}</strong><span>{command.description}</span><small>{command.sourceLabel}</small>
            </button>) : <div className="slash-picker-status">No matching commands</div>}
        </div>}
        {showMentions && <div id="mention-list" className="slash-picker mention-picker" role="listbox" aria-label="Files">
          {mentionsError ? <div className="slash-picker-status">{mentionsError} <button type="button" onClick={onRequestMentions}>Retry</button></div>
            : !mentionFiles ? <div className="slash-picker-status">{mentionsLoading ? "Loading files…" : "No files"}</div>
            : mentionSuggestions.length ? mentionSuggestions.map((entry, index) => {
              const bare = entry.path.replace(/\/$/, "");
              const cut = bare.lastIndexOf("/") + 1;
              return <button id={`mention-option-${index}`} type="button" role="option" aria-selected={index === mentionIndex} className={`slash-option mention-option ${index === mentionIndex ? "selected" : ""}`} key={entry.path} onMouseDown={(event) => event.preventDefault()} onClick={() => insertMention(entry)}>
                <Icon name={entry.directory ? "folder" : "file"} /><strong>{entry.path.slice(cut)}</strong><span>{bare.slice(0, cut)}</span>
              </button>;
            }) : <div className="slash-picker-status">No matching files</div>}
          {mentionFiles && mentionsTruncated && <div className="slash-picker-status">Only the first 20,000 files are listed.</div>}
        </div>}
        {queuedEntries.length > 0 && (
          <div className="composer-queue" role="list" aria-label="Queued messages">
            {queuedEntries.map((entry, index) => (
              <div className="queued-message" role="listitem" key={`${entry.kind}-${index}`}>
                <span className={`queued-tag ${entry.kind}`}>{entry.kind === "steer" ? "Steering" : "Queued"}</span>
                <span className="queued-text">{entry.text}</span>
                <button type="button" className="queued-remove" aria-label="Restore queued messages to the composer" onClick={() => void restoreQueued()}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
          </div>
        )}
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
          aria-controls={showCommands ? "slash-command-list" : showMentions ? "mention-list" : undefined}
          aria-expanded={showCommands || showMentions}
          aria-activedescendant={showCommands && suggestions.length ? `slash-option-${Math.min(slashIndex, suggestions.length - 1)}`
            : showMentions && mentionSuggestions.length ? `mention-option-${Math.min(mentionIndex, mentionSuggestions.length - 1)}` : undefined}
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
            setMentionIndex(0);
            setMentionDismissedAt((at) => at !== undefined && value[at] === "@" ? at : undefined);
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
            if (showMentions && !event.nativeEvent.isComposing) {
              const count = mentionSuggestions.length;
              if (event.key === "ArrowDown" && count) { event.preventDefault(); setMentionIndex((index) => (index + 1) % count); return; }
              if (event.key === "ArrowUp" && count) { event.preventDefault(); setMentionIndex((index) => (index - 1 + count) % count); return; }
              if ((event.key === "Tab" || event.key === "Enter") && count && !event.shiftKey) { event.preventDefault(); insertMention(mentionSuggestions[Math.min(mentionIndex, count - 1)]); return; }
              if (event.key === "Escape") { event.preventDefault(); setMentionDismissedAt(mention.start); return; }
            }
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              if (event.altKey) { void queueDraft("follow_up"); return; }
              void send();
            }
          }}
          placeholder={placeholder ?? (providers.length === 0 ? "Connect a provider to start…" : busy ? `${agentName} is working — ⏎ steers the run, ⌥⏎ queues for after…` : mode === "plan" ? `Describe the work — in Plan mode ${agentName} inspects and proposes a plan without changing files…` : mode === "ultraplan" ? `Describe the work — in Ultra Plan ${agentName} interviews you in depth, one question at a time, before proposing a plan…` : `Ask ${agentName} to inspect, change, or run something…`)}
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
