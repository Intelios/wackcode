import { encodeContexts, restoreContexts } from "../../browser-context";
/**
 * Chat mode's composer: a round, friendly pill instead of the Code composer's toolbar. It keeps
 * what a conversation needs — attachments, the model and reasoning pickers, Stop, and queued
 * messages with Steer — and leaves out slash commands, workspace file mentions, planning modes
 * and stats. Its `@` picker is chats only: Chat mode may reference other Chat mode chats, and has
 * no workspace files to point into (`chat-refs.ts`).
 *
 * Drafts live in App (`useComposerDrafts`) and every async update captures its draft key, as
 * in Composer.tsx: a late send failure or queue restore writes back to the chat it came from.
 * The attachment tray and queued rows are the shared pieces in `../composer/`.
 */
import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type DragEvent, type SetStateAction } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type { ImageContent, ProviderRecord, QueuedMessage, RestoredPrompt, TaskStatus, ThinkingLevel } from "../../types";
import { attachmentProblem, attachFiles, filesFrom, largePasteText, pastedTextFile, splitFileSection, type FileAttachment } from "../../attachment-utils";
import { activeMention, type ChatMentionSuggestion, type MentionSuggestion } from "../../mention-utils";
import { chatMentionToken, encodeChatRefs, rankChats, survivingChatRefs } from "../../chat-refs";
import { EMPTY_DRAFT, type ComposerDraft, type ComposerDraftState } from "../../hooks/useComposerDrafts";
import { AttachmentTray } from "../composer/AttachmentTray";
import { PasteNotice } from "../composer/PasteNotice";
import { QueuedRows, queuedDisplay, useQueueActions } from "../composer/QueuedRows";
import { Icon } from "../Icons";
import { ModelPicker, ReasoningToggle, type ModelFavoritesProps } from "../ModelPicker";
import { ImageLightbox } from "../ui/ImageLightbox";
import { Tooltip } from "../ui/Tooltip";
import { MentionPicker } from "../ui/MentionPicker";
import { ChatAvatar } from "./ChatBubbles";

/** Same overshooting glide as the Code composer's hero→dock move. */
const GLIDE_EASE: [number, number, number, number] = [0.34, 1.3, 0.64, 1];

export interface ChatComposerHandle {
  focus: () => void;
  /** Puts text in the draft (a hero idea) and focuses it, caret at the end. */
  fill: (text: string) => void;
}

interface ChatComposerProps extends ModelFavoritesProps {
  draftState: ComposerDraftState;
  status: TaskStatus;
  /** The parent has settled; sends start alongside remaining background work. */
  backgroundWorking?: boolean;
  providers: ProviderRecord[];
  providerId?: string;
  modelId?: string;
  thinkingLevel?: ThinkingLevel;
  placeholder: string;
  agentName: string;
  disabled?: boolean;
  /** The hero hands the sent text to the conversation; the pill holds it still meanwhile. */
  frozen?: string;
  hero: boolean;
  queuedMessages?: QueuedMessage[];
  seed?: { text: string; images?: ImageContent[]; nonce: number };
  /** Chats the `@` picker offers: this chat excluded, Chat mode chats only. */
  mentionChats?: ChatMentionSuggestion[];
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  onSend: (message: string, images: ImageContent[], files: FileAttachment[], queue?: boolean) => Promise<boolean>;
  onSteer?: (messageId: string) => Promise<boolean>;
  onDequeue?: () => Promise<(RestoredPrompt | string)[] | undefined>;
  onStop: () => void;
  onOpenSettings: () => void;
  /** Reports whether there's something typed, so the hero's duck can perk up. */
  onTyping?: (typing: boolean) => void;
}

export const ChatComposer = forwardRef<ChatComposerHandle, ChatComposerProps>(function ChatComposer({
  draftState, status, backgroundWorking = false, providers, providerId, modelId, thinkingLevel, favoriteModels, favoriteSaving, onSetFavorite,
  placeholder, agentName, disabled, frozen, hero, queuedMessages, seed, mentionChats, onConfigure, onSend, onSteer, onDequeue, onStop, onOpenSettings, onTyping
}, handle) {
  const reduce = useReducedMotion() ?? false;
  const { key: draftKey, value, update: updateDraft } = draftState;
  const { text: draft, images, files } = value;
  const contexts = value.browserContexts ?? [];
  const chatRefs = value.chatRefs ?? [];
  const attachmentError = attachmentProblem(value);
  const activeDraftKey = useRef(draftKey);
  activeDraftKey.current = draftKey;
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const seedNonces = useRef(new Map<string, number>());
  const [notice, setNotice] = useState<string>();
  /** The pasted text that became an attachment, while its notice offers the way back. */
  const [pasteNotice, setPasteNotice] = useState<FileAttachment>();
  const [dragging, setDragging] = useState(false);
  const [shownImage, setShownImage] = useState<{ index: number; url: string } | null>(null);
  /** Bumped on each send: the send button does a little hop. */
  const [launches, setLaunches] = useState(0);
  const [caret, setCaret] = useState(draft.length);
  const [mentionIndex, setMentionIndex] = useState(0);
  /** Where the `@` token Escape closed starts; it stays closed until that `@` goes away. */
  const [mentionDismissedAt, setMentionDismissedAt] = useState<number>();

  function setField<K extends keyof ComposerDraft>(field: K, next: SetStateAction<ComposerDraft[K]>) {
    updateDraft((current) => ({ ...current, [field]: typeof next === "function" ? (next as (v: ComposerDraft[K]) => ComposerDraft[K])(current[field]) : next }));
  }

  const busy = status === "running" || status === "stopping";
  const queueing = busy && !backgroundWorking;
  const model = providers.find((provider) => provider.id === providerId)?.models.find((item) => item.id === modelId);
  const vision = model?.vision === true;
  const noVisionMessage = `${model?.name || model?.id || "This model"} doesn't accept images. Turn on Vision for it in Settings.`;
  const blockedByModel = (images.length + contexts.length > 0 && !vision) || Boolean(attachmentError);
  const shownText = frozen ?? draft;
  const mention = !disabled && frozen === undefined ? activeMention(draft, caret) : null;
  const showMentions = mention !== null && mention.start !== mentionDismissedAt;
  const mentionSuggestions = useMemo(
    () => showMentions && mentionChats?.length ? rankChats(mentionChats, mention.query) : [],
    [mentionChats, showMentions, mention?.query]
  );
  const canSend = !disabled && frozen === undefined && providers.length > 0 && status !== "stopping" && (Boolean(draft.trim()) || files.length > 0 || contexts.length > 0) && !blockedByModel;

  const queue = useQueueActions({
    draftKey, activeDraftKey, status, blocked: Boolean(disabled) || frozen !== undefined, onSteer, onDequeue,
    onError: setNotice,
    onRestore: (restored, restoredFiles, restoredImages, restoredContexts, restoredChatRefs) => {
      updateDraft((current) => ({
        ...current,
        images: [...current.images, ...restoredImages],
        chatRefs: [...(current.chatRefs ?? []), ...restoredChatRefs].filter((ref, index, all) => all.findIndex((item) => item.id === ref.id) === index),
        browserContexts: [...(current.browserContexts ?? []), ...restoredContexts]
      }));
      if (restoredFiles.length) setField("files", (current) => [...current, ...restoredFiles]);
      if (restored) setField("text", (current) => current.trim() ? `${current}\n\n${restored}` : restored);
      requestAnimationFrame(() => { if (activeDraftKey.current === draftKey) areaRef.current?.focus(); });
    }
  });

  useImperativeHandle(handle, () => ({
    focus: () => areaRef.current?.focus(),
    fill: (text: string) => {
      updateDraft((current) => ({ ...current, text }));
      setCaret(text.length);
      requestAnimationFrame(() => {
        const area = areaRef.current;
        if (!area) return;
        area.focus();
        area.setSelectionRange(text.length, text.length);
      });
    }
  }), [updateDraft]);

  useEffect(() => { onTyping?.(Boolean(draft.trim() || files.length)); }, [draft, files.length, onTyping]);
  useLayoutEffect(() => { setNotice(undefined); setPasteNotice(undefined); setShownImage(null); setDragging(false); setCaret(draft.length); setMentionIndex(0); setMentionDismissedAt(undefined); }, [draftKey]);

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 220)}px`;
  }, [shownText]);

  // The conversation reserves room for the docked pill through --composer-h.
  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const host = wrap?.closest(".chat-area");
    if (!wrap || !(host instanceof HTMLElement)) return;
    const update = () => host.style.setProperty("--composer-h", `${wrap.offsetHeight}px`);
    update();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(update);
    observer?.observe(wrap);
    return () => { observer?.disconnect(); host.style.removeProperty("--composer-h"); };
  }, []);

  const seedText = useRef(seed?.text);
  const seedImages = useRef(seed?.images);
  seedImages.current = seed?.images;
  seedText.current = seed?.text;
  useEffect(() => {
    const nonce = seed?.nonce;
    if (nonce === undefined || seedText.current === undefined || seedNonces.current.get(draftKey) === nonce) return;
    seedNonces.current.set(draftKey, nonce);
    const restored = splitFileSection(seedText.current);
    updateDraft((current) => ({ ...current, text: restored.text, files: restored.files, chatRefs: restored.chatRefs ?? [], ...restoreContexts(restored.browserContexts ?? [], seedImages.current ?? []) }));
    setPasteNotice(undefined);
    areaRef.current?.focus();
  }, [seed?.nonce, draftKey]);

  async function send() {
    if (!canSend) return;
    const message = draft.trim();
    const keptImages = images;
    const keptFiles = files;
    updateDraft(EMPTY_DRAFT);
    setNotice(undefined);
    setPasteNotice(undefined);
    setLaunches((count) => count + 1);
    const outgoing = encodeContexts(message, keptImages, contexts);
    // Referenced chats ride a tail section after the browser contexts, never as content.
    const text = encodeChatRefs(outgoing.text, survivingChatRefs(message, chatRefs));
    const ok = await onSend(text, outgoing.images, keptFiles, queueing ? true : undefined);
    if (!ok) {
      updateDraft((current) => ({
        text: current.text ? `${message}\n\n${current.text}` : message,
        images: [...keptImages, ...current.images],
        files: [...keptFiles, ...current.files],
        chatRefs,
        browserContexts: [...contexts, ...(current.browserContexts ?? [])]
      }));
    }
  }

  /** Insert a referenced chat where the `@` token is and remember its id for the tail section. */
  function insertMention(entry: MentionSuggestion) {
    if (!mention || entry.kind !== "chat") return;
    const value = chatMentionToken(entry.name);
    const rest = draft.slice(mention.end);
    const gap = /^\s/.test(rest) ? "" : " ";
    const next = draft.slice(0, mention.start) + value + gap + rest;
    const position = mention.start + value.length + 1;
    const ref = { id: entry.id, name: entry.name };
    setField("chatRefs", (current) => [...(current ?? []).filter((item) => item.id !== ref.id), ref]);
    setField("text", next);
    setCaret(position);
    setMentionIndex(0);
    requestAnimationFrame(() => { if (activeDraftKey.current !== draftKey) return; areaRef.current?.focus(); areaRef.current?.setSelectionRange(position, position); });
  }

  async function addFiles(picked: File[]) {
    if (picked.length === 0) return;
    const result = await attachFiles({ images, files }, picked, { vision, noVisionMessage, mentions: false, reservedImages: contexts.length });
    setField("images", result.images);
    setField("files", result.files);
    if (activeDraftKey.current === draftKey) setNotice(result.error);
  }

  /** Put a pasted attachment's text back in the draft, where the notice's button asks for it. */
  function insertPastedText() {
    if (!pasteNotice || disabled || frozen !== undefined) return;
    const { name, text } = pasteNotice;
    const area = areaRef.current;
    const start = area?.selectionStart ?? draft.length;
    const end = area?.selectionEnd ?? start;
    const caretAfter = start + text.length;
    setField("text", draft.slice(0, start) + text + draft.slice(end));
    setField("files", files.filter((file) => file.name !== name));
    setPasteNotice(undefined);
    setNotice(undefined);
    requestAnimationFrame(() => {
      if (activeDraftKey.current !== draftKey) return;
      areaRef.current?.focus();
      areaRef.current?.setSelectionRange(caretAfter, caretAfter);
    });
  }

  const acceptsDrop = (event: DragEvent) => !disabled && providers.length > 0 && event.dataTransfer.types.includes("Files");
  const queued = queuedDisplay(queuedMessages);
  const queueDisabled = Boolean(disabled) || frozen !== undefined || status === "stopping" || queue.pending;

  return (
    <div
      ref={wrapRef}
      className={`chat-composer-wrap${hero ? " hero" : ""}${disabled ? " disabled" : ""}${dragging ? " dropping" : ""}`}
      onDragOver={(event) => { if (!acceptsDrop(event)) return; event.preventDefault(); event.dataTransfer.dropEffect = "copy"; setDragging(true); }}
      onDragLeave={(event) => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false); }}
      onDrop={(event) => { if (!acceptsDrop(event)) return; event.preventDefault(); setDragging(false); void addFiles(filesFrom(event.dataTransfer)); }}
    >
      <QueuedRows entries={queued} agentName={agentName} disabled={queueDisabled} className="chat-queue"
        tag={<span className="chat-queue-tag" aria-hidden="true"><Icon name="clock" /></span>}
        canSteer={status === "running" && Boolean(onSteer)} canRestore={Boolean(onDequeue)}
        onSteer={(id) => void queue.steer(id)} onRestore={() => void queue.restore()} />
      <motion.div
        className={`chat-composer${frozen !== undefined ? " frozen" : ""}${draft.trim() ? " has-text" : ""}`}
        aria-disabled={disabled || undefined}
        layout={reduce ? false : "position"}
        transition={{ duration: 0.6, ease: GLIDE_EASE }}
      >
        <AttachmentTray images={images} files={files} browserContexts={contexts} onRemoveContext={(index) => setField("browserContexts", contexts.filter((_, i) => i !== index))} className="chat-attachments"
          onOpenImage={(index, url) => setShownImage({ index, url })}
          onRemoveImage={(index) => { setField("images", images.filter((_, i) => i !== index)); setNotice(undefined); }}
          onRemoveFile={(index) => {
            const next = files.filter((_, i) => i !== index);
            setField("files", next);
            setNotice(undefined);
            if (next.every((file) => file.name !== pasteNotice?.name)) setPasteNotice(undefined);
          }} />
        <AnimatePresence initial={false}>
          {pasteNotice && (
            <motion.div key="paste" className="chat-composer-notice paste-notice" role="status" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
              <PasteNotice file={pasteNotice} onInsert={insertPastedText} disabled={disabled || frozen !== undefined} />
            </motion.div>
          )}
          {(notice || blockedByModel) && (
            <motion.div key="notice" className="chat-composer-notice" role="status" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
              {attachmentError || (blockedByModel ? `${noVisionMessage} Or remove the images to send.` : notice)}
            </motion.div>
          )}
        </AnimatePresence>
        {showMentions && <MentionPicker id="chat-mention-list" label="Chats" suggestions={mentionSuggestions} index={mentionIndex} onPick={insertMention}
          empty={mentionChats?.length ? "No matching chats" : "No other chats to reference"} />}
        <div className="chat-composer-row">
          {providers.length > 0 && (
            <Tooltip label="Attach files or images">
              <button type="button" className="chat-round-button attach" aria-label="Attach files" disabled={disabled} onClick={() => fileRef.current?.click()}>
                <Icon name="plus" />
              </button>
            </Tooltip>
          )}
          <input ref={fileRef} type="file" multiple hidden data-testid="chat-attach-input"
            onChange={(event) => { const picked = filesFrom(event.target.files); event.target.value = ""; void addFiles(picked); }} />
          <textarea
            ref={areaRef}
            aria-label="Message"
            rows={1}
            value={shownText}
            readOnly={frozen !== undefined}
            disabled={disabled || providers.length === 0}
            aria-controls={showMentions ? "chat-mention-list" : undefined}
            aria-expanded={showMentions}
            aria-activedescendant={showMentions && mentionSuggestions.length ? `mention-option-${Math.min(mentionIndex, mentionSuggestions.length - 1)}` : undefined}
            placeholder={providers.length === 0 ? "Connect a provider to start…" : status === "stopping" ? `${agentName} is stopping…` : queueing ? `${agentName} is replying — ⏎ sends after` : placeholder}
            onChange={(event) => {
              if (frozen !== undefined) return;
              const value = event.target.value;
              setField("text", value);
              setCaret(event.target.selectionStart);
              setMentionIndex(0);
              setMentionDismissedAt((at) => at !== undefined && value[at] === "@" ? at : undefined);
              setNotice(undefined);
            }}
            onClick={(event) => setCaret(event.currentTarget.selectionStart)}
            onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
            onPaste={(event) => {
              if (disabled || frozen !== undefined) return;
              // A paste this large would swamp the draft; it becomes a text file like a picked one.
              const pasted = largePasteText(event.clipboardData);
              if (pasted !== undefined) {
                const plan = pastedTextFile(pasted, files);
                if ("file" in plan) {
                  event.preventDefault();
                  const file = plan.file;
                  setField("files", [...files, file]);
                  setPasteNotice(file);
                  setNotice(undefined);
                } else {
                  // The caps refused it: leave the insertion to the browser and say where it went.
                  setPasteNotice(undefined);
                  setNotice(`${plan.error} The paste went into the draft.`);
                }
                return;
              }
              if (event.clipboardData.types.includes("text/plain")) return;
              const picked = filesFrom(event.clipboardData);
              if (picked.length === 0) return;
              event.preventDefault();
              void addFiles(picked);
            }}
            onKeyDown={(event) => {
              if (showMentions && !event.nativeEvent.isComposing) {
                const count = mentionSuggestions.length;
                if (event.key === "ArrowDown" && count) { event.preventDefault(); setMentionIndex((index) => (index + 1) % count); return; }
                if (event.key === "ArrowUp" && count) { event.preventDefault(); setMentionIndex((index) => (index - 1 + count) % count); return; }
                if ((event.key === "Tab" || event.key === "Enter") && count && !event.shiftKey) { event.preventDefault(); insertMention(mentionSuggestions[Math.min(mentionIndex, count - 1)]); return; }
                if (event.key === "Escape") { event.preventDefault(); setMentionDismissedAt(mention.start); return; }
              }
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void send(); }
            }}
          />
          <div className="chat-composer-actions">
            <AnimatePresence initial={false} mode="popLayout">
              {busy && (
                <motion.span key="stop" initial={reduce ? false : { scale: 0.4, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={{ scale: 0.4, opacity: 0 }} transition={{ type: "spring", stiffness: 520, damping: 26 }}>
                  <Tooltip label={status === "stopping" ? "Stopping…" : "Stop"}>
                    <button type="button" className="chat-round-button stop" aria-label="Stop" onClick={onStop} disabled={disabled || status === "stopping"}>
                      <Icon name="stop" />
                    </button>
                  </Tooltip>
                </motion.span>
              )}
            </AnimatePresence>
            <Tooltip label={queueing ? `Send after ${agentName} finishes (⏎)` : "Send (⏎)"}>
              <button type="button" className={`chat-round-button send${canSend ? " ready" : ""}${queueing ? " queue" : ""}`}
                aria-label={queueing ? "Queue message" : "Send message"} disabled={!canSend} onClick={() => void send()}>
                <motion.span key={launches} className="chat-send-glyph"
                  initial={reduce || launches === 0 ? false : { y: 14, x: -6, opacity: 0, rotate: -20 }}
                  animate={{ y: 0, x: 0, opacity: 1, rotate: 0 }} transition={{ type: "spring", stiffness: 460, damping: 22, delay: 0.08 }}>
                  <Icon name="send" />
                </motion.span>
              </button>
            </Tooltip>
          </div>
        </div>
        <div className="chat-composer-foot">
          {providers.length === 0 ? (
            <button type="button" className="model-pill callout" onClick={onOpenSettings}><Icon name="key" /> Connect a provider</button>
          ) : (
            <>
              <ChatAvatar className="chat-composer-avatar" />
              <ModelPicker providers={providers} favoriteModels={favoriteModels} favoriteSaving={favoriteSaving} onSetFavorite={onSetFavorite}
                providerId={providerId} modelId={modelId} disabled={busy} popoverSide={hero ? "bottom" : "top"} onConfigure={onConfigure} />
              <ReasoningToggle providers={providers} providerId={providerId} modelId={modelId} thinkingLevel={thinkingLevel}
                disabled={busy} popoverSide={hero ? "bottom" : "top"} onConfigure={onConfigure} />
            </>
          )}
        </div>
      </motion.div>
      {shownImage && <ImageLightbox preview={shownImage.url} alt={`Attached image ${shownImage.index + 1}`} onClose={() => setShownImage(null)} />}
    </div>
  );
});
