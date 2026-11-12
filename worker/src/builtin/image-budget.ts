/**
 * One byte budget for every image a chat sends to the model. Only the request copy changes;
 * saved images, transcript previews and originals remain untouched.
 *
 * The working set is the latest image-bearing tool batch plus the latest user attachments.
 * A batch can span several tool results: pruning any of those before the next model call
 * makes a requested comparison impossible and can trap the model in a reread/evict loop.
 * Older tool images go first, then older attachments, in chunks to keep the prefix stable.
 * If the working set itself exceeds the budget, resize its request copies rather than dropping
 * fresh images. This keeps the gateway body cap without hiding a just-completed tool result.
 */
import { createHash } from "node:crypto";
import type { ExtensionAPI, ResizedImage } from "@earendil-works/pi-coding-agent";
import { INLINE_IMAGE_LIMITS } from "../image-limits.js";
import type { ImageContent } from "../protocol.js";
import { storedImageRecord } from "../saved-images.js";

/** Maximum combined base64 payload of images in a model request. */
export const MAX_CONTEXT_IMAGE_BYTES = 3 * 1024 * 1024;
/** Older images removed per step; never includes the working set. */
export const PRUNE_CHUNK = 3;
export const PRUNED_TOOL_IMAGE_TEXT = "[Earlier image omitted from this request. Use read_image with its saved reference to reopen it, or list_images to find it.]";
export const PRUNED_USER_IMAGE_TEXT = "[Earlier attachment omitted from this request. Use read_image with its saved reference to reopen it, or list_images to find it. The saved image does not need to be attached again.]";
const VIEW_CACHE_ENTRIES = 16;
const VIEW_CACHE_BYTES = 16 * 1024 * 1024;

type ResizeImage = typeof import("@earendil-works/pi-coding-agent")["resizeImage"];
type ImageKind = "tool" | "user";

interface MessageLike {
  role?: unknown;
  content?: unknown;
}

interface ImageSlot {
  message: number;
  block: number;
  index: number;
  bytes: number;
  kind: ImageKind;
  image: ImageContent;
}

function asMessage(value: unknown): MessageLike {
  return typeof value === "object" && value !== null ? value as MessageLike : {};
}

function imageSlots(messages: readonly unknown[]): ImageSlot[] {
  const slots: ImageSlot[] = [];
  messages.forEach((value, message) => {
    const raw = asMessage(value);
    const kind = raw.role === "toolResult" ? "tool" : raw.role === "user" ? "user" : undefined;
    if (!kind || !Array.isArray(raw.content)) return;
    let index = 0;
    (raw.content as unknown[]).forEach((block, blockIndex) => {
      if (typeof block !== "object" || block === null || (block as { type?: unknown }).type !== "image") return;
      const image = block as ImageContent;
      slots.push({ message, block: blockIndex, index: index++, bytes: typeof image.data === "string" ? image.data.length : 0, kind, image });
    });
  });
  return slots;
}

function removalOrder(messages: readonly unknown[], slots: readonly ImageSlot[]): ImageSlot[] {
  let batchEnd: number | undefined;
  let attachment: number | undefined;
  for (const slot of slots) {
    if (slot.kind === "tool") batchEnd = slot.message;
    else attachment = slot.message;
  }
  const protectedMessages = new Set<number>();
  if (attachment !== undefined) protectedMessages.add(attachment);
  if (batchEnd !== undefined) {
    let message = batchEnd;
    do { protectedMessages.add(message--); }
    while (message >= 0 && asMessage(messages[message]).role === "toolResult");
  }
  const candidates = slots.filter((slot) => !protectedMessages.has(slot.message));
  return [...candidates.filter((slot) => slot.kind === "tool"), ...candidates.filter((slot) => slot.kind === "user")];
}

/**
 * Prune only images the model has already had a chance to inspect. The protected working set
 * can still exceed the budget; budgetImages fits those request copies without losing images.
 */
export function pruneImages<T>(
  messages: readonly T[],
  budget = MAX_CONTEXT_IMAGE_BYTES,
  chunk = PRUNE_CHUNK,
  canReopen = true
): T[] | undefined {
  const slots = imageSlots(messages);
  let total = slots.reduce((sum, slot) => sum + slot.bytes, 0);
  if (total <= budget) return undefined;

  const candidates = removalOrder(messages, slots);
  let needed = 0;
  while (total > budget && needed < candidates.length) {
    total -= candidates[needed++].bytes;
  }
  if (!needed) return undefined;
  const remove = Math.min(candidates.length, Math.ceil(needed / Math.max(1, chunk)) * Math.max(1, chunk));
  const byMessage = new Map<number, Map<number, ImageKind>>();
  for (const slot of candidates.slice(0, remove)) {
    const blocks = byMessage.get(slot.message) ?? new Map<number, ImageKind>();
    blocks.set(slot.block, slot.kind);
    byMessage.set(slot.message, blocks);
  }

  return messages.map((value, message): T => {
    const blocks = byMessage.get(message);
    if (!blocks) return value;
    const content = asMessage(value).content as unknown[];
    return {
      ...(value as Record<string, unknown>),
      content: content.map((block, blockIndex) => {
        const kind = blocks.get(blockIndex);
        return kind ? { type: "text", text: canReopen
          ? kind === "tool" ? PRUNED_TOOL_IMAGE_TEXT : PRUNED_USER_IMAGE_TEXT
          : "[Earlier image omitted to keep this request small. Read the source file again if you need to see it.]" } : block;
      })
    } as T;
  });
}

export async function budgetImages<T>(
  messages: readonly T[],
  resize: ResizeImage,
  budget = MAX_CONTEXT_IMAGE_BYTES,
  chunk = PRUNE_CHUNK,
  canReopen = true
): Promise<T[] | undefined> {
  const pruned = pruneImages(messages, budget, chunk, canReopen);
  const current = pruned ?? messages;
  const slots = imageSlots(current);
  if (slots.reduce((sum, slot) => sum + slot.bytes, 0) <= budget) return pruned;

  const maxBytes = Math.floor(budget / slots.length);
  const replacements = new Map<number, Map<number, unknown[]>>();
  for (const slot of slots) {
    if (slot.bytes <= maxBytes) continue;
    const blocks = replacements.get(slot.message) ?? new Map<number, unknown[]>();
    let replacement: unknown[] = [{ type: "text", text: "[Image could not be resized to fit this request. Inspect it individually, or use its local file for exact measurements.]" }];
    try {
      const resized = await resize(Buffer.from(slot.image.data, "base64"), slot.image.mimeType, { ...INLINE_IMAGE_LIMITS, maxBytes });
      if (resized && resized.data.length <= maxBytes) {
        const record = storedImageRecord(current[slot.message], slot.index, slot.image);
        const width = record?.width ?? resized.originalWidth;
        const height = record?.height ?? resized.originalHeight;
        const source = record ? "original" : "incoming viewing copy";
        replacement = [
          { type: "image", data: resized.data, mimeType: resized.mimeType },
          { type: "text", text: `[Image for this request: ${source} ${width} × ${height}, viewing copy ${resized.width} × ${resized.height}. Multiply viewing-copy coordinates by ${(width / resized.width).toFixed(2)} horizontally and ${(height / resized.height).toFixed(2)} vertically to map to the ${source}. Saved originals are unchanged.]` }
        ];
      }
    } catch {
      // Pi swallows context-hook errors and sends the unbounded originals, so fail per image.
    }
    blocks.set(slot.block, replacement);
    replacements.set(slot.message, blocks);
  }
  return current.map((value, message): T => {
    const blocks = replacements.get(message);
    return blocks ? { ...(value as Record<string, unknown>), content: (asMessage(value).content as unknown[]).flatMap((block, index) => blocks.get(index) ?? [block]) } as T : value;
  });
}

/** Applies to parent chats of both kinds and to children, with private request-copy caches. */
export function createImageBudgetExtension(canReopen = true, resize: ResizeImage = async (...args) => (await import("@earendil-works/pi-coding-agent")).resizeImage(...args)) {
  return function imageBudget(pi: ExtensionAPI) {
    const views = new Map<string, ResizedImage>();
    let viewBytes = 0;
    const cachedResize: ResizeImage = async (bytes, mimeType, options) => {
      const key = `${JSON.stringify(options)}:${createHash("sha256").update(mimeType).update("\0").update(bytes).digest("hex")}`;
      const cached = views.get(key);
      if (cached) {
        views.delete(key);
        views.set(key, cached);
        return cached;
      }
      const result = await resize(bytes, mimeType, options);
      if (result && result.data.length <= VIEW_CACHE_BYTES) {
        views.set(key, result);
        viewBytes += result.data.length;
        while (views.size > VIEW_CACHE_ENTRIES || viewBytes > VIEW_CACHE_BYTES) {
          const oldest = views.keys().next().value!;
          viewBytes -= views.get(oldest)!.data.length;
          views.delete(oldest);
        }
      }
      return result;
    };
    pi.on("context", async (event, ctx) => {
      if (!ctx.model?.input.includes("image")) return;
      const messages = await budgetImages(event.messages, cachedResize, MAX_CONTEXT_IMAGE_BYTES, PRUNE_CHUNK, canReopen);
      return messages ? { messages } : undefined;
    });
  };
}
