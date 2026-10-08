/**
 * Shared by the Code composer and Chat mode's composer: the picked images and text files
 * waiting to send. Each image opens full size from the data URL the composer already holds.
 */
import { imageDataUrl, type FileAttachment } from "../../attachment-utils";
import type { ImageContent } from "../../types";
import { Icon } from "../Icons";

interface AttachmentTrayProps {
  images: ImageContent[];
  files: FileAttachment[];
  onOpenImage: (index: number, url: string) => void;
  onRemoveImage: (index: number) => void;
  onRemoveFile: (index: number) => void;
  className?: string;
}

export function AttachmentTray({ images, files, onOpenImage, onRemoveImage, onRemoveFile, className = "composer-attachments" }: AttachmentTrayProps) {
  if (images.length === 0 && files.length === 0) return null;
  return (
    <div className={className}>
      {images.map((image, index) => (
        <div className="attachment-thumb" key={index}>
          <button type="button" className="attachment-open" aria-label={`Open attached image ${index + 1}`} onClick={() => onOpenImage(index, imageDataUrl(image))}>
            <img src={imageDataUrl(image)} alt={`Attached image ${index + 1}`} />
          </button>
          <button type="button" className="attachment-remove" aria-label={`Remove image ${index + 1}`} onClick={() => onRemoveImage(index)}>
            <Icon name="close" />
          </button>
        </div>
      ))}
      {files.map((file, index) => (
        <div className="attachment-file" key={index}>
          <Icon name="file" />
          <span className="attachment-file-name">{file.name}</span>
          <button type="button" className="attachment-remove" aria-label={`Remove file ${index + 1}`} onClick={() => onRemoveFile(index)}>
            <Icon name="close" />
          </button>
        </div>
      ))}
    </div>
  );
}
