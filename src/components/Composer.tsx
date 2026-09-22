import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type ReactNode } from "react";
import type { ImageContent, ProviderRecord, SessionSnapshot, TaskMode, TaskStatus, ThinkingLevel } from "../types";
import { ACCEPTED_IMAGE_TYPES, attachImages, imageDataUrl, imageFilesFrom } from "../attachment-utils";
import { formatTokens } from "../chat-utils";
import { Icon } from "./Icons";
import { ContextPanel } from "./ContextPanel";
import { ModelPicker, ReasoningToggle } from "./ModelPicker";
import { ModeToggle } from "./ModeToggle";
import { Tooltip } from "./ui/Tooltip";

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
  onStop: () => void;
  onOpenSettings: () => void;
  /** When true the composer is visually dimmed and non-interactive (e.g. a dialog needs attention). */
  disabled?: boolean;
  /** Replaces the draft whenever `nonce` changes, e.g. with the text of a rewound message. */
  seed?: { text: string; nonce: number };
}

export function Composer({ status, providerId, modelId, thinkingLevel, providers, stats, header, placeholder, popoverSide = "top", mode, onModeChange, onConfigure, onSend, onStop, onOpenSettings, disabled, seed }: ComposerProps) {
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<ImageContent[]>([]);
  const [attachNotice, setAttachNotice] = useState<string>();
  const [dragging, setDragging] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const busy = status === "running" || status === "stopping";
  const model = providers.find((provider) => provider.id === providerId)?.models.find((item) => item.id === modelId);
  const vision = model?.vision === true;
  const noVisionMessage = `${model?.name || model?.id || "This model"} doesn't accept images. Turn on Vision for it in Settings.`;
  // Images picked for a vision model, then the model was switched to one without it.
  const blockedByModel = attachments.length > 0 && !vision;

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 200)}px`;
  }, [draft]);

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
    const message = draft.trim();
    if (!message || busy || blockedByModel) return;
    const images = attachments;
    setDraft("");
    setAttachments([]);
    setAttachNotice(undefined);
    const ok = await onSend(message, images);
    if (!ok) {
      setDraft(message);
      setAttachments(images);
    }
  }

  async function addFiles(files: File[]) {
    if (files.length === 0) return;
    if (!vision) {
      setAttachNotice(noVisionMessage);
      return;
    }
    const result = await attachImages(attachments, files);
    setAttachments(result.images);
    setAttachNotice(result.error);
  }

  function removeAttachment(index: number) {
    setAttachments((current) => current.filter((_, itemIndex) => itemIndex !== index));
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
      {header && <div className="composer-header">{header}</div>}
      <div className="composer" aria-disabled={disabled || undefined}>
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
        <textarea
          ref={areaRef}
          value={draft}
          rows={1}
          onChange={(event) => setDraft(event.target.value)}
          onPaste={(event) => {
            // Rich-text apps put an image rendition next to copied text; that is a text paste.
            if (event.clipboardData.types.includes("text/plain")) return;
            const files = imageFilesFrom(event.clipboardData);
            if (files.length === 0) return;
            event.preventDefault();
            void addFiles(files);
          }}
          onKeyDown={(event) => {
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
      </div>
    </div>
  );
}
