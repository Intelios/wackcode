import { createContext, useCallback, useContext, useMemo, useState } from "react";
import type { NormalizedMessage } from "../types";
import { replyImages, type ReplyImageSource } from "../reply-images";
import { Icon } from "./Icons";
import { ImageLightbox } from "./ui/ImageLightbox";

interface ReplyImageScopeValue {
  images: ReadonlyMap<string, ReplyImageSource>;
  loadMessage?: (entryId: string, index: number) => Promise<string | undefined>;
  loadTool?: (toolCallId: string, index: number) => Promise<string | undefined>;
}

export const ReplyImageScope = createContext<ReplyImageScopeValue | undefined>(undefined);

export function useReplyImageScope(
  messages: readonly NormalizedMessage[],
  loadMessage: ReplyImageScopeValue["loadMessage"],
  loadTool: ReplyImageScopeValue["loadTool"],
  enabled = true
): ReplyImageScopeValue | undefined {
  return useMemo(() => enabled ? { images: replyImages(messages), loadMessage, loadTool } : undefined, [messages, loadMessage, loadTool, enabled]);
}

/** Only saved images in this transcript's branch can supply pixels; Markdown supplies labels. */
export function ReplyImage({ reference, alt, caption }: { reference: string; alt?: string; caption?: string }) {
  const scope = useContext(ReplyImageScope);
  const image = scope?.images.get(reference);
  const [shown, setShown] = useState(false);
  const [failedPreview, setFailedPreview] = useState<string>();
  const description = alt?.trim() || "Image";
  const entryId = image?.entryId;
  const index = image?.index;
  const kind = image?.kind;
  const toolCallId = image?.toolCallId;
  const loadTool = scope?.loadTool;
  const loadMessage = scope?.loadMessage;
  const load = useCallback(() => {
    if (entryId === undefined || index === undefined) return Promise.resolve(undefined);
    return (kind === "tool" && toolCallId
      ? loadTool?.(toolCallId, index)
      : loadMessage?.(entryId, index)) ?? Promise.resolve(undefined);
  }, [entryId, index, kind, toolCallId, loadTool, loadMessage]);
  const unavailable = !image || image.failed || (image.thumbnail && failedPreview === image.thumbnail);
  const preview = image?.thumbnail;
  if (unavailable) {
    return <span className="reply-image-unavailable" role="img" aria-label={`Image unavailable: ${description}`}><Icon name="image" />Image unavailable{alt ? `: ${alt}` : ""}</span>;
  }
  if (!preview) {
    return <span className="reply-image-pending" role="img" aria-label={`Preparing image: ${description}`}><Icon name="image" /><span>Preparing image…</span></span>;
  }
  return (
    <span className="reply-image">
      <button type="button" className="reply-image-button" aria-label={`Open image: ${description}`} onClick={() => setShown(true)}>
        <img src={preview} alt={description} onError={() => setFailedPreview(preview)} />
      </button>
      {caption && <span className="reply-image-caption">{caption}</span>}
      {shown && <ImageLightbox key={JSON.stringify([image.imageId, preview])} preview={preview} load={load} alt={description} onClose={() => setShown(false)} />}
    </span>
  );
}
