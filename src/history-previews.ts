/** Cold thumbnails never enter the live revision chain or change an unrelated message's
 * identity. A terminal batch also settles missing previews after a reader fails or is stopped. */
import type { HistoryPreview, NormalizedBlock, TaskRuntime } from "./types";

export function applyHistoryPreviews(
  runtime: TaskRuntime | undefined,
  frame: { historyId: string; previews: readonly HistoryPreview[]; done?: true }
): TaskRuntime | undefined {
  if (!runtime?.snapshot || runtime.historyId !== frame.historyId) return runtime;
  const updates = new Map(frame.previews.map((preview) => [preview.imageId, preview]));
  const update = <T extends { imageId?: string; thumbnail?: string; failed?: true }>(image: T): T => {
    if (!image.imageId || image.thumbnail || image.failed) return image;
    const preview = updates.get(image.imageId);
    if (preview?.thumbnail) return { ...image, thumbnail: preview.thumbnail };
    if (preview?.failed || frame.done) return { ...image, failed: true };
    return image;
  };
  let changed = false;
  const messages = runtime.snapshot.messages.map((message) => {
    let messageChanged = false;
    const blocks = message.blocks.map((block): NormalizedBlock => {
      let next = block.type === "image" ? update(block) : block;
      if (block.images) {
        const images = block.images.map(update);
        if (images.some((image, index) => image !== block.images![index])) next = { ...next, images };
      }
      messageChanged ||= next !== block;
      return next;
    });
    changed ||= messageChanged;
    return messageChanged ? { ...message, blocks } : message;
  });
  if (!changed && !frame.done) return runtime;
  return {
    ...runtime,
    ...(frame.done ? { historyId: undefined } : {}),
    snapshot: changed ? { ...runtime.snapshot, messages } : runtime.snapshot,
  };
}
