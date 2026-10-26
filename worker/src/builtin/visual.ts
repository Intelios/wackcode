/**
 * Inline visual replies and their private preview use the same sandboxed frame as the desktop.
 * Markup stays in the call arguments, except local assets: their checked bytes are inlined into
 * details.html so a saved visual survives files moving. Snapshots omit that large document and
 * the card retrieves it on demand. Sub-agents never get these tools (they have no transcript).
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { BuiltinHost } from "./host.js";
import { inlineVisualImages } from "./visual-images.js";

export const SHOW_VISUAL_TOOL_NAME = "show_visual";
export const PREVIEW_VISUAL_TOOL_NAME = "preview_visual";
/** Read by the worker's tool filters: sub-agents never get either tool. */
export const VISUAL_TOOL_NAMES = [SHOW_VISUAL_TOOL_NAME, PREVIEW_VISUAL_TOOL_NAME] as const;
export const MAX_VISUAL_TITLE = 60;
export const MAX_VISUAL_HTML_BYTES = 128 * 1024;
const MAX_FIT_HEIGHT = 20_000;
const MEASURE_WIDTHS = [360, 560, 720] as const;

/** The CSS variables the frame defines before the model's markup runs (`visual_frame.html`). */
export const VISUAL_THEME_VARIABLES = [
  "--bg", "--surface", "--text", "--text-soft", "--text-dim", "--border",
  "--accent", "--accent-text", "--on-accent", "--font", "--mono", "--radius",
  "--danger", "--warning", "--success", "--info", "--chart-1", "--chart-2", "--chart-3", "--chart-4", "--chart-5", "--chart-6",
] as const;

const HTML_PARAM = {
  type: "string",
  description: "A self-contained HTML body with optional <style> and <script>. No network resources. Inline SVG and data: URLs work; local images may use absolute file paths. Colour with the app's CSS variables.",
} as const;

export const SHOW_VISUAL_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["title", "html"],
  properties: {
    title: { type: "string", description: "A short accessible title, 1–60 characters, e.g. \"Latency by region\"." },
    html: HTML_PARAM,
  },
} as const;

export const PREVIEW_VISUAL_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["html"],
  properties: {
    html: HTML_PARAM,
    width: { type: "number", description: "Preview width in pixels, 240–1600 (default 720)." },
    appearance: { type: "string", enum: ["light", "dark"], description: "Optional colour scheme to check; omitted uses the app's current appearance." },
  },
} as const;

/** Shared verbatim with Chat mode's guide, since Pi drops tool guidelines for custom prompts. */
export const VISUAL_GUIDE_LINES: readonly string[] = [
  "Charts, comparisons, diagrams, UI mockups and widgets use show_visual; plain lists, code and short answers stay Markdown.",
  "HTML body, optional <style>/<script>, no network. Use inline SVG/data: or absolute image paths in attributes/url(); images persist. Chat mode: scratchpad only.",
  `Theme variables: ${VISUAL_THEME_VARIABLES.join(", ")}. --chart-1 follows the accent; other chart/semantic colours retain meaning. --wc-* and shadcn aliases work too. No hard-coded colours.`,
  "About 720 px wide, content fits up to 20,000 px tall. Transparent html/body; no outer card/border/title banner/side padding. Filled inner boxes: 16 px padding, var(--radius). Charts: fixed pixel heights. No vh; no height:100% on html/body.",
  "Before show_visual, preview_visual the html and fix its screenshot, contentHeight and script errors. Optional width: 240–1600 (default 720); appearance: light/dark (default current). Preview cannot publish/send.",
  "wackcode.send(text) sends a user message only on explicit button/choice clicks; never on load/timers or inventing the user's words. One visual per idea. Don't announce or restate the page.",
];

type HtmlValidation = { ok: true; html: string } | { ok: false; error: string };
type Normalized = { ok: true; title: string; html: string } | { ok: false; error: string };

/** Scan styles only: an image's base64 or explanatory prose may coincidentally contain "vh". */
export function hasViewportHeightStyles(html: string): boolean {
  const styles: string[] = [];
  for (const match of html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style\s*>/gi)) styles.push(match[1]);
  for (const match of html.matchAll(/\bstyle\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi)) styles.push(match[1] ?? match[2] ?? match[3]);
  return styles.some((style) => {
    const css = style.replace(/\/\*[\s\S]*?\*\//g, "").replace(/data:[^,\s"')]*;base64,[a-z\d+/=\s]+/gi, "");
    return /(?:\b\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?vh\b/i.test(css);
  });
}

function normalizeHtml(html: unknown): HtmlValidation {
  if (typeof html !== "string" || !html.trim()) return { ok: false, error: "html must be a non-empty HTML body." };
  const bytes = Buffer.byteLength(html, "utf8");
  if (bytes > MAX_VISUAL_HTML_BYTES) return { ok: false, error: `html is ${formatSize(bytes)}, over the ${formatSize(MAX_VISUAL_HTML_BYTES)} limit. Draw the same idea with less markup, or answer in prose instead.` };
  if (hasViewportHeightStyles(html)) return { ok: false, error: "Visuals fit their content, so vh units cannot be used in styles. Replace viewport heights with fixed pixels for charts, or let the content determine its height, then preview again." };
  return { ok: true, html };
}

export function normalizeShowVisualParams(input: unknown): Normalized {
  if (typeof input !== "object" || input === null || Array.isArray(input)) return { ok: false, error: "Expected an object with a title and html body." };
  const { title, html } = input as Record<string, unknown>;
  if (typeof title !== "string" || !title.trim()) return { ok: false, error: "title must be a short non-empty string." };
  const trimmed = title.trim().replace(/\s+/g, " ");
  if (trimmed.length > MAX_VISUAL_TITLE) return { ok: false, error: `title must be at most ${MAX_VISUAL_TITLE} characters.` };
  const validated = normalizeHtml(html);
  return validated.ok ? { ok: true, title: trimmed, html: validated.html } : validated;
}

/** "62.1 KB" — the size the result and the refusal both quote. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface ShowVisualDetails {
  v: 1;
  title: string;
  heights?: Array<{ width: number; height: number }>;
  inlined?: true;
  /** Only when local images were copied; omitted from snapshots and fetched on demand. */
  html?: string;
}

interface RenderedVisual {
  id?: string;
  width?: number;
  height?: number;
  contentHeight?: number;
  errors?: unknown;
  image?: { data?: string; mimeType?: string };
}

export function createVisualExtension(host: BuiltinHost, options: { isChat?: () => boolean } = {}) {
  const prepare = (html: string) => inlineVisualImages(html, { isChat: options.isChat?.(), cwd: host.workspace() });
  return function visual(pi: ExtensionAPI) {
    pi.registerTool({
      name: SHOW_VISUAL_TOOL_NAME,
      label: "Show visual",
      description: "Answer with a self-contained HTML visual drawn inline in the conversation: a chart, comparison, diagram, proposed UI or interactive widget. It follows the app's theme and fits its content. Preview first with preview_visual and fix the screenshot. Scripts and styles are allowed, with no network; inline assets or use absolute local image paths. Don't announce or restate the page.",
      promptSnippet: "draw charts, diagrams, mockups or widgets in the conversation",
      promptGuidelines: [...VISUAL_GUIDE_LINES],
      parameters: SHOW_VISUAL_PARAMS,
      async execute(_toolCallId, params: unknown, signal) {
        const parsed = normalizeShowVisualParams(params);
        if (!parsed.ok) throw new Error(parsed.error);
        const document = await prepare(parsed.html);
        const details: ShowVisualDetails = { v: 1, title: parsed.title, ...(document.inlined ? { inlined: true, html: document.html } : {}) };
        try {
          const result = await host.canvas({ op: "render", images: false, artboards: MEASURE_WIDTHS.map((width) => ({ id: `visual-${width}`, kind: "preview", html: document.html, width })) }, signal) as { artboards?: RenderedVisual[] };
          const heights = MEASURE_WIDTHS.flatMap((width) => {
            const rendered = result?.artboards?.find((item) => item.id === `visual-${width}`);
            const height = rendered?.contentHeight;
            return typeof height === "number" && Number.isFinite(height) && height > 0 ? [{ width, height: Math.min(MAX_FIT_HEIGHT, Math.ceil(height)) }] : [];
          });
          if (heights.length) details.heights = heights;
        } catch (error) {
          if (signal?.aborted) throw error;
          // Measuring improves first paint, but a renderer failure must not lose the answer.
        }
        return {
          content: [{ type: "text" as const, text: `Shown "${parsed.title}" inline in the conversation (${formatSize(Buffer.byteLength(document.html, "utf8"))}). The user can see and interact with it now. Don't announce or restate the page.` }],
          details,
        };
      },
    });
    pi.registerTool({
      name: PREVIEW_VISUAL_TOOL_NAME,
      label: "Preview visual",
      description: "Privately render an inline HTML visual before publishing it with show_visual. Returns a PNG for vision models, contentHeight and script errors. Uses the same sandbox and theme, with optional light/dark appearance. Fix what the preview shows before publishing.",
      promptSnippet: "check a visual before publishing",
      promptGuidelines: [...VISUAL_GUIDE_LINES],
      parameters: PREVIEW_VISUAL_PARAMS,
      async execute(_toolCallId, params: unknown, signal) {
        if (!params || typeof params !== "object" || Array.isArray(params)) throw new Error("Expected an object with an html body.");
        const input = params as Record<string, unknown>;
        const parsed = normalizeHtml(input.html);
        if (!parsed.ok) throw new Error(parsed.error);
        const width = input.width === undefined ? 720 : input.width;
        if (typeof width !== "number" || !Number.isFinite(width) || width < 240 || width > 1600) throw new Error("Preview width must be between 240 and 1600 pixels.");
        if (input.appearance !== undefined && input.appearance !== "light" && input.appearance !== "dark") throw new Error("Preview appearance must be light or dark.");
        const document = await prepare(parsed.html);
        const vision = host.supportsVision();
        const result = await host.canvas({ op: "render", images: vision, ...(input.appearance ? { appearance: input.appearance } : {}), artboards: [{ id: "visual-preview", kind: "preview", html: document.html, width }] }, signal) as { artboards?: RenderedVisual[] };
        const rendered = result?.artboards?.find((item) => item.id === "visual-preview");
        if (!rendered) throw new Error("The host returned no visual preview. Try preview_visual again.");
        const contentHeight = typeof rendered.contentHeight === "number" && Number.isFinite(rendered.contentHeight) ? Math.ceil(rendered.contentHeight) : null;
        const errors = (Array.isArray(rendered.errors) ? rendered.errors : []).filter((error): error is string => typeof error === "string").slice(0, 5).map((error) => host.redact(error).slice(0, 300));
        const content: Array<{ type: "text"; text: string } | { type: "image"; data: string; mimeType: string }> = [{ type: "text", text: `Visual preview at ${Math.round(width)} px wide. contentHeight: ${contentHeight ?? "unavailable"}. ${errors.length ? `Script errors: ${errors.join(" | ")}` : "No script errors."}${vision ? "" : " This model cannot receive images; use the height and error report."}` }];
        if (vision && rendered.image?.data) content.push({ type: "image", data: rendered.image.data, mimeType: rendered.image.mimeType ?? "image/png" });
        return { content, details: { v: 1, width: Math.round(width), contentHeight, errors, ...(vision ? {} : { unsupported: "vision" }) } };
      },
    });
  };
}
