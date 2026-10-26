/** Reply references name saved session entries, never files or renderer-local image ids. */
import type { NormalizedMessage } from "./types";

export interface ReplyImageReference {
  entryId: string;
  index: number;
}

export interface ReplyImageSource extends ReplyImageReference {
  kind: "message" | "tool";
  toolCallId?: string;
  imageId?: string;
  thumbnail?: string;
  failed?: true;
}

const REFERENCE = /^wackcode-image:\/\/entry\/([A-Za-z0-9_-]{1,256})\/(0|[1-9][0-9]*)$/;

export function parseReplyImageReference(value: string): ReplyImageReference | undefined {
  const match = REFERENCE.exec(value);
  if (!match || match[0] !== value) return undefined;
  const index = Number(match[2]);
  return Number.isSafeInteger(index) ? { entryId: match[1], index } : undefined;
}

export function replyImageReference(entryId: string, index: number): string {
  return `wackcode-image://entry/${entryId}/${index}`;
}

export function replyImages(messages: readonly NormalizedMessage[]): ReadonlyMap<string, ReplyImageSource> {
  const images = new Map<string, ReplyImageSource>();
  for (const message of messages) {
    if (!message.entryId) continue;
    const entryId = message.entryId;
    const add = (index: number, image: { imageId?: string; thumbnail?: string; failed?: true }, kind: ReplyImageSource["kind"], toolCallId?: string) => {
      const reference = replyImageReference(entryId, index);
      if (parseReplyImageReference(reference)) images.set(reference, { entryId, index, kind, toolCallId, imageId: image.imageId, thumbnail: image.thumbnail, failed: image.failed });
    };
    if (message.role === "user") {
      message.blocks.filter((block) => block.type === "image").forEach((image, index) => add(index, image, "message"));
    } else if (message.role === "tool") {
      // Normalization attaches a result's previews to its final block only.
      const result = message.blocks.filter((block) => block.type === "tool-result" && block.images).at(-1);
      if (result?.toolCallId) result.images?.forEach((image, index) => add(index, image, "tool", result.toolCallId));
    }
  }
  return images;
}
