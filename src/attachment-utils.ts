import type { ImageContent } from "./types";

/**
 * Composer-side attachment limits. `src-tauri/src/commands.rs` (`validate_images`) enforces the
 * same image numbers, so keep those in sync. The worker then resizes each image to Pi's own
 * inline limits, so these only need to keep obviously unusable files out. Text files exist
 * only inside the message text (see `composeFileSection`), so their limits are the composer's
 * own: they exist to keep one message from swallowing the context window.
 */
export const ACCEPTED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;
export const MAX_IMAGES = 8;
export const MAX_IMAGE_BYTES = 20 * 1024 * 1024;
export const MAX_FILES = 8;
export const MAX_FILE_BYTES = 256 * 1024;
export const MAX_FILE_TOTAL_BYTES = 512 * 1024;

/** A text file attached to a message; folded into the message text at send time. */
export interface FileAttachment {
  /** The file name as picked — what the tray and the transcript show. */
  name: string;
  /** The file's full text, as read (`readFileText`). */
  text: string;
}

/** What the composer holds: images ride the prompt as `ImageContent`, text files as text. */
export interface AttachmentState {
  images: ImageContent[];
  files: FileAttachment[];
}

export interface AttachResult extends AttachmentState {
  /** Why some files were left out, if any were. */
  error?: string;
}

/** Every file in a paste, drop, or file-picker selection, in order. */
export function filesFrom(source: DataTransfer | FileList | null | undefined): File[] {
  if (!source) return [];
  const files = "files" in source ? source.files : source;
  return Array.from(files);
}

/**
 * Read `picked` into attachments, appending to `existing` up to the limits. PNG, JPEG, GIF and
 * WebP go through as images (a model without Vision refuses them with `noVisionMessage`); every
 * other readable file goes through as text. Files that don't fit are skipped and explained,
 * never silently dropped.
 */
export async function attachFiles(existing: AttachmentState, picked: File[], options: {
  vision: boolean;
  noVisionMessage: string;
  /** Whether `@` mentions exist where this composer is; false in Chat mode, which has no workspace to point into. */
  mentions?: boolean;
}): Promise<AttachResult> {
  const images = [...existing.images];
  const files = [...existing.files];
  const problems: string[] = [];
  let totalBytes = files.reduce((sum, file) => sum + file.text.length, 0);
  for (const file of picked) {
    const name = file.name || "That file";
    if ((ACCEPTED_IMAGE_TYPES as readonly string[]).includes(file.type)) {
      if (!options.vision) {
        problems.push(options.noVisionMessage);
        continue;
      }
      if (file.size > MAX_IMAGE_BYTES) {
        problems.push(`${name} is larger than ${MAX_IMAGE_BYTES / 1024 / 1024} MB.`);
        continue;
      }
      if (images.length >= MAX_IMAGES) {
        problems.push(`Attach at most ${MAX_IMAGES} images to one message.`);
        break;
      }
      try {
        images.push(await readImageFile(file));
      } catch {
        problems.push(`${name} could not be read.`);
      }
      continue;
    }
    if (file.size > MAX_FILE_BYTES) {
      problems.push(`${name} is larger than ${MAX_FILE_BYTES / 1024} KB.`);
      continue;
    }
    if (files.length >= MAX_FILES) {
      problems.push(`Attach at most ${MAX_FILES} files to one message.`);
      break;
    }
    let text: string;
    try {
      text = await readFileText(file);
    } catch {
      problems.push(`${name} isn't a text file or a PNG, JPEG, GIF, or WebP image.${options.mentions === false ? "" : " Put it in the workspace and mention its path with @ instead."}`);
      continue;
    }
    if (totalBytes + text.length > MAX_FILE_TOTAL_BYTES) {
      problems.push(`Attached text can add up to at most ${MAX_FILE_TOTAL_BYTES / 1024} KB.`);
      continue;
    }
    totalBytes += text.length;
    files.push({ name, text });
  }
  return { images, files, error: problems[0] };
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

/** A text file's contents; anything that isn't UTF-8 text (or holds NUL bytes) is refused. */
export function readFileText(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    const refuse = () => reject(new Error("The file is not text"));
    reader.onerror = refuse;
    reader.onload = () => {
      const bytes = new Uint8Array(reader.result as ArrayBuffer);
      if (bytes.includes(0)) return refuse();
      try {
        resolve(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
      } catch {
        refuse();
      }
    };
    reader.readAsArrayBuffer(file);
  });
}

export function imageDataUrl(image: ImageContent): string {
  return `data:${image.mimeType};base64,${image.data}`;
}

/**
 * Attached text files ride the message text itself: a prompt is text plus images, so at send
 * time the user's words get one generated `<attached-files>` section appended. Both halves live
 * here — `composeFileSection` writes it, `splitFileSection` reads it back for the transcript,
 * the message editor, the queued-message chips, and Copy.
 *
 *     …the user's own words…
 *
 *     <attached-files>
 *     <file name="notes.md" lines="2">
 *     first line
 *     second line
 *     </file>
 *     </attached-files>
 *
 * `lines` counts the file's lines exactly (`text.split("\n").length`), so the parser takes that
 * many lines verbatim and content containing lookalike tags round-trips untouched. Only a tail
 * that parses through to the very end of the message counts as a section; anything else is
 * plain text. Slash-command messages never carry files (the Composer refuses them): Pi expands
 * a leading `/command`, and a generated section must not fold into an expansion. Sending the
 * same text literally is fine — nothing expands.
 */
export function composeFileSection(text: string, files: FileAttachment[]): string {
  if (files.length === 0) return text;
  const lines = [SECTION_OPEN];
  for (const file of files) {
    const content = file.text.split("\n");
    lines.push(`<file name="${escapeAttr(file.name)}" lines="${content.length}">`, ...content, "</file>");
  }
  lines.push(SECTION_CLOSE);
  const section = lines.join("\n");
  return text ? `${text}\n\n${section}` : section;
}

export interface SplitFiles {
  /** Everything the user typed, without the generated section. */
  text: string;
  files: FileAttachment[];
}

/** The message's own words and its attached files — the inverse of `composeFileSection`. */
export function splitFileSection(message: string): SplitFiles {
  // Scan section starts from the end: a genuine section is the message tail, and a lookalike
  // inside the user's own words must not shadow it.
  const head = `\n\n${SECTION_OPEN}\n`;
  for (let at = message.lastIndexOf(head); at >= 0; at = message.lastIndexOf(head, at - 1)) {
    const files = parseSection(message, at + 2);
    if (files) return { text: message.slice(0, at), files };
  }
  if (message.startsWith(`${SECTION_OPEN}\n`)) {
    const files = parseSection(message, 0);
    if (files) return { text: "", files };
  }
  return { text: message, files: [] };
}

const SECTION_OPEN = "<attached-files>";
const SECTION_CLOSE = "</attached-files>";
const FILE_CLOSE = "</file>";

/**
 * The files a section holds when it starts at `start` and parses through to the message's end:
 * open tag, exactly `lines` lines of content, close tag — per file. Line counts (rather than
 * pattern matching) make attached content that itself contains these tags round-trip exactly.
 */
function parseSection(message: string, start: number): FileAttachment[] | undefined {
  const lines = message.slice(start).split("\n");
  if (lines[0] !== SECTION_OPEN) return undefined;
  const files: FileAttachment[] = [];
  let index = 1;
  while (index < lines.length && lines[index] !== SECTION_CLOSE) {
    const open = FILE_OPEN.exec(lines[index]);
    if (!open) return undefined;
    const name = unescapeAttr(open[1]);
    const count = Number(open[2]);
    index += 1;
    if (index + count >= lines.length) return undefined;
    const text = lines.slice(index, index + count).join("\n");
    index += count;
    if (lines[index] !== FILE_CLOSE) return undefined;
    index += 1;
    files.push({ name, text });
  }
  if (files.length === 0 || index !== lines.length - 1 || lines[index] !== SECTION_CLOSE) return undefined;
  return files;
}

/** An open tag's name and line count; names are attribute-escaped so any file name round-trips. */
const FILE_OPEN = /^<file name="((?:[^"&]|&(?:quot|amp|lt|gt|#10|#13);)*)" lines="([1-9][0-9]*)">$/;

const ATTR_ENTITIES: Record<string, string> = { quot: "\"", amp: "&", lt: "<", gt: ">", "#10": "\n", "#13": "\r" };

function escapeAttr(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/\r/g, "&#13;")
    .replace(/\n/g, "&#10;");
}

function unescapeAttr(value: string): string {
  return value.replace(/&(quot|amp|lt|gt|#10|#13);/g, (_, entity: string) => ATTR_ENTITIES[entity]);
}
