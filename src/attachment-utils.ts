import type { ImageContent } from "./types";

/**
 * Composer-side attachment limits. `src-tauri/src/commands.rs` (`validate_images`) enforces the
 * same numbers, so keep them in sync. The worker then resizes each image to Pi's own inline
 * limits, so these only need to keep obviously unusable files out.
 */
export const ACCEPTED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;
export const MAX_IMAGES = 8;
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024;

/** The image files in a paste, drop, or file-picker selection, in order. */
export function imageFilesFrom(source: DataTransfer | FileList | null | undefined): File[] {
  if (!source) return [];
  const files = "files" in source ? source.files : source;
  return Array.from(files).filter((file) => file.type.startsWith("image/"));
}

export interface AttachResult {
  images: ImageContent[];
  /** Why some files were left out, if any were. */
  error?: string;
}

/**
 * Read `files` into Pi `ImageContent`, appending to `existing` up to the limits. Files that don't
 * fit are skipped and explained, never silently dropped.
 */
export async function attachImages(existing: ImageContent[], files: File[]): Promise<AttachResult> {
  const images = [...existing];
  const problems: string[] = [];
  for (const file of files) {
    if (!(ACCEPTED_IMAGE_TYPES as readonly string[]).includes(file.type)) {
      problems.push(`${file.name || "That image"} isn't a PNG, JPEG, GIF, or WebP.`);
      continue;
    }
    if (file.size > MAX_IMAGE_BYTES) {
      problems.push(`${file.name || "That image"} is larger than ${MAX_IMAGE_BYTES / 1024 / 1024} MB.`);
      continue;
    }
    if (images.length >= MAX_IMAGES) {
      problems.push(`Attach at most ${MAX_IMAGES} images to one message.`);
      break;
    }
    try {
      images.push(await readImageFile(file));
    } catch {
      problems.push(`${file.name || "That image"} could not be read.`);
    }
  }
  return { images, error: problems[0] };
}

export function readImageFile(file: File): Promise<ImageContent> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("The file could not be read"));
    reader.onload = () => {
      const url = String(reader.result ?? "");
      const comma = url.indexOf(",");
      if (!url.startsWith("data:") || comma < 0) {
        reject(new Error("The file could not be read"));
        return;
      }
      resolve({ type: "image", data: url.slice(comma + 1), mimeType: file.type });
    };
    reader.readAsDataURL(file);
  });
}

export function imageDataUrl(image: ImageContent): string {
  return `data:${image.mimeType};base64,${image.data}`;
}
