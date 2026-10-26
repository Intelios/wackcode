/**
 * Local visual assets are copied into the saved tool result, never exposed through a file URL
 * or the asset protocol. A filename only finds candidates: the bytes decide the MIME type.
 * Chat mode applies exactly the file tools' scratchpad policy before opening each candidate.
 */
import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { resolve } from "node:path";

export const MAX_VISUAL_IMAGE_BYTES = 4 * 1024 * 1024;
export const MAX_VISUAL_PAGE_BYTES = 8 * 1024 * 1024;
const IMAGE_EXTENSION = "(?:png|jpe?g|gif|webp|avif|bmp|ico|svg)";

/** Return only supported image signatures; an extension never grants access to text files. */
export function visualImageMime(bytes: Buffer): string | undefined {
  if (bytes.length >= 8 && bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return "image/png";
  if (bytes.length >= 3 && bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return "image/jpeg";
  if (/^GIF8[79]a$/.test(bytes.subarray(0, 6).toString("ascii"))) return "image/gif";
  if (bytes.length >= 12 && bytes.toString("ascii", 0, 4) === "RIFF" && bytes.toString("ascii", 8, 12) === "WEBP") return "image/webp";
  if (bytes.length >= 16 && bytes.toString("ascii", 4, 8) === "ftyp") {
    const boxSize = bytes.readUInt32BE(0);
    const end = Math.min(boxSize, bytes.length);
    if (boxSize >= 16 && (bytes.toString("ascii", 8, 12) === "avif" || bytes.toString("ascii", 8, 12) === "avis")) return "image/avif";
    for (let offset = 16; offset + 4 <= end; offset += 4) {
      if (["avif", "avis"].includes(bytes.toString("ascii", offset, offset + 4))) return "image/avif";
    }
  }
  if (bytes.length >= 14 && bytes.toString("ascii", 0, 2) === "BM") return "image/bmp";
  if (bytes.length >= 6 && bytes.readUInt32LE(0) === 0x00010000 && bytes.readUInt16LE(4) > 0) return "image/x-icon";
  // SVG must be the root element, after an optional XML declaration and comments. Do not
  // accept arbitrary text merely containing an <svg> substring (or an HTML document).
  const text = bytes.toString("utf8").replace(/^\uFEFF/, "");
  if (/^\s*(?:<\?xml\s[^?]*\?>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg(?:\s|\/?>)/i.test(text)) return "image/svg+xml";
  return undefined;
}

interface ImageReference { start: number; end: number; path: string }

function imageReferences(html: string): ImageReference[] {
  const found: ImageReference[] = [];
  const doubleQuoted = `"(\\/(?!\\/)[^"\\r\\n]*?\\.${IMAGE_EXTENSION})"`;
  const singleQuoted = `'(\\/(?!\\/)[^'\\r\\n]*?\\.${IMAGE_EXTENSION})'`;
  const attributes = new RegExp(`\\b[\\w:-]+\\s*=\\s*(?:${doubleQuoted}|${singleQuoted})`, "gi");
  const urls = new RegExp(`url\\(\\s*(?:${doubleQuoted}|${singleQuoted}|(\\/(?!\\/)[^"'()\\r\\n]*?\\.${IMAGE_EXTENSION}))\\s*\\)`, "gi");
  for (const pattern of [attributes, urls]) {
    for (const match of html.matchAll(pattern)) {
      const path = match[1] ?? match[2] ?? match[3];
      const start = match.index + match[0].indexOf(path);
      if (!found.some((reference) => reference.start === start)) found.push({ start, end: start + path.length, path });
    }
  }
  return found.sort((a, b) => a.start - b.start);
}

async function readImage(path: string): Promise<Buffer> {
  // A pipe named *.png must not block the worker before the regular-file check can run.
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK);
  try {
    const stat = await file.stat();
    if (!stat.isFile()) throw new Error("is not a regular image file");
    if (stat.size > MAX_VISUAL_IMAGE_BYTES) throw new Error("exceeds the 4 MiB image limit");
    // Bound the read too: a file growing after stat cannot turn this into an unbounded read.
    const bytes = Buffer.alloc(Math.min(stat.size + 1, MAX_VISUAL_IMAGE_BYTES + 1));
    let length = 0;
    while (length < bytes.length) {
      const { bytesRead } = await file.read(bytes, length, bytes.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > MAX_VISUAL_IMAGE_BYTES || length > stat.size) throw new Error("changed while reading; try again");
    return bytes.subarray(0, length);
  } finally { await file.close(); }
}

export async function inlineVisualImages(html: string, options: { isChat?: boolean; cwd?: string } = {}): Promise<{ html: string; inlined: boolean }> {
  const references = imageReferences(html);
  let bytes = Buffer.byteLength(html, "utf8");
  const pageTooLarge = () => new Error("The finished visual exceeds the 8 MiB page limit. Use smaller images or fewer copies.");
  if (bytes > MAX_VISUAL_PAGE_BYTES) throw pageTooLarge();
  const replacements = new Map<string, string>();
  const failures: string[] = [];
  const occurrences = new Map<string, number>();
  for (const { path } of references) occurrences.set(path, (occurrences.get(path) ?? 0) + 1);
  for (const path of new Set(references.map((reference) => reference.path))) {
    try {
      let source = path;
      if (options.isChat) {
        if (!options.cwd) throw new Error("this chat's scratchpad folder is not available");
        // Loaded at execution time: policy.ts's tool allowlist imports visual.ts.
        const { confine } = await import("./chat-mode/policy.js");
        const confined = confine("read", path, options.cwd);
        if (!confined.ok) throw new Error(confined.reason);
        source = confined.path !== undefined ? resolve(options.cwd, confined.path) : path;
      }
      const image = await readImage(source);
      const mime = visualImageMime(image);
      if (!mime) throw new Error("its bytes are not a supported image (PNG, JPEG, GIF, WebP, AVIF, BMP, ICO or SVG)");
      const prefix = `data:${mime};base64,`;
      const encodedBytes = prefix.length + Math.ceil(image.length / 3) * 4;
      bytes += (encodedBytes - Buffer.byteLength(path, "utf8")) * occurrences.get(path)!;
      if (bytes > MAX_VISUAL_PAGE_BYTES) break;
      replacements.set(path, prefix + image.toString("base64"));
    } catch (error) {
      const reason = error instanceof Error ? error.message : "could not read the file";
      failures.push(`${path}: ${reason}`);
    }
  }
  if (bytes > MAX_VISUAL_PAGE_BYTES) throw pageTooLarge();
  if (failures.length) throw new Error(`Could not include local images. Fix or remove these paths:\n${failures.join("\n")}`);
  let result = "";
  let offset = 0;
  for (const reference of references) {
    const replacement = replacements.get(reference.path)!;
    result += html.slice(offset, reference.start) + replacement;
    offset = reference.end;
  }
  result += html.slice(offset);
  return { html: result, inlined: references.length > 0 };
}
