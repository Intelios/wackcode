/**
 * Reply images reference an image already saved on this chat's current branch. Only the
 * request copy gets these labels: session originals, transcript text and image budgets stay
 * unchanged. Pi clones context before handlers run, so associate it with projection provenance
 * by image contents and message metadata, never object identity or position. Ambiguity gives
 * no reference. This extension belongs only to parent chats, before image-budget pruning.
 */
import type { ExtensionAPI, SessionProjection } from "@earendil-works/pi-coding-agent";

export interface ImageReference { entryId: string; index: number }

const ENTRY_ID = /^[A-Za-z0-9_-]{1,256}$/;
const REFERENCE = /^wackcode-image:\/\/entry\/([A-Za-z0-9_-]{1,256})\/(0|[1-9]\d*)$/;

const validEntryId = (entryId: string) => ENTRY_ID.exec(entryId)?.[0] === entryId;

export const IMAGE_REPLY_GUIDANCE = 'Images: show selected pictures with ![description](reference "Optional caption"), copying the supplied reference. Select useful images; never invent references. Read local images with an available read tool first; file paths and remote image URLs cannot display. Do not claim to inspect images you cannot see.';

export function parseImageReference(value: unknown): ImageReference | undefined {
  if (typeof value !== "string") return undefined;
  const match = REFERENCE.exec(value);
  if (!match || match[0] !== value) return undefined;
  const index = Number(match[2]);
  return Number.isSafeInteger(index) ? { entryId: match[1], index } : undefined;
}

export function imageReferenceUrl(entryId: string, index: number): string | undefined {
  return validEntryId(entryId) && Number.isSafeInteger(index) && index >= 0
    ? `wackcode-image://entry/${entryId}/${index}` : undefined;
}

interface Image { data: string; mimeType: string }
interface Source { entryId: string; images: Image[] }
interface MessageLike { role?: unknown; timestamp?: unknown; toolCallId?: unknown; content?: unknown }

function imageMessage(value: unknown): { message: MessageLike; key: string; images: Image[] } | undefined {
  if (!value || typeof value !== "object") return undefined;
  const message = value as MessageLike;
  if (message.role !== "user" && message.role !== "toolResult") return undefined;
  if (typeof message.timestamp !== "number" || !Number.isFinite(message.timestamp) || !Array.isArray(message.content)) return undefined;
  if (message.role === "toolResult" && (typeof message.toolCallId !== "string" || !message.toolCallId)) return undefined;
  const images: Image[] = [];
  for (const block of message.content) {
    if (!block || typeof block !== "object" || block.type !== "image") continue;
    if (typeof block.data !== "string" || !block.data || typeof block.mimeType !== "string" || !block.mimeType) return undefined;
    images.push({ data: block.data, mimeType: block.mimeType });
  }
  if (!images.length) return undefined;
  return { message, images, key: JSON.stringify([message.role, message.timestamp, message.role === "toolResult" ? message.toolCallId : null, images.length]) };
}

function sameImages(left: Image[], right: Image[]): boolean {
  return left.length === right.length && left.every((image, index) => image.mimeType === right[index].mimeType && image.data === right[index].data);
}

export function annotateImageReferences<T>(messages: readonly T[], projection: Pick<SessionProjection, "entries">): T[] | undefined {
  const sources = new Map<string, Source[]>();
  for (const entry of projection.entries) {
    if (entry.sourceEntry.type !== "message" || !validEntryId(entry.sourceEntry.id)) continue;
    for (const raw of entry.messages) {
      const candidate = imageMessage(raw);
      if (!candidate) continue;
      const matches = sources.get(candidate.key) ?? [];
      matches.push({ entryId: entry.sourceEntry.id, images: candidate.images });
      sources.set(candidate.key, matches);
    }
  }
  let changed = false;
  const annotated = messages.map((value): T => {
    const candidate = imageMessage(value);
    if (!candidate) return value;
    const matches = (sources.get(candidate.key) ?? []).filter((source) => sameImages(source.images, candidate.images));
    if (matches.length !== 1) return value;
    const references = candidate.images.map((_, index) => `Image ${index + 1}: ${imageReferenceUrl(matches[0].entryId, index)}`);
    const text = `\n\n<wackcode-image-references-v1>\n${references.join("\n")}\n</wackcode-image-references-v1>`;
    const content = candidate.message.content as Array<{ type?: unknown; text?: unknown }>;
    if (content.at(-1)?.type === "text" && content.at(-1)?.text === text) return value;
    changed = true;
    return { ...(value as Record<string, unknown>), content: [...content, { type: "text", text }] } as T;
  });
  return changed ? annotated : undefined;
}

export function createImageReferencesExtension() {
  return function imageReferences(pi: ExtensionAPI) {
    pi.on("context", (event, ctx) => {
      const messages = annotateImageReferences(event.messages, ctx.sessionManager.buildSessionProjection());
      return messages ? { messages } : undefined;
    });
  };
}
