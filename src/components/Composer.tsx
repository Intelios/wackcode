import { useEffect, useLayoutEffect, useMemo, useRef, useState, type DragEvent, type ReactNode, type SetStateAction } from "react";
import { AnimatePresence, motion } from "motion/react";
import { DEFAULT_EXECUTION_POLICY } from "../execution-policy";
import { DEFAULT_AGENT_NAME } from "../agentName";
import { ExecutionPolicyNotice } from "./ExecutionPolicyNotice";
import type { ExecutionPolicyConfig, ImageContent, ProviderRecord, QueuedMessage, SessionSnapshot, SlashCommand, TaskMode, TaskStatus, ThinkingLevel } from "../types";
import { attachFiles, filesFrom, imageDataUrl, splitFileSection, type FileAttachment } from "../attachment-utils";
import { activeMention, mentionValue, rankMentions, type MentionSuggestion } from "../mention-utils";
import { activeSlashCommand } from "../command-utils";
import { EMPTY_DRAFT, type ComposerDraft, type ComposerDraftState } from "../hooks/useComposerDrafts";
import { Icon } from "./Icons";
import { ContextPanel } from "./ContextPanel";
import { ModelPicker, ReasoningToggle, type ModelFavoritesProps } from "./ModelPicker";
import { ModeToggle } from "./ModeToggle";
import { Tooltip } from "./ui/Tooltip";
import { ImageLightbox } from "./ui/ImageLightbox";

/** Matches `--ease` in styles.css. */
const EASE: [number, number, number, number] = [0.33, 1, 0.68, 1];
/** Springy settle (same bezier as `.segmented`) — makes the hero→dock glide land with a hint of overshoot. */
const GLIDE_EASE: [number, number, number, number] = [0.34, 1.3, 0.64, 1];

/**
 * The composer's inline hint once a command that takes arguments is selected: the hint plus a
 * short qualifier, or undefined when there's nothing to say. `<arg>` reads required, `[arg]`
 * optional — the convention `command-utils.ts` documents. Skills get their own note because
 * their arguments are appended after the body rather than substituted in (worker `index.ts`).
 */
export function argHintNote(command: SlashCommand | undefined): string | undefined {
  const hint = command?.argumentHint?.trim();
  if (!command || !hint) return undefined;
  const qualifier = command.source === "skill"
    ? "arguments are appended after the skill"
    : hint.startsWith("<") ? "required argument"
    : hint.startsWith("[") ? "optional" : undefined;
  return qualifier ? `${hint} — ${qualifier}` : hint;
}

interface ComposerProps extends ModelFavoritesProps {
  status: TaskStatus;
  /** The parent has settled; sends start immediately alongside remaining children. */
  backgroundWorking?: boolean;
  /** App-owned draft for the selected chat; the composer stays mounted across selection. */
  draftState?: ComposerDraftState;
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
  executionPolicy?: ExecutionPolicyConfig;
  appliedExecutionPolicy?: ExecutionPolicyConfig;
  onModeChange?: (mode: TaskMode) => void;
  onConfigure: (patch: { providerId?: string; modelId?: string; thinkingLevel?: ThinkingLevel }) => void;
  /** Resolves false when the send failed; the composer then restores the draft and attachments.
   * Attached text files travel as `files` and join the message text at the wire; `queue` is true
   * while running so the message waits for the active work to finish. */
  onSend: (message: string, images: ImageContent[], files: FileAttachment[], queue?: boolean) => Promise<boolean>;
  commands?: SlashCommand[];
  commandsReady?: boolean;
  commandsLoading?: boolean;
  commandsError?: string;
  onRequestCommands?: () => void;
  onCommand?: (name: string, args: string, images: ImageContent[]) => Promise<boolean>;
  onLiteral?: (message: string, images: ImageContent[], files: FileAttachment[]) => Promise<boolean>;
  /** Workspace files for `@` mentions, relative to it; undefined until loaded. */
  mentionFiles?: string[];
  mentionsLoading?: boolean;
  mentionsError?: string;
  mentionsTruncated?: boolean;
  /** Called whenever a new `@` token opens, so the list is fresh. Omit to turn mentions off. */
  onRequestMentions?: () => void;
  /** Messages queued on the running prompt, shown between the transcript and the draft. */
  queuedMessages?: QueuedMessage[];
  /** Interrupts active work and sends this worker-owned entry next. App reports failures. */
  onSteer?: (messageId: string) => Promise<boolean>;
  /** Takes all queued messages back out of the worker; resolves with their texts for the draft. */
  onDequeue?: () => Promise<string[] | undefined>;
  onStop: () => void;
  onOpenSettings: () => void;
  /** When true the composer is visually dimmed and non-interactive (e.g. a dialog needs attention). */
  disabled?: boolean;
  /** Replaces the draft whenever `nonce` changes, e.g. with the text of a rewound message
   *  (whose attached files are restored from its generated section). */
  seed?: { text: string; nonce: number };
  /** Traces an ambient accent line around the border; used on the draft hero only. */
  comet?: boolean;
  /** Shows this text read-only instead of the draft while the hero composer hands off to the docked one. */
  frozen?: string;
  /** The configured persona name in the built-in placeholder and busy copy. */
  agentName?: string;
}

export function Composer({ draftState, status, backgroundWorking = false, providerId, modelId, thinkingLevel, providers, favoriteModels, favoriteSaving, onSetFavorite, stats, header, placeholder, popoverSide = "top", mode, executionPolicy = DEFAULT_EXECUTION_POLICY, appliedExecutionPolicy, onModeChange, onConfigure, onSend, commands = [], commandsReady, commandsLoading, commandsError, onRequestCommands, onCommand, onLiteral, mentionFiles, mentionsLoading, mentionsError, mentionsTruncated, onRequestMentions, queuedMessages, onSteer, onDequeue, onStop, onOpenSettings, disabled, seed, comet, frozen, agentName = DEFAULT_AGENT_NAME }: ComposerProps) {
  const [localDraft, setLocalDraft] = useState<ComposerDraft>(EMPTY_DRAFT);
  const value = draftState?.value ?? localDraft;
  const updateDraft = draftState?.update ?? setLocalDraft;
  const { text: draft, images: attachments, files } = value;
  const draftKey = draftState?.key ?? "local";
  const activeDraftKey = useRef(draftKey);
  activeDraftKey.current = draftKey;
  function setField<K extends keyof ComposerDraft>(field: K, next: SetStateAction<ComposerDraft[K]>) {
    updateDraft((current) => ({ ...current, [field]: typeof next === "function"
      ? (next as (value: ComposerDraft[K]) => ComposerDraft[K])(current[field]) : next }));
  }
  const setDraft = (next: SetStateAction<string>) => setField("text", next);
  const setAttachments = (next: SetStateAction<ImageContent[]>) => setField("images", next);
  const setFiles = (next: SetStateAction<FileAttachment[]>) => setField("files", next);
  const [attachNotice, setAttachNotice] = useState<string>();
  const [slashNotice, setSlashNotice] = useState<string>();
  const [slashDismissedAt, setSlashDismissedAt] = useState<number>();
  const [slashIndex, setSlashIndex] = useState(0);
  const [caret, setCaret] = useState(draft.length);
  const [mentionIndex, setMentionIndex] = useState(0);
  /** Where the `@` token Escape closed starts; it stays closed until that `@` goes away. */
  const [mentionDismissedAt, setMentionDismissedAt] = useState<number>();
  const [dragging, setDragging] = useState(false);
  /** The attachment the lightbox is showing; its own data URL is already full size. */
  const [shownImage, setShownImage] = useState<{ index: number; url: string } | null>(null);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const highlightRef = useRef<HTMLDivElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const seedNonces = useRef(new Map<string, number>());
  const fileRef = useRef<HTMLInputElement>(null);
  // Queue actions are single-flight per chat, including when selection leaves and returns.
  // The ref guards same-tick repeats; state disables controls without changing worker-owned rows.
  const queueActionLocks = useRef(new Set<string>());
  const [pendingQueueKeys, setPendingQueueKeys] = useState<ReadonlySet<string>>(() => new Set());
  const queuePending = pendingQueueKeys.has(draftKey);
  const busy = status === "running" || status === "stopping";
  const queueButton = busy && !backgroundWorking;
  const model = providers.find((provider) => provider.id === providerId)?.models.find((item) => item.id === modelId);
  const vision = model?.vision === true;
  const noVisionMessage = `${model?.name || model?.id || "This model"} doesn't accept images. Turn on Vision for it in Settings.`;
  // Images picked for a vision model, then the model was switched to one without it.
  const blockedByModel = attachments.length > 0 && !vision;
  const tokenEnd = draft.search(/\s/);
  const commandEnd = tokenEnd < 0 ? draft.length : tokenEnd;
  const commandToken = draft.startsWith("/") ? draft.slice(1, commandEnd) : "";
  const composerText = frozen ?? draft;
  const leadingCommand = /^(\s*)\/([^\s]+)/.exec(composerText);
  const highlightedCommand = leadingCommand && commands.some((command) => command.name === leadingCommand[2])
    ? leadingCommand : null;
  const slash = activeSlashCommand(draft, caret);
  const suggestions = commands.filter((command) => command.name.toLowerCase().includes(slash?.query.toLowerCase() ?? ""));
  const showCommands = slash !== null && slash.start !== slashDismissedAt && !disabled && frozen === undefined;
  // Once a command with a hint is selected but before any argument is typed, surface it inline.
  // `insertCommand` leaves the caret after the trailing space, so this keys off the draft being
  // a bare command token — not where the caret sits.
  const bareCommand = draft.startsWith("/") && !draft.slice(commandEnd).trim();
  const argNote = !showCommands && frozen === undefined && !disabled && bareCommand
    ? argHintNote(commands.find((command) => command.name === commandToken))
    : undefined;
  const mention = onRequestMentions && !showCommands && !disabled && frozen === undefined ? activeMention(draft, caret) : null;
  const showMentions = mention !== null && mention.start !== mentionDismissedAt;
  const mentionQuery = showMentions ? mention.query : undefined;
  const mentionSuggestions = useMemo(
    () => mentionQuery !== undefined && mentionFiles ? rankMentions(mentionFiles, mentionQuery) : [],
    [mentionFiles, mentionQuery]
  );
  const mentionStart = showMentions ? mention.start : undefined;
  useEffect(() => { if (mentionStart !== undefined) onRequestMentions?.(); }, [mentionStart]);
  const slashStart = showCommands ? slash.start : undefined;
  useEffect(() => {
    if (slashStart !== undefined && commandsReady !== true && !commandsLoading && !commandsError) onRequestCommands?.();
  }, [slashStart, commandsReady, commandsLoading, commandsError, draftKey]);
  // Opening a new token also refreshes a cached catalogue, as typing a leading slash did.
  useEffect(() => {
    if (slashStart !== undefined && commandsReady === true && !commandsLoading && !commandsError) onRequestCommands?.();
  }, [slashStart, draftKey]);

  // Picker state and notices belong to the visible composer, never to the previous chat.
  useLayoutEffect(() => {
    setShownImage(null);
    setAttachNotice(undefined);
    setSlashNotice(undefined);
    setSlashDismissedAt(undefined);
    setSlashIndex(0);
    setCaret(draft.length);
    setMentionIndex(0);
    setMentionDismissedAt(undefined);
    setDragging(false);
    areaRef.current?.setSelectionRange(draft.length, draft.length);
  }, [draftKey]);

  useEffect(() => { if (commandsError && draft.startsWith("/") && !commandsLoading) setSlashNotice(commandsError); }, [commandsError, commandsLoading]);

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 200)}px`;
    syncHighlightScroll();
  }, [draft, frozen, highlightedCommand?.[0]]);

  // Keep the decorative text aligned with native editing, including caret-driven scrolling.
  function syncHighlightScroll() {
    const area = areaRef.current;
    const highlight = highlightRef.current;
    if (!area || !highlight) return;
    highlight.scrollTop = area.scrollTop;
    highlight.scrollLeft = area.scrollLeft;
  }

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
    if (seedNonce === undefined || seedText.current === undefined || seedNonces.current.get(draftKey) === seedNonce) return;
    seedNonces.current.set(draftKey, seedNonce);
    // A rewound message's text carries its attached files in the generated section; put them
    // back in the tray instead of showing the section in the draft.
    const restored = splitFileSection(seedText.current);
    setDraft(restored.text);
    setFiles(restored.files);
    areaRef.current?.focus();
  }, [seedNonce, draftKey]);

  /** Attached files travel inside the message text, so they can't ride a `/` command: Pi
   *  expands commands (and the queue resolves the catalog entry first) and the generated
   *  section would fold into the expansion. Sending the same text literally never expands. */
  function filesVsCommand(message: string): boolean {
    return files.length > 0 && message.startsWith("/");
  }

  /** Every running send queues for after the active work, raw or command-expanded.
   * Steer is a separate action on a stable queued-message id. */
  async function queueDraft(literal = false) {
    const message = draft.trim();
    if (!message || status !== "running" || disabled || frozen !== undefined || providers.length === 0 || blockedByModel) return;
    if (!literal && filesVsCommand(message)) {
      setSlashNotice("Remove attached files before running this command.");
      setSlashDismissedAt(slash?.start);
      return;
    }
    const images = attachments;
    const keptFiles = files;
    updateDraft(EMPTY_DRAFT);
    setAttachNotice(undefined);
    const ok = literal && onLiteral ? await onLiteral(message, images, keptFiles) : await onSend(message, images, keptFiles, true);
    if (!ok) {
      updateDraft((current) => ({
        text: current.text ? `${message}\n\n${current.text}` : message,
        images: [...images, ...current.images],
        files: [...keptFiles, ...current.files]
      }));
    }
  }

  async function send() {
    if (frozen !== undefined || disabled || providers.length === 0 || status === "stopping") return;
    const message = draft.trim();
    if (!message || blockedByModel) return;
    if (filesVsCommand(message)) {
      setSlashNotice("Remove attached files before running this command.");
      setSlashDismissedAt(slash?.start);
      return;
    }
    if (busy) {
      // Pi is working: queue instead of sending. Slash commands that act on the app itself
      // cannot queue (they are not messages); skills, templates and unknown text can — Pi
      // expands the first two and refuses extension commands.
      const appCommand = message.startsWith("/") && commands.some((command) => command.name === /^\/([^\s]+)/.exec(message)?.[1] && command.source === "app");
      if (appCommand) {
        // `/goal pause|resume|clear` are the exception: they drive a live loop, so they
        // dispatch exactly like an idle-time command (App's sendSlash gates the same way).
        const goalMatch = /^\/(goal)\s+(pause|resume|clear)\s*$/.exec(message);
        if (goalMatch && onCommand) {
          try {
            const ok = await onCommand("goal", goalMatch[2], []);
            if (ok) updateDraft((current) => current === value ? EMPTY_DRAFT : current);
            if (activeDraftKey.current === draftKey) {
              if (ok) { setSlashNotice(undefined); setSlashDismissedAt(slash?.start); }
              else setSlashNotice("That command could not run. Try again.");
            }
          } catch (reason) { if (activeDraftKey.current === draftKey) setSlashNotice(String(reason)); }
          return;
        }
        setSlashNotice(`Wait for ${agentName} to finish before running /${/^\/([^\s]+)/.exec(message)?.[1]}.`);
        setSlashDismissedAt(slash?.start);
        return;
      }
      await queueDraft();
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
        setSlashDismissedAt(slash?.start);
        return;
      }
      const selected = commands.find((command) => command.name === name);
      if (images.length && (!selected || selected.source === "extension" || selected.source === "app")) {
        setSlashNotice("Remove images before running this command.");
        return;
      }
      try {
        const ok = await onCommand(name, args, images);
        if (ok) updateDraft((current) => current === value ? EMPTY_DRAFT : current);
        if (activeDraftKey.current === draftKey) {
          if (ok) { setSlashNotice(undefined); setSlashDismissedAt(slash?.start); }
          else setSlashNotice("That command could not run. Try again.");
        }
      } catch (reason) { if (activeDraftKey.current === draftKey) setSlashNotice(String(reason)); }
      return;
    }
    const keptFiles = files;
    updateDraft(EMPTY_DRAFT);
    setAttachNotice(undefined);
    const ok = await onSend(message, images, keptFiles);
    if (!ok) {
      updateDraft((current) => ({
        text: current.text ? `${message}\n\n${current.text}` : message,
        images: [...images, ...current.images],
        files: [...keptFiles, ...current.files]
      }));
    }
  }

  function insertCommand(name: string) {
    if (!slash) return;
    // Pi dispatches only a leading command. Move the chosen token there, keeping the
    // surrounding draft as arguments and the caret at the original insertion point.
    const prefix = draft.slice(0, slash.start).trimStart();
    const suffix = draft.slice(slash.end).trimStart();
    const next = `/${name} ${prefix}${suffix}`;
    const position = name.length + 2 + prefix.length;
    setDraft(next);
    setCaret(position);
    setSlashDismissedAt(0);
    setSlashNotice(undefined);
    requestAnimationFrame(() => { if (activeDraftKey.current !== draftKey) return; areaRef.current?.focus(); areaRef.current?.setSelectionRange(position, position); });
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
    requestAnimationFrame(() => { if (activeDraftKey.current !== draftKey) return; areaRef.current?.focus(); areaRef.current?.setSelectionRange(position, position); });
  }

  async function sendLiteral() {
    if (!onLiteral || disabled || frozen !== undefined || providers.length === 0 || status === "stopping" || !draft.trim() || blockedByModel) return;
    if (busy) return void queueDraft(true);
    const ok = await onLiteral(draft.trim(), attachments, files);
    if (ok) {
      updateDraft((current) => current === value ? EMPTY_DRAFT : current);
      if (activeDraftKey.current === draftKey) setSlashNotice(undefined);
    }
  }

  /** Images need the model's Vision; text files ride the message text and always attach. */
  async function addFiles(picked: File[]) {
    if (picked.length === 0) return;
    const result = await attachFiles({ images: attachments, files }, picked, { vision, noVisionMessage });
    setAttachments(result.images);
    setFiles(result.files);
    if (activeDraftKey.current === draftKey) setAttachNotice(result.error);
  }

  function removeAttachment(index: number) {
    const next = attachments.filter((_, itemIndex) => itemIndex !== index);
    setAttachments(next);
    setAttachNotice(undefined);
  }

  function removeFile(index: number) {
    const next = files.filter((_, itemIndex) => itemIndex !== index);
    setFiles(next);
    setAttachNotice(undefined);
  }

  async function changeQueue(action: () => Promise<void>) {
    if (disabled || frozen !== undefined || status === "stopping" || queueActionLocks.current.has(draftKey)) return;
    queueActionLocks.current.add(draftKey);
    setPendingQueueKeys(new Set(queueActionLocks.current));
    try {
      await action();
    } catch (reason) {
      // App normally reports failures through its runtime and resolves false/undefined.
      // A rejected callback still leaves the queue and draft intact and releases the controls.
      if (activeDraftKey.current === draftKey) setAttachNotice(String(reason));
    } finally {
      queueActionLocks.current.delete(draftKey);
      setPendingQueueKeys(new Set(queueActionLocks.current));
    }
  }

  async function steerQueued(messageId: string) {
    if (!onSteer || status !== "running") return;
    await changeQueue(async () => { await onSteer(messageId); });
  }

  /** Restore all queued texts to their originating draft, keeping newer typing and files.
   * Queue-state events remove the rows; the generated file sections return to the tray. */
  async function restoreQueued() {
    if (!onDequeue) return;
    await changeQueue(async () => {
      const texts = await onDequeue();
      if (!texts || texts.length === 0) return;
      if (activeDraftKey.current === draftKey) setSlashNotice(undefined);
      const parts = texts.map((text) => splitFileSection(text));
      const restoredFiles = parts.flatMap((part) => part.files);
      if (restoredFiles.length > 0) setFiles((current) => [...current, ...restoredFiles]);
      const restored = parts.map((part) => part.text).filter(Boolean).join("\n\n");
      if (restored) setDraft((current) => (current.trim() ? `${current}\n\n${restored}` : restored));
      requestAnimationFrame(() => { if (activeDraftKey.current === draftKey) areaRef.current?.focus(); });
    });
  }

  const acceptsDrop = (event: DragEvent) => !disabled && providers.length > 0 && event.dataTransfer.types.includes("Files");

  // Show the words, not generated file sections. Never reconstruct a steered payload from them.
  const queuedEntries = (queuedMessages ?? []).map((entry) => ({ ...entry, text: splitFileSection(entry.text).text }));
  const queueActionsDisabled = disabled || frozen !== undefined || status === "stopping" || queuePending;

  return (
    <div
      ref={wrapRef}
      className={`composer-wrap ${disabled ? "disabled" : ""} ${dragging ? "dropping" : ""}`}
      onDragOver={(event) => {
        if (!acceptsDrop(event)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
        setDragging(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={(event) => {
        if (!acceptsDrop(event)) return;
        event.preventDefault();
        setDragging(false);
        void addFiles(filesFrom(event.dataTransfer));
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
            {queuedEntries.map((entry) => (
              <div className="queued-message" role="listitem" key={entry.id}>
                <span className="queued-tag">Queued</span>
                <span className="queued-text">{entry.text}</span>
                <Tooltip label={`Interrupt ${agentName} and send this next. Keep other messages queued.`}>
                  <button type="button" className="secondary-button compact queued-steer" disabled={queueActionsDisabled || status !== "running" || !onSteer} onClick={() => void steerQueued(entry.id)}>
                    Steer
                  </button>
                </Tooltip>
                <Tooltip label="Restore all queued messages to the composer">
                  <button type="button" className="queued-remove" aria-label="Restore queued messages to the composer" disabled={queueActionsDisabled || !onDequeue} onClick={() => void restoreQueued()}>
                    <Icon name="close" />
                  </button>
                </Tooltip>
              </div>
            ))}
          </div>
        )}
        {(attachments.length > 0 || files.length > 0) && (
          <div className="composer-attachments">
            {attachments.map((image, index) => (
              <div className="attachment-thumb" key={index}>
                <button type="button" className="attachment-open" aria-label={`Open attached image ${index + 1}`} onClick={() => setShownImage({ index, url: imageDataUrl(image) })}>
                  <img src={imageDataUrl(image)} alt={`Attached image ${index + 1}`} />
                </button>
                <button type="button" className="attachment-remove" aria-label={`Remove image ${index + 1}`} onClick={() => removeAttachment(index)}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
            {files.map((file, index) => (
              <div className="attachment-file" key={index}>
                <Icon name="file" />
                <span className="attachment-file-name">{file.name}</span>
                <button type="button" className="attachment-remove" aria-label={`Remove file ${index + 1}`} onClick={() => removeFile(index)}>
                  <Icon name="close" />
                </button>
              </div>
            ))}
          </div>
        )}
        {(attachNotice || blockedByModel) && (
          <div className="attachment-notice" role="status">{blockedByModel ? `${noVisionMessage} Or remove the images to send.` : attachNotice}</div>
        )}
        {slashNotice && <div className="attachment-notice" role="status">{slashNotice} <button type="button" disabled={disabled || frozen !== undefined || status === "stopping" || blockedByModel || !onLiteral} onClick={() => void sendLiteral()}>Send as message</button></div>}
        <AnimatePresence initial={false}>
          {argNote && (
            <motion.div
              className="arg-hint"
              role="note"
              initial={{ opacity: 0, y: -3 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -3, transition: { duration: 0.12, ease: EASE } }}
              transition={{ duration: 0.18, ease: EASE }}
            >{argNote}</motion.div>
          )}
        </AnimatePresence>
        <div className={`composer-input${highlightedCommand ? " highlighted" : ""}`}>
          {highlightedCommand && <div ref={highlightRef} className="composer-highlight" aria-hidden="true">
            {highlightedCommand[1]}<span className="composer-command">/{highlightedCommand[2]}</span>{composerText.slice(highlightedCommand[0].length)}{"\n"}
          </div>}
          <textarea
            ref={areaRef}
            aria-controls={showCommands ? "slash-command-list" : showMentions ? "mention-list" : undefined}
            aria-expanded={showCommands || showMentions}
            aria-activedescendant={showCommands && suggestions.length ? `slash-option-${Math.min(slashIndex, suggestions.length - 1)}`
              : showMentions && mentionSuggestions.length ? `mention-option-${Math.min(mentionIndex, mentionSuggestions.length - 1)}` : undefined}
            value={composerText}
            rows={1}
            readOnly={frozen !== undefined}
            onChange={(event) => {
              if (frozen !== undefined) return;
              const value = event.target.value;
              setDraft(value);
              setCaret(event.target.selectionStart);
              setSlashIndex(0);
              setMentionIndex(0);
              setMentionDismissedAt((at) => at !== undefined && value[at] === "@" ? at : undefined);
              setSlashNotice(undefined);
              setSlashDismissedAt(undefined);
            }}
            onClick={(event) => setCaret(event.currentTarget.selectionStart)}
            onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
            onScroll={syncHighlightScroll}
            onPaste={(event) => {
              // Rich-text apps put an image rendition next to copied text; that is a text paste.
              if (event.clipboardData.types.includes("text/plain")) return;
              const picked = filesFrom(event.clipboardData);
              if (picked.length === 0) return;
              event.preventDefault();
              void addFiles(picked);
            }}
            onKeyDown={(event) => {
              if (showCommands && !event.nativeEvent.isComposing) {
                if (event.key === "ArrowDown" && suggestions.length) { event.preventDefault(); setSlashIndex((index) => (index + 1) % suggestions.length); return; }
                if (event.key === "ArrowUp" && suggestions.length) { event.preventDefault(); setSlashIndex((index) => (index - 1 + suggestions.length) % suggestions.length); return; }
                if ((event.key === "Tab" || event.key === "Enter") && suggestions.length && !event.shiftKey) { event.preventDefault(); insertCommand(suggestions[Math.min(slashIndex, suggestions.length - 1)].name); return; }
                if (event.key === "Escape") { event.preventDefault(); setSlashDismissedAt(slash.start); return; }
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
                void send();
              }
            }}
            placeholder={placeholder ?? (providers.length === 0 ? "Connect a provider to start…" : status === "stopping" ? `${agentName} is stopping — keep your next message here…` : queueButton ? `${agentName} is working — ⏎ queues for after…` : mode === "plan" ? `Describe the work — in Plan mode ${agentName} inspects and proposes a plan for your approval…` : mode === "ultraplan" ? `Describe the work — in Ultra Plan ${agentName} interviews you one question at a time…` : `Ask ${agentName} to inspect, change, or run something…`)}
            disabled={disabled || providers.length === 0}
          />
        </div>
        <div className="composer-toolbar">
          <div className="composer-left">
            {providers.length === 0 ? (
              <button type="button" className="model-pill callout" onClick={onOpenSettings}>
                <Icon name="key" /> Connect a provider
              </button>
            ) : (
              <>
                <Tooltip label="Attach files or images">
                  <button
                    type="button"
                    className="attach-button"
                    aria-label="Attach files"
                    disabled={disabled}
                    onClick={() => fileRef.current?.click()}
                  >
                    <Icon name="paperclip" />
                  </button>
                </Tooltip>
                <input
                  ref={fileRef}
                  type="file"
                  multiple
                  hidden
                  data-testid="attach-input"
                  onChange={(event) => {
                    const picked = filesFrom(event.target.files);
                    event.target.value = "";
                    void addFiles(picked);
                  }}
                />
                <ModelPicker
                  providers={providers}
                  favoriteModels={favoriteModels}
                  favoriteSaving={favoriteSaving}
                  onSetFavorite={onSetFavorite}
                  providerId={providerId}
                  modelId={modelId}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                {stats && <ContextPanel stats={stats} popoverSide={popoverSide} />}
                <ReasoningToggle
                  providers={providers}
                  providerId={providerId}
                  modelId={modelId}
                  thinkingLevel={thinkingLevel}
                  disabled={busy}
                  popoverSide={popoverSide}
                  onConfigure={onConfigure}
                />
                {onModeChange && <>
                  <ModeToggle mode={mode ?? "build"} disabled={busy} onChange={onModeChange} />
                  <ExecutionPolicyNotice saved={executionPolicy} applied={appliedExecutionPolicy} running={busy} />
                </>}
              </>
            )}
          </div>
          <div className="composer-right">
            {busy && (
              <Tooltip label={status === "stopping" ? "Stopping…" : "Stop"}>
                <button type="button" className="send-button stop" onClick={onStop} disabled={disabled || frozen !== undefined || status === "stopping"} aria-label="Stop">
                  <Icon name="stop" />
                </button>
              </Tooltip>
            )}
            <Tooltip label={status === "stopping" ? `Wait for ${agentName} to stop` : queueButton ? `Queue for after ${agentName} finishes (⏎)` : "Send (⏎)"}>
              <button type="button" className={`send-button${queueButton ? " queue" : ""}`} onClick={() => void send()} disabled={disabled || frozen !== undefined || providers.length === 0 || status === "stopping" || !draft.trim() || blockedByModel} aria-label={queueButton ? "Queue message" : "Send message"}>
                <Icon name="send" />
                {queueButton && <span>Queue</span>}
              </button>
            </Tooltip>
          </div>
        </div>
      </motion.div>
      {shownImage && <ImageLightbox preview={shownImage.url} alt={`Attached image ${shownImage.index + 1}`} onClose={() => setShownImage(null)} />}
    </div>
  );
}
