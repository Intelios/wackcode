/**
 * Originals live beside the session (inside the scratchpad for Chat mode); Pi keeps only its
 * bounded viewing copies. Finalized messages carry relative blob identities, never paths or
 * pixels, so forks can copy their originals without depending on the source chat. A record
 * applies only while its inline digest still matches: context edits cannot inherit old pixels.
 */
import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { link, mkdir, open, unlink, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import type { ResizedImage } from "@earendil-works/pi-coding-agent";
import type { ImageContent } from "./protocol.js";
import { INLINE_IMAGE_LIMITS } from "./image-limits.js";
import { transcriptEntries } from "./transcript.js";
import type { EntryLike } from "./tree.js";

export const IMAGE_METADATA_KEY = "wackcodeImages";
export const MAX_STORED_IMAGE_BYTES = 20 * 1024 * 1024;
const ENTRY_ID = /^[A-Za-z0-9_-]{1,256}$/;
const REFERENCE = /^wackcode-image:\/\/entry\/([A-Za-z0-9_-]{1,256})\/(0|[1-9]\d*)$/;
const BLOB = /^[a-f0-9]{64}\.(?:png|jpg|gif|webp|bmp|avif|ico|bin)$/;
const VIEW_CACHE_ENTRIES = 16;
const VIEW_CACHE_BYTES = 16 * 1024 * 1024;

export interface ImageReference { entryId: string; index: number }
export function parseImageReference(value: unknown): ImageReference | undefined {
  if (typeof value !== "string") return undefined;
  const match = REFERENCE.exec(value);
  if (!match || match[0] !== value) return undefined;
  const index = Number(match[2]);
  return Number.isSafeInteger(index) ? { entryId: match[1], index } : undefined;
}
export function imageReferenceUrl(entryId: string, index: number): string | undefined {
  return ENTRY_ID.exec(entryId)?.[0] === entryId && Number.isSafeInteger(index) && index >= 0
    ? `wackcode-image://entry/${entryId}/${index}` : undefined;
}

export interface StoredImage {
  blob: string;
  mimeType: string;
  width: number;
  height: number;
  viewWidth: number;
  viewHeight: number;
  inlineHash: string;
  quality?: "saved-copy";
}
export interface ImageMetadata {
  v: 1;
  images: Array<StoredImage | null>;
  error?: string;
}
export interface SavedImage {
  reference: string;
  entryId: string;
  index: number;
  image: ImageContent;
  record?: StoredImage;
  timestamp?: number;
  source: string;
  role: "user" | "toolResult";
  replay: boolean;
  error?: string;
}
export const imageHash = (image: ImageContent) => createHash("sha256").update(image.mimeType).update("\0").update(image.data).digest("hex");
const bytesHash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const dimensions = (value: unknown) => Number.isSafeInteger(value) && (value as number) > 0;

export function imageMetadata(value: unknown): ImageMetadata | undefined {
  const candidate = (value as Record<string, unknown> | undefined)?.[IMAGE_METADATA_KEY] as ImageMetadata | undefined;
  return candidate?.v === 1 && Array.isArray(candidate.images) ? candidate : undefined;
}
function recordFor(value: unknown, image: ImageContent): StoredImage | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as StoredImage;
  return typeof record.blob === "string" && BLOB.test(record.blob) && typeof record.mimeType === "string"
    && dimensions(record.width) && dimensions(record.height) && dimensions(record.viewWidth) && dimensions(record.viewHeight)
    && record.inlineHash === imageHash(image) ? record : undefined;
}
export function storedImageRecord(message: unknown, index: number, image: ImageContent): StoredImage | undefined {
  return recordFor(imageMetadata(message)?.images[index], image);
}

export function savedImages(branch: EntryLike[]): SavedImage[] {
  return transcriptEntries(branch).flatMap(({ entryId, raw }) => {
    const message = raw as Record<string, unknown>;
    if (!message || !["user", "toolResult"].includes(String(message.role)) || !Array.isArray(message.content)) return [];
    if (message.role === "toolResult" && (typeof message.toolCallId !== "string" || !message.toolCallId)) return [];
    const metadata = imageMetadata(message);
    return message.content.filter((block) => block?.type === "image").flatMap((block, index): SavedImage[] => {
      const reference = imageReferenceUrl(entryId, index);
      if (!reference || typeof block.data !== "string" || !block.data || typeof block.mimeType !== "string" || !block.mimeType) return [];
      const image: ImageContent = { type: "image", data: block.data, mimeType: block.mimeType };
      return [{ reference, entryId, index, image, record: recordFor(metadata?.images[index], image),
        timestamp: typeof message.timestamp === "number" ? message.timestamp : undefined,
        source: message.role === "user" ? `User attachment ${index + 1}` : `${String(message.toolName ?? "Tool")} image ${index + 1}`,
        role: message.role as "user" | "toolResult",
        replay: message.role === "toolResult" && message.toolName === "read_image",
        ...(typeof metadata?.error === "string" ? { error: metadata.error } : {}) }];
    });
  });
}
export function resolveSavedImage(reference: unknown, branch: EntryLike[]): SavedImage | undefined {
  const parsed = parseImageReference(reference);
  return parsed ? savedImages(branch).find((image) => image.entryId === parsed.entryId && image.index === parsed.index) : undefined;
}

type Resize = (bytes: Uint8Array, mimeType: string, options: typeof INLINE_IMAGE_LIMITS) => Promise<ResizedImage | null>;
export interface ImageStoreOptions {
  directory: string;
  cwd: string;
  isChat: boolean;
  resizeImage: Resize;
  convertToPng: (data: string, mimeType: string) => Promise<{ data: string; mimeType: string } | null>;
}
export interface PreparedImage { image: ImageContent; record: StoredImage; resized: ResizedImage }
export interface LoadedImage extends PreparedImage { path: string; quality: "original" | "saved-copy"; warning?: string }

export class ImageStore {
  // Per-store, content-addressed viewing copies only. Never cache filesystem reads:
  // load/save must still enforce confinement and verify original blobs on every access.
  // Limits participate in the key; failed conversions are never retained.
  private readonly views = new Map<string, ResizedImage>();
  private viewBytes = 0;
  private readonly viewing = new Map<string, Promise<ResizedImage>>();
  constructor(readonly options: ImageStoreOptions) {}

  private async checkedPath(path: string, operation: "read" | "write"): Promise<string> {
    if (!this.options.isChat) return path;
    // Dynamic import avoids the Chat allowlist -> tools -> store -> allowlist cycle.
    const { confine } = await import("./builtin/chat-mode/policy.js");
    const result = confine(operation, path, this.options.cwd);
    if (!result.ok) throw new Error(result.reason);
    return resolve(this.options.cwd, result.path ?? path);
  }

  private async blobPath(blob: string, operation: "read" | "write"): Promise<string> {
    if (!BLOB.test(blob)) throw new Error("That saved image has an invalid file identity.");
    return this.checkedPath(join(this.options.directory, blob), operation);
  }

  async read(record: Pick<StoredImage, "blob" | "mimeType">): Promise<ImageContent> {
    const path = await this.blobPath(record.blob, "read");
    const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW);
    try {
      const stat = await file.stat();
      if (!stat.isFile() || stat.size > MAX_STORED_IMAGE_BYTES) throw new Error("The saved image is not a valid image file.");
      const bytes = Buffer.alloc(stat.size + 1);
      let length = 0;
      while (length < bytes.length) {
        const result = await file.read(bytes, length, bytes.length - length, null);
        if (!result.bytesRead) break;
        length += result.bytesRead;
      }
      const original = bytes.subarray(0, length);
      if (length !== stat.size || bytesHash(original) !== record.blob.split(".")[0]) throw new Error("The saved original image has changed.");
      return { type: "image", mimeType: record.mimeType, data: original.toString("base64") };
    } finally { await file.close(); }
  }

  private async save(image: ImageContent): Promise<string> {
    const bytes = Buffer.from(image.data, "base64");
    if (bytes.length > MAX_STORED_IMAGE_BYTES) throw new Error("The original image exceeds the 20 MB image limit.");
    const extension = ({ "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp", "image/bmp": "bmp", "image/avif": "avif", "image/x-icon": "ico" } as Record<string, string>)[image.mimeType] ?? "bin";
    const blob = `${bytesHash(bytes)}.${extension}`;
    await mkdir(await this.checkedPath(this.options.directory, "write"), { recursive: true, mode: 0o700 });
    const destination = await this.blobPath(blob, "write");
    const temporary = await this.checkedPath(join(this.options.directory, `.${randomUUID()}.tmp`), "write");
    await writeFile(temporary, bytes, { flag: "wx", mode: 0o600 });
    try {
      try { await link(temporary, destination); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error; }
      await this.read({ blob, mimeType: image.mimeType });
    } finally { await unlink(temporary); }
    return blob;
  }

  async view(image: ImageContent): Promise<ResizedImage> {
    const key = `${JSON.stringify(INLINE_IMAGE_LIMITS)}:${imageHash(image)}`;
    const cached = this.views.get(key);
    if (cached) {
      this.views.delete(key);
      this.views.set(key, cached);
      return { ...cached };
    }
    let pending = this.viewing.get(key);
    if (!pending) {
      pending = this.resizeView(image).then((result) => {
        const bytes = result.data.length;
        if (bytes <= VIEW_CACHE_BYTES) {
          this.views.set(key, { ...result });
          this.viewBytes += bytes;
          while (this.views.size > VIEW_CACHE_ENTRIES || this.viewBytes > VIEW_CACHE_BYTES) {
            const oldest = this.views.keys().next().value!;
            this.viewBytes -= this.views.get(oldest)!.data.length;
            this.views.delete(oldest);
          }
        }
        return result;
      }).finally(() => { this.viewing.delete(key); });
      this.viewing.set(key, pending);
    }
    return { ...await pending };
  }

  private async resizeView(image: ImageContent): Promise<ResizedImage> {
    let result = await this.options.resizeImage(Buffer.from(image.data, "base64"), image.mimeType, INLINE_IMAGE_LIMITS);
    if (!result) {
      const converted = await this.options.convertToPng(image.data, image.mimeType);
      if (converted) result = await this.options.resizeImage(Buffer.from(converted.data, "base64"), converted.mimeType, INLINE_IMAGE_LIMITS);
    }
    if (!result) throw new Error("The image could not be read or resized below the inline image size limit.");
    return result;
  }

  async prepare(original: ImageContent): Promise<PreparedImage> {
    const resized = await this.view(original);
    const blob = await this.save(original);
    const image: ImageContent = { type: "image", data: resized.data, mimeType: resized.mimeType };
    return { image, resized, record: { blob, mimeType: original.mimeType, width: resized.originalWidth, height: resized.originalHeight,
      viewWidth: resized.width, viewHeight: resized.height, inlineHash: imageHash(image) } };
  }

  async original(saved: SavedImage): Promise<ImageContent> {
    if (!saved.record) return saved.image;
    try { return await this.read(saved.record); }
    catch { throw new Error("The original image file is missing or changed. Use read_image to reopen its saved viewing copy."); }
  }

  async load(saved: SavedImage): Promise<LoadedImage> {
    let original = saved.image;
    let warning = saved.error ?? "Only the saved viewing copy is available. The original resolution was not preserved.";
    let quality: LoadedImage["quality"] = "saved-copy";
    if (saved.record) {
      try { original = await this.read(saved.record); quality = saved.record.quality ?? "original"; }
      catch { warning = "The original image file is missing or changed. Only the saved viewing copy is available."; }
    }
    const prepared = await this.prepare(original);
    if (quality === "saved-copy") prepared.record.quality = "saved-copy";
    return { ...prepared, path: await this.blobPath(prepared.record.blob, "read"), quality, ...(quality === "saved-copy" ? { warning } : {}) };
  }

  async copyFrom(source: ImageStore, branch: EntryLike[]): Promise<number> {
    let failed = 0;
    const records = new Map(savedImages(branch).flatMap((image) => image.record ? [[image.record.blob, image.record] as const] : []));
    for (const record of records.values()) {
      try { await this.save(await source.read(record)); }
      catch { failed += 1; }
    }
    return failed;
  }
}
