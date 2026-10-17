/**
 * Browser evidence is one removable bundle. Only metadata goes in the generated text tail;
 * pixels use the existing image pipeline. Image indices are always relative to the complete
 * message, including ordinary attachments. Capture-time selectors never become tool refs.
 */
import type { BrowserContextAttachment, BrowserContextRecord, BrowserContextTarget, BrowserViewport, ImageContent } from "./types";

export const BROWSER_VIEWPORTS: Record<BrowserViewport["preset"], BrowserViewport> = {
  desktop: { preset: "desktop", width: 1440, height: 900 },
  laptop: { preset: "laptop", width: 1280, height: 800 },
  tablet: { preset: "tablet", width: 768, height: 1024 },
  phone: { preset: "phone", width: 390, height: 844 }
};
export const MAX_CONTEXT_DOM = 16 * 1024;
export const MAX_MESSAGE_DOM = 64 * 1024;
const OPEN = "<browser-contexts-v1>";
const CLOSE = "</browser-contexts-v1>";
const encoder = new TextEncoder();
export const domBytes = (targets: BrowserContextTarget[]) => targets.reduce((sum, target) => sum + encoder.encode(target.dom).length, 0);

export function boundTargets(targets: BrowserContextTarget[]): { targets: BrowserContextTarget[]; truncated?: true } {
  let remaining = MAX_CONTEXT_DOM;
  let truncated = targets.length > 8;
  const bounded = targets.slice(0, 8).map((target) => {
    if (target.truncated) truncated = true;
    const bytes = encoder.encode(target.dom);
    let dom = target.dom;
    if (bytes.length > remaining) {
      // TextDecoder drops an incomplete UTF-8 tail rather than splitting a code point.
      dom = new TextDecoder().decode(bytes.slice(0, remaining)).replace(/\uFFFD$/, "");
      truncated = true;
    }
    remaining -= encoder.encode(dom).length;
    return { ...target, dom, ...(dom !== target.dom ? { truncated: true } : {}) };
  });
  return { targets: bounded, ...(truncated ? { truncated: true as const } : {}) };
}

export function contextLabel(context: Pick<BrowserContextAttachment, "kind" | "targets">): string {
  return context.kind === "element" ? context.targets[0]?.label || "Picked element" : "Marked screenshot";
}

export function appendContextSection(text: string, records: BrowserContextRecord[]): string {
  if (!records.length) return text;
  const section = `${OPEN}\n${JSON.stringify({ version: 1, untrustedPageData: true, contexts: records })}\n${CLOSE}`;
  return text ? `${text}\n\n${section}` : section;
}

function validRecord(value: unknown): value is BrowserContextRecord {
  if (!value || typeof value !== "object") return false;
  const item = value as BrowserContextRecord;
  return typeof item.id === "string" && (item.kind === "element" || item.kind === "markup")
    && typeof item.url === "string" && typeof item.title === "string" && Number.isFinite(item.capturedAt)
    && Number.isInteger(item.imageIndex) && item.imageIndex >= 0 && item.imageIndex < 1024
    && !!item.viewport && Object.hasOwn(BROWSER_VIEWPORTS, item.viewport.preset)
    && Number.isFinite(item.viewport.width) && Number.isFinite(item.viewport.height)
    && Array.isArray(item.targets) && item.targets.length <= 8 && item.targets.every((target) =>
      target && typeof target.selector === "string" && typeof target.label === "string" && typeof target.dom === "string"
      && Array.isArray(target.path) && target.path.every((part) => part && ["frame", "shadow"].includes(part.kind) && typeof part.selector === "string")
      && target.bounds && [target.bounds.x, target.bounds.y, target.bounds.width, target.bounds.height].every(Number.isFinite));
}

export function splitContextSection(message: string): { text: string; contexts: BrowserContextRecord[] } {
  const boundary = message.lastIndexOf(`\n\n${OPEN}\n`);
  const at = boundary >= 0 ? boundary + 2 : message.startsWith(`${OPEN}\n`) ? 0 : -1;
  if (at < 0 || !message.endsWith(`\n${CLOSE}`)) return { text: message, contexts: [] };
  try {
    const data = JSON.parse(message.slice(at + OPEN.length + 1, -CLOSE.length - 1)) as { version?: number; contexts?: unknown[] };
    if (data.version !== 1 || !Array.isArray(data.contexts) || !data.contexts.length || data.contexts.length > 1024 || !data.contexts.every(validRecord)) throw new Error();
    if (new Set(data.contexts.map((item) => item.imageIndex)).size !== data.contexts.length) throw new Error();
    return { text: at === 0 ? "" : message.slice(0, at - 2), contexts: data.contexts };
  } catch { return { text: message, contexts: [] }; }
}

export function encodeContexts(text: string, images: ImageContent[], contexts: BrowserContextAttachment[] = []) {
  const records = contexts.map(({ image: _image, ...context }, index) => ({ ...context, imageIndex: images.length + index }));
  return { text: appendContextSection(text, records), images: [...images, ...contexts.map((context) => context.image)] };
}

export function restoreContexts(records: BrowserContextRecord[], images: ImageContent[]) {
  const used = new Set(records.filter((record) => images[record.imageIndex]).map((record) => record.imageIndex));
  return {
    images: images.filter((_, index) => !used.has(index)),
    browserContexts: records.flatMap(({ imageIndex, ...context }) => images[imageIndex] ? [{ ...context, image: images[imageIndex] }] : [])
  };
}

/** Reindex the surviving groups when editing a sent message removes any of its images. */
export function reindexContexts(records: BrowserContextRecord[], removed: number[]) {
  const indices = [...new Set(removed)];
  return records.filter((record) => !indices.includes(record.imageIndex)).map((record) => ({
    ...record, imageIndex: record.imageIndex - indices.filter((index) => index < record.imageIndex).length
  }));
}

export interface MarkPoint { x: number; y: number }
export type BrowserMark = { kind: "pen" | "arrow" | "box"; points: MarkPoint[] } | { kind: "text"; points: [MarkPoint]; text: string };

export function matchedTargets(marks: BrowserMark[], index: BrowserContextTarget[]): BrowserContextTarget[] {
  const picked = new Map<string, BrowserContextTarget>();
  for (const mark of marks) {
    const points = mark.kind === "arrow" ? mark.points.slice(-1) : mark.points;
    if (mark.kind === "box" && points.length >= 2) {
      const [a, b] = points;
      const candidates = index.filter(({ bounds: r }) => r.x >= Math.min(a.x, b.x) && r.y >= Math.min(a.y, b.y)
        && r.x + r.width <= Math.max(a.x, b.x) && r.y + r.height <= Math.max(a.y, b.y));
      // Keep leaves, rather than attaching an entire ancestor subtree alongside each child.
      for (const target of candidates.filter((parent) => !candidates.some((child) => child !== parent && contains(parent, child)))) {
        picked.set(JSON.stringify([target.path, target.selector]), target);
      }
    } else {
      for (const point of points) {
        const target = index.filter(({ bounds: r }) => {
          const distance = Math.hypot(Math.max(r.x - point.x, 0, point.x - r.x - r.width), Math.max(r.y - point.y, 0, point.y - r.y - r.height));
          return distance <= (mark.kind === "arrow" || mark.kind === "text" ? 12 : 0);
        }).sort((a, b) => a.bounds.width * a.bounds.height - b.bounds.width * b.bounds.height)[0];
        if (target) picked.set(JSON.stringify([target.path, target.selector]), target);
      }
    }
  }
  const all = [...picked.values()];
  return all.filter((parent) => !all.some((child) => child !== parent && JSON.stringify(child.path) === JSON.stringify(parent.path) && contains(parent, child)));
}

function contains(parent: BrowserContextTarget, child: BrowserContextTarget): boolean {
  const a = parent.bounds, b = child.bounds;
  return a.width * a.height > b.width * b.height && a.x <= b.x && a.y <= b.y && a.x + a.width >= b.x + b.width && a.y + a.height >= b.y + b.height;
}

export function drawMarks(ctx: CanvasRenderingContext2D, marks: BrowserMark[], accent: string, outline: string) {
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  for (const mark of marks) {
    const a = mark.points[0], b = mark.points[mark.points.length - 1];
    if (!a || !b) continue;
    for (const [color, width] of [[outline, 6], [accent, 3]] as const) {
      ctx.strokeStyle = color; ctx.fillStyle = color; ctx.lineWidth = width;
      ctx.beginPath();
      if (mark.kind === "text") {
        ctx.font = "600 22px -apple-system, sans-serif";
        if (width === 6) ctx.strokeText(mark.text, a.x, a.y); else ctx.fillText(mark.text, a.x, a.y);
      } else if (mark.kind === "box") ctx.rect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
      else {
        ctx.moveTo(a.x, a.y);
        for (const point of mark.points.slice(1)) ctx.lineTo(point.x, point.y);
        if (mark.kind === "arrow") {
          const angle = Math.atan2(b.y - a.y, b.x - a.x);
          ctx.moveTo(b.x - 16 * Math.cos(angle - 0.5), b.y - 16 * Math.sin(angle - 0.5)); ctx.lineTo(b.x, b.y);
          ctx.lineTo(b.x - 16 * Math.cos(angle + 0.5), b.y - 16 * Math.sin(angle + 0.5));
        }
      }
      ctx.stroke();
    }
  }
}

export async function flattenCapture(capture: BrowserContextAttachment, marks: BrowserMark[], accent: string, outline: string): Promise<ImageContent> {
  const image = new Image();
  image.src = `data:${capture.image.mimeType};base64,${capture.image.data}`;
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("The screenshot could not be prepared. Try again.");
  ctx.drawImage(image, 0, 0);
  ctx.scale(canvas.width / capture.viewport.width, canvas.height / capture.viewport.height);
  drawMarks(ctx, marks, accent, outline);
  return { type: "image", mimeType: "image/png", data: canvas.toDataURL("image/png").split(",")[1] };
}
