/**
 * One byte budget for every image a chat sends to the model.
 *
 * Images are the largest thing in a request, and Pi re-sends the whole history on every turn.
 * Compaction never drops the images on its kept side, so a chat that reads screenshots,
 * captures the browser, or looks at an app a dozen times grows its request by megabytes each
 * time — until the gateway in front of the provider answers HTTP 413 (nginx's `client_max_body_size`
 * is commonly 1–10 MB) for every turn after that, whatever the model.
 *
 * This rewrites only the copy Pi sends (the `context` hook), never the session itself: the
 * transcript, the lightbox and `tool_image` keep every original. Older tool images go first —
 * the agent can take a new screenshot or read the file again — and only then images the user
 * attached. The newest image is always kept, so the output of the tool call that just finished
 * reaches the model; the rest of its message only goes once every older image has.
 *
 * Removal happens in whole chunks (`PRUNE_CHUNK`) so the rewritten prefix stays byte-identical
 * from call to call between chunk boundaries, keeping the provider's prompt cache warm. The
 * budget leaves most of the same headroom for the same reason.
 *
 * `INLINE_IMAGE_LIMITS` keeps each individual image small; this bounds what many images add up
 * to.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** Base64 bytes of images one request may carry, before the newest message is added. */
export const MAX_CONTEXT_IMAGE_BYTES = 3 * 1024 * 1024;
/** Images removed per step, so the model-facing prefix is stable for a few turns. */
export const PRUNE_CHUNK = 3;
export const PRUNED_TOOL_IMAGE_TEXT = "[Earlier image removed to keep this request small. Read the file or take a new screenshot if you need to see it again.]";
export const PRUNED_USER_IMAGE_TEXT = "[An image the user attached earlier was removed to keep this request small. Ask them to attach it again if you need it.]";

type ImageKind = "tool" | "user";

interface MessageLike {
  role?: unknown;
  content?: unknown;
}

interface ImageSlot {
  message: number;
  block: number;
  bytes: number;
  kind: ImageKind;
}

function asMessage(value: unknown): MessageLike {
  return typeof value === "object" && value !== null ? value as MessageLike : {};
}

function kindOf(message: MessageLike): ImageKind | undefined {
  if (message.role === "toolResult") return "tool";
  if (message.role === "user") return "user";
  return undefined;
}

function imageSlots(messages: readonly unknown[]): ImageSlot[] {
  const slots: ImageSlot[] = [];
  messages.forEach((value, message) => {
    const raw = asMessage(value);
    const kind = kindOf(raw);
    if (!kind || !Array.isArray(raw.content)) return;
    (raw.content as unknown[]).forEach((block, blockIndex) => {
      if (typeof block !== "object" || block === null || (block as { type?: unknown }).type !== "image") return;
      const data = (block as { data?: unknown }).data;
      slots.push({ message, block: blockIndex, bytes: typeof data === "string" ? data.length : 0, kind });
    });
  });
  return slots;
}

/**
 * The images to remove, in order: tool results older than the newest message first, then the
 * user's own images older than it, then the newest message's own images — all oldest first. The
 * single newest image is never removed, so the tool call that just finished still reaches the
 * model; the rest of its message only goes once every older image has.
 */
function removalOrder(slots: readonly ImageSlot[]): ImageSlot[] {
  const newest = slots[slots.length - 1];
  if (!newest) return [];
  const candidates = slots.filter((slot) => slot !== newest);
  const outside = candidates.filter((slot) => slot.message !== newest.message);
  const inside = candidates.filter((slot) => slot.message === newest.message);
  return [
    ...outside.filter((slot) => slot.kind === "tool"),
    ...outside.filter((slot) => slot.kind === "user"),
    ...inside.filter((slot) => slot.kind === "tool"),
    ...inside.filter((slot) => slot.kind === "user")
  ];
}

/**
 * Returns `messages` with the oldest images replaced by a short placeholder, or `undefined` when
 * the images already fit. Messages it changes are shallow copies; the input is never mutated.
 */
export function pruneImages<T>(
  messages: readonly T[],
  budget = MAX_CONTEXT_IMAGE_BYTES,
  chunk = PRUNE_CHUNK
): T[] | undefined {
  const slots = imageSlots(messages);
  let total = slots.reduce((sum, slot) => sum + slot.bytes, 0);
  if (total <= budget) return undefined;

  const candidates = removalOrder(slots);
  if (!candidates.length) return undefined;

  // How many of the oldest candidates have to go, then rounded up to a whole chunk so the next
  // few images can arrive without rewriting the prefix again.
  let needed = 0;
  while (total > budget && needed < candidates.length) {
    total -= candidates[needed].bytes;
    needed += 1;
  }
  if (needed === 0) return undefined;
  const remove = Math.min(candidates.length, Math.ceil(needed / Math.max(1, chunk)) * Math.max(1, chunk));

  const byMessage = new Map<number, Map<number, ImageKind>>();
  for (const slot of candidates.slice(0, remove)) {
    const blocks = byMessage.get(slot.message) ?? new Map<number, ImageKind>();
    blocks.set(slot.block, slot.kind);
    byMessage.set(slot.message, blocks);
  }

  let changed = false;
  const rewritten = messages.map((value, message): T => {
    const blocks = byMessage.get(message);
    if (!blocks) return value;
    const raw = asMessage(value);
    const content = raw.content as unknown[];
    changed = true;
    return {
      ...(value as Record<string, unknown>),
      content: content.map((block, blockIndex) => {
        const kind = blocks.get(blockIndex);
        return kind ? { type: "text", text: kind === "tool" ? PRUNED_TOOL_IMAGE_TEXT : PRUNED_USER_IMAGE_TEXT } : block;
      })
    } as unknown as T;
  });
  return changed ? rewritten : undefined;
}

/** The extension that applies the budget to every request, in coding and Chat mode chats alike. */
export function createImageBudgetExtension() {
  return function imageBudget(pi: ExtensionAPI) {
    pi.on("context", (event) => {
      const messages = pruneImages(event.messages);
      return messages ? { messages } : undefined;
    });
  };
}
