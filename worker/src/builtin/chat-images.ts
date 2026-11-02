/**
 * Parent chats archive images before Pi normalizes them, then bind the records on message_end
 * before persistence. This factory must follow other tool-result/message-end handlers. Originals
 * never ride tool details, model requests or snapshots; read_image returns a bounded view.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { ImageContent } from "../protocol.js";
import { IMAGE_METADATA_KEY, ImageStore, imageHash, parseImageReference, resolveSavedImage, savedImages, type ImageMetadata, type StoredImage } from "../saved-images.js";
import type { BuiltinHost } from "./host.js";

export const CHAT_IMAGE_TOOL_NAMES = ["list_images", "read_image"] as const;
export const CHAT_IMAGES_GUIDANCE = "list_images finds saved attachments and tool images, including before compaction. read_image reopens a reference and returns a local file for pixel measurements. Measure that file rather than the resized view. Reopen after a fork for its current path; do not request reattachment.";

export function createChatImagesExtension(host: Pick<BuiltinHost, "supportsVision" | "notice">) {
  let store: ImageStore | undefined;
  let prompt: ImageMetadata | undefined;
  const results = new Map<string, ImageMetadata>();
  const getStore = () => {
    if (!store) throw new Error("This chat's image store is not available.");
    return store;
  };
  const controller = {
    configure(next: ImageStore) { store = next; },
    store: getStore,
    clearPrompt() { prompt = undefined; },
    async preparePrompt(images: ImageContent[] | undefined, retained?: Array<StoredImage | null>): Promise<ImageContent[]> {
      prompt = undefined;
      const prepared: ImageContent[] = [];
      const records: Array<StoredImage | null> = [];
      for (const [index, image] of (images ?? []).entries()) {
        const record = retained?.[index];
        if (record?.inlineHash === imageHash(image)) {
          prepared.push(image);
          records.push(record);
        } else {
          const result = await getStore().prepare(image);
          if (retained) result.record.quality = "saved-copy";
          prepared.push(result.image);
          records.push(result.record);
        }
      }
      if (records.length) prompt = { v: 1, images: records };
      return prepared;
    },
  };

  const factory = (pi: ExtensionAPI) => {
    pi.registerTool({
      name: "list_images",
      label: "List saved images",
      description: "Find images saved in this chat, including attachments and screenshots before compaction. Returns references for read_image, newest first. Reopening an image does not add another source.",
      parameters: { type: "object", additionalProperties: false, properties: {
        offset: { type: "integer", minimum: 0, description: "Results to skip; defaults to 0." },
        limit: { type: "integer", minimum: 1, maximum: 100, description: "Images to return; defaults to 20, at most 100." },
      } },
      async execute(_id, input: unknown, signal, _update, ctx) {
        const { offset = 0, limit = 20 } = (input ?? {}) as { offset?: unknown; limit?: unknown };
        if (typeof offset !== "number" || typeof limit !== "number" || !Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1 || limit > 100) throw new Error("Use a non-negative offset and a limit from 1 to 100.");
        const all = savedImages(ctx.sessionManager.getBranch()).filter((image) => !image.replay).reverse();
        const images = [];
        for (const saved of all.slice(offset, offset + limit)) {
          if (signal?.aborted) throw new Error("Listing images was stopped.");
          let width = saved.record?.width, height = saved.record?.height;
          if (!saved.record) {
            try { const view = await getStore().view(saved.image); width = view.originalWidth; height = view.originalHeight; }
            catch { /* An undecodable legacy image still has a discoverable reference. */ }
          }
          images.push({ reference: saved.reference, source: saved.source, timestamp: saved.timestamp, width, height,
            quality: saved.record && !saved.record.quality ? "original stored" : "saved viewing copy only" });
        }
        return { content: [{ type: "text" as const, text: JSON.stringify({ images, total: all.length,
          ...(offset + limit < all.length ? { nextOffset: offset + limit } : {}) }) }], details: {} };
      },
    });
    pi.registerTool({
      name: "read_image",
      label: "Reopen saved image",
      description: "Reopen a saved chat image by its supplied wackcode-image reference. Returns a bounded viewing copy and an absolute path to the original file for pixel analysis. Use list_images if its reference is no longer in context. Accepts no paths or remote URLs.",
      parameters: { type: "object", additionalProperties: false, required: ["reference"], properties: {
        reference: { type: "string", description: "A supplied wackcode-image://entry/... reference from this chat." },
      } },
      async execute(toolCallId, input: unknown, signal, _update, ctx) {
        const reference = (input as { reference?: unknown } | undefined)?.reference;
        if (typeof reference !== "string" || !parseImageReference(reference)) throw new Error("Use a supplied saved image reference. Call list_images to find one.");
        const saved = resolveSavedImage(reference, ctx.sessionManager.getBranch());
        if (!saved) throw new Error("That image is not available on this chat's current branch. Call list_images to find an available image.");
        if (signal?.aborted) throw new Error("Reopening the image was stopped.");
        const loaded = await getStore().load(saved);
        if (signal?.aborted) throw new Error("Reopening the image was stopped.");
        const vision = host.supportsVision();
        if (vision) results.set(toolCallId, { v: 1, images: [loaded.record] });
        const text = [saved.reference, `${loaded.quality === "original" ? "Original" : "Saved copy"}: ${loaded.record.width} × ${loaded.record.height} pixels (${loaded.record.mimeType}).`,
          `Local file: ${loaded.path}`, loaded.warning,
          vision ? `Viewing copy: ${loaded.resized.width} × ${loaded.resized.height} pixels. Use the local file for exact measurements.` : "This model cannot view images; the file and its metadata are available."].filter(Boolean).join("\n");
        return { content: [{ type: "text" as const, text }, ...(vision ? [loaded.image] : [])], details: {} };
      },
    });
    pi.on("tool_result", async (event) => {
      if (event.toolName === "read_image" || !event.content.some((block) => block.type === "image")) return;
      const records: Array<StoredImage | null> = [];
      const content: typeof event.content = [];
      const errors: string[] = [];
      for (const block of event.content) {
        if (block.type !== "image") { content.push(block); continue; }
        try {
          const result = await getStore().prepare(block);
          records.push(result.record);
          content.push(result.image);
          if (result.resized.wasResized) content.push({ type: "text", text: `[Image resized: original ${result.record.width} × ${result.record.height}, viewing copy ${result.record.viewWidth} × ${result.record.viewHeight}. Use read_image for the original file.]` });
        } catch (error) {
          records.push(null);
          content.push(block);
          errors.push(error instanceof Error ? error.message : "The original image could not be saved.");
        }
      }
      const error = errors.length ? `The original image could not be saved: ${errors.join(" ")}` : undefined;
      if (error) { content.push({ type: "text", text: error }); host.notice(error, "warning"); }
      results.set(event.toolCallId, { v: 1, images: records, ...(error ? { error } : {}) });
      const image = content.find((block) => block.type === "image");
      return { content, ...(event.toolName === "read" && image?.type === "image"
        ? { structuredContent: { type: "image", data: image.data, mimeType: image.mimeType,
          note: content.filter((block) => block.type === "text").map((block) => block.text).join("\n") } }
        : event.structuredContent === undefined ? {} : { structuredContent: event.structuredContent }) };
    });
    pi.on("message_end", (event) => {
      const message = event.message;
      if (message.role !== "user" && message.role !== "toolResult") return;
      const metadata = message.role === "user" ? prompt : message.role === "toolResult" ? results.get(message.toolCallId) : undefined;
      if (message.role === "user") prompt = undefined;
      if (message.role === "toolResult") results.delete(message.toolCallId);
      if (!metadata || !Array.isArray(message.content)) return;
      const images = message.content.filter((block) => block.type === "image");
      const bound = images.map((image, index) => {
        const digest = imageHash(image as ImageContent);
        if (images.length === metadata.images.length) {
          const record = metadata.images[index];
          return record?.inlineHash === digest ? record : null;
        }
        // Pi can omit an invalid image. Recover only an unambiguous surviving source.
        const matches = metadata.images.filter((record) => record?.inlineHash === digest);
        return matches.length === 1 ? matches[0] : null;
      });
      return { message: { ...message, [IMAGE_METADATA_KEY]: { ...metadata, images: bound } } };
    });
    pi.on("agent_settled", () => { results.clear(); prompt = undefined; });
  };
  return { factory, controller };
}
