import { useLayoutEffect, useRef, useState } from "react";
import type { NormalizedBlock } from "../types";
import type { FileAttachment } from "../attachment-utils";
import { Icon } from "./Icons";

interface Props {
  /** The message's own words, without the generated attached-files section. */
  text: string;
  /** The original message's images, as the transcript shows them. */
  images: NormalizedBlock[];
  /** The original message's attached text files. */
  files: FileAttachment[];
  /** Whether the chat's model accepts images. */
  vision: boolean;
  modelName?: string;
  /** Resolves false when the send did not go through; the editor then stays open. */
  onSend: (text: string, files: FileAttachment[], removeImages: number[]) => Promise<boolean>;
  onCancel: () => void;
}

/** Edits a sent message in place. Sending makes the edit a new version of the message. */
export function MessageEditor({ text, images, files, vision, modelName, onSend, onCancel }: Props) {
  const [draft, setDraft] = useState(text);
  const [removed, setRemoved] = useState<number[]>([]);
  const [removedFiles, setRemovedFiles] = useState<number[]>([]);
  const [busy, setBusy] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const kept = images.filter((_, index) => !removed.includes(index));
  const keptFiles = files.filter((_, index) => !removedFiles.includes(index));
  const blocked = kept.length > 0 && !vision;

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    area.style.height = `${Math.min(area.scrollHeight, 320)}px`;
  }, [draft]);

  useLayoutEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.focus();
    area.setSelectionRange(area.value.length, area.value.length);
  }, []);

  async function send() {
    const message = draft.trim();
    if (!message || busy || blocked) return;
    setBusy(true);
    const sent = await onSend(message, keptFiles, removed);
    if (!sent) setBusy(false);
  }

  return (
    <div className="message-editor">
      {images.length > 0 && (
        <div className="composer-attachments">
          {images.map((image, index) => removed.includes(index) ? null : (
            <div className="attachment-thumb" key={image.imageId ?? index}>
              {image.thumbnail
                ? <img src={image.thumbnail} alt={`Attached image ${index + 1}`} />
                : <div className="image-pending" role="img" aria-label={`Attached image ${index + 1}`}><Icon name="image" /></div>}
              <button type="button" className="attachment-remove" aria-label={`Remove image ${index + 1}`} onClick={() => setRemoved((current) => [...current, index])}>
                <Icon name="close" />
              </button>
            </div>
          ))}
        </div>
      )}
      {keptFiles.length > 0 && (
        <div className="composer-attachments">
          {files.map((file, index) => removedFiles.includes(index) ? null : (
            <div className="attachment-file" key={index}>
              <Icon name="file" />
              <span className="attachment-file-name">{file.name}</span>
              <button type="button" className="attachment-remove" aria-label={`Remove file ${index + 1}`} onClick={() => setRemovedFiles((current) => [...current, index])}>
                <Icon name="close" />
              </button>
            </div>
          ))}
        </div>
      )}
      {blocked && (
        <div className="attachment-notice" role="status">
          {modelName || "This model"} doesn't accept images. Remove them to send, or turn on Vision for it in Settings.
        </div>
      )}
      <textarea
        ref={areaRef}
        aria-label="Edit message"
        value={draft}
        rows={1}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          } else if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            void send();
          }
        }}
      />
      <div className="message-editor-actions">
        <button type="button" className="secondary-button" onClick={onCancel}>Cancel</button>
        <button type="button" className="primary-button" disabled={!draft.trim() || busy || blocked} onClick={() => void send()}>
          {busy ? "Sending…" : "Send"}
        </button>
      </div>
    </div>
  );
}
