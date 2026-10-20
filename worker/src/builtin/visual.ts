/**
 * Built-in `show_visual` tool: the agent answers part of a turn with a small, self-contained
 * HTML document — a chart, a comparison, a diagram, a mockup of proposed UI, a tiny calculator
 * — drawn inline in the conversation instead of described in prose.
 *
 * The desktop draws it (`src/components/VisualCard.tsx`): the call renders as a card where it
 * lands, so the model writes prose around a visual rather than duplicating its contents. The
 * HTML itself stays in the call's arguments, which Pi persists, so saved history and cold
 * reopening need nothing extra from this tool.
 *
 * The document runs sandboxed with no network at all (`src-tauri/src/visuals.rs`), so the tool
 * only has to keep the payload sane: a short title and a body under `MAX_VISUAL_HTML_BYTES`.
 * Scripts are allowed on purpose — that is what makes a visual interactive — but they can only
 * reach the host through `wackcode.send`, which the desktop gates (see docs/security.md).
 *
 * Chat mode loads it like any other WackCode tool; sub-agents never do (`childToolNames`), since
 * a child has no transcript to draw in. The user can switch it off in Settings › Tools ›
 * Built-ins (`SWITCHABLE_BUILTIN_TOOLS`); saved visuals still render.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const SHOW_VISUAL_TOOL_NAME = "show_visual";
/** Read by the worker's tool filters: sub-agents never get it. */
export const VISUAL_TOOL_NAMES = [SHOW_VISUAL_TOOL_NAME] as const;
export const MAX_VISUAL_TITLE = 60;
export const MAX_VISUAL_HTML_BYTES = 64 * 1024;

/** The CSS variables the frame defines before the model's markup runs (`visuals.rs`). */
export const VISUAL_THEME_VARIABLES = [
  "--bg", "--surface", "--text", "--text-soft", "--text-dim", "--border",
  "--accent", "--accent-text", "--on-accent", "--font", "--mono", "--radius",
] as const;

export const SHOW_VISUAL_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["title", "html"],
  properties: {
    title: {
      type: "string",
      description: "A short title for the card, 1-60 characters, e.g. \"Latency by region\".",
    },
    html: {
      type: "string",
      description:
        "A self-contained HTML body: markup, optional <style> and optional <script>. No external resources of any kind — the document has no network access, so inline everything (inline SVG, data: URLs). Use the app's CSS variables for colour.",
    },
  },
} as const;

/**
 * What the model must know about the tool, in one place: the coding guide re-serves these
 * `promptGuidelines`, and Chat mode's guide includes the same words (`buildChatGuide`).
 */
export const VISUAL_GUIDE_LINES: readonly string[] = [
  "Use show_visual when seeing an answer beats reading it: a chart or a comparison, a diagram or flow, a mockup of proposed UI, a small interactive calculator or picker. Keep plain lists, code and short answers as Markdown.",
  `A visual is a self-contained HTML body: <style> and <script> are allowed and run sandboxed. It has no network at all, so inline everything (inline SVG, data: URLs) and never reference an external file, font, image or URL.`,
  `Match the app's look with its CSS variables — ${VISUAL_THEME_VARIABLES.join(", ")} — which follow the user's theme. Never hard-code colours; the document also answers the app's own --wc-* names, so styles copied from WackCode's CSS work unchanged.`,
  "Design for about 720 px wide and under 640 px tall: the card grows to fit its content and scrolls past that.",
  "wackcode.send(text) sends that text as the user's next message when they click something. Wire it only to explicit buttons or choices the user presses — never on load or on a timer, and never put words in the user's mouth.",
  "One visual per idea. The card shows your title, so keep the prose around it short and point at it instead of repeating its contents.",
];

type Normalized = { ok: true; title: string; html: string } | { ok: false; error: string };

export function normalizeShowVisualParams(input: unknown): Normalized {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "Expected an object with a title and html body." };
  }
  const { title, html } = input as Record<string, unknown>;
  if (typeof title !== "string" || !title.trim()) return { ok: false, error: "title must be a short non-empty string." };
  const trimmed = title.trim().replace(/\s+/g, " ");
  if (trimmed.length > MAX_VISUAL_TITLE) {
    return { ok: false, error: `title must be at most ${MAX_VISUAL_TITLE} characters.` };
  }
  if (typeof html !== "string" || !html.trim()) return { ok: false, error: "html must be a non-empty HTML body." };
  const bytes = Buffer.byteLength(html, "utf8");
  if (bytes > MAX_VISUAL_HTML_BYTES) {
    return {
      ok: false,
      error: `html is ${formatSize(bytes)}, over the ${formatSize(MAX_VISUAL_HTML_BYTES)} limit. Draw the same idea with less markup, or answer in prose instead.`,
    };
  }
  return { ok: true, title: trimmed, html };
}

/** "62.1 KB" — the size the result and the refusal both quote. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export interface ShowVisualDetails {
  /** Versioned result shape, like the other WackCode cards. */
  v: 1;
  title: string;
}

export function createVisualExtension() {
  return function showVisual(pi: ExtensionAPI) {
    pi.registerTool({
      name: SHOW_VISUAL_TOOL_NAME,
      label: "Show visual",
      description:
        "Answer part of the turn with a small self-contained HTML document drawn inline in the conversation: a chart, a comparison, a diagram, a mockup of proposed UI, or a tiny interactive widget. It is not a web page: it sits in the chat in the app's own theme, sized to its content. Scripts and styles are allowed; there is no network, so inline every asset. Refer to the visual in your reply instead of repeating what it shows.",
      promptSnippet: "draw a chart, diagram, mockup or small interactive widget inline in the conversation",
      promptGuidelines: [...VISUAL_GUIDE_LINES],
      parameters: SHOW_VISUAL_PARAMS,
      async execute(_toolCallId, params: unknown) {
        const parsed = normalizeShowVisualParams(params);
        if (!parsed.ok) throw new Error(parsed.error);
        const size = formatSize(Buffer.byteLength(parsed.html, "utf8"));
        return {
          content: [{
            type: "text" as const,
            text: `Shown "${parsed.title}" inline in the conversation (${size}). The user can see and interact with it now; refer to it briefly in your reply instead of repeating its contents.`,
          }],
          details: { v: 1, title: parsed.title } satisfies ShowVisualDetails,
        };
      },
    });
  };
}
