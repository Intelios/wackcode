/**
 * Built-in canvas: a board of artboards the user pans and zooms in the chat transcript, for exploring
 * UI alternatives side by side (themes, layout directions, desktop and phone).
 *
 * Three tools, all read-only (they change no file, so Plan and Ultra Plan allow them):
 *
 * - `canvas_guide` serves the authoring guide (`guide.ts`) on demand, so the long rules never ride
 *   in the system prompt. `/canvas` puts the whole guide in its prompt instead.
 * - `canvas` adds, revises and removes artboards. The markup stays in the call's arguments, which
 *   Pi saves, and the result's `details` say which artboard each argument became; the branch is
 *   the record (`state.ts`), so rewinds and restarts need nothing else. The desktop folds the
 *   same calls out of the snapshot and draws the board (`src/components/canvas/`).
 * - `canvas_look` asks the host to render artboards offscreen (`src-tauri/src/canvas_render.rs`)
 *   and returns screenshots (vision models only), a render report and optionally the resolved
 *   markup, so the agent can check its own work.
 *
 * Code chats only: never in `chatFactories`. Sub-agents never get these (`childToolNames`): a
 * child has no canvas to draw on. The user can switch them off together in Settings › Tools ›
 * Built-ins (`SWITCHABLE_BUILTIN_TOOLS`); saved canvases still render.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { BuiltinHost } from "../host.js";
import { CANVAS_GUIDE_SECTIONS, canvasGuide } from "./guide.js";
import {
  CANVAS_TOOL_NAME, MAX_ARTBOARDS_PER_CALL, MAX_ARTBOARD_HEIGHT, MAX_ARTBOARD_WIDTH, MAX_CANVAS_TITLE, MIN_ARTBOARD_SIZE,
  type Artboard, type Canvas, type CanvasDetails, canvasesFromBranch, describeArtboard, planCall, resolveArtboardHtml,
} from "./state.js";

export { CANVAS_TOOL_NAME } from "./state.js";
export const CANVAS_GUIDE_TOOL_NAME = "canvas_guide";
export const CANVAS_LOOK_TOOL_NAME = "canvas_look";
/** Read by the worker's tool filters: sub-agents never get them, and they switch off together. */
export const CANVAS_TOOL_NAMES = [CANVAS_TOOL_NAME, CANVAS_GUIDE_TOOL_NAME, CANVAS_LOOK_TOOL_NAME] as const;
export const MAX_LOOK_ARTBOARDS = 4;

/** Shared by all three tools, so the coding guide serves them once (it deduplicates). */
export const CANVAS_GUIDE_LINES: readonly string[] = [
  "Use the canvas for UI exploration: several mockups side by side that the user pans, zooms and compares — theme options, layout directions, desktop beside phone. A single chart, diagram or mockup doesn't need a canvas.",
  "Before your first canvas call in a chat, read canvas_guide (unless /canvas already gave you the guide). After drawing, check your artboards with canvas_look, then keep the reply short and ask which direction to take.",
];

const ARTBOARD_SCHEMA = {
  type: "object",
  additionalProperties: false,
  properties: {
    id: { type: "string", description: "An existing artboard's id, to revise it; only the fields you give change. Omit to add a new artboard." },
    name: { type: "string", description: "Short name the user will refer to, e.g. \"Dusk\" or \"Compact list\". Required for a new artboard." },
    group: { type: "string", description: "Row label; artboards with the same group sit in one row, e.g. \"Themes\" or \"Inbox\"." },
    width: { type: "number", description: `Width in CSS pixels (${MIN_ARTBOARD_SIZE}-${MAX_ARTBOARD_WIDTH}): 1440 desktop, 1280 laptop, 834 tablet, 390 phone. Required for a new artboard, except a variant, which takes its base's size.` },
    height: { type: "number", description: `Height in CSS pixels (${MIN_ARTBOARD_SIZE}-${MAX_ARTBOARD_HEIGHT}) for a fixed screen; omit to fit the content. A variant that gives no width takes its base's height too.` },
    html: { type: "string", description: "A self-contained HTML body with its own <style> and optional <script>. No network: inline every asset." },
    basedOn: { type: "string", description: "Id or name of another artboard on this canvas whose markup this one reuses; pair with css. Not with html." },
    css: { type: "string", description: "CSS appended after the markup, e.g. a :root { --… } theme override for a variant." },
    notes: { type: "string", description: "One line on what this artboard is betting on, shown under its name." },
  },
} as const;

export const CANVAS_PARAMS = {
  type: "object",
  additionalProperties: false,
  properties: {
    canvas: { type: "string", description: "An existing canvas's id. Omit to start a new canvas, which then needs a title." },
    title: { type: "string", description: `The canvas's title, 1-${MAX_CANVAS_TITLE} characters. Required to start one; on an existing canvas it renames it.` },
    artboards: { type: "array", maxItems: MAX_ARTBOARDS_PER_CALL, items: ARTBOARD_SCHEMA, description: `Artboards to add or revise, at most ${MAX_ARTBOARDS_PER_CALL} per call.` },
    remove: { type: "array", items: { type: "string" }, description: "Ids of artboards to remove." },
  },
} as const;

export const CANVAS_GUIDE_PARAMS = {
  type: "object",
  additionalProperties: false,
  properties: {
    sections: {
      type: "array",
      items: { type: "string", enum: [...CANVAS_GUIDE_SECTIONS] },
      description: "Which parts to read; omit for the whole guide. basics: how the canvas works. explore: drawing alternatives. themes: theme variants. fidelity: matching the project. craft: design quality. look: checking your work. example: a sample call.",
    },
  },
} as const;

export const CANVAS_LOOK_PARAMS = {
  type: "object",
  additionalProperties: false,
  properties: {
    canvas: { type: "string", description: "The canvas's id; omit when the chat has only one." },
    // No maxItems: Pi would refuse the whole call; the tool looks at the first few and names the rest.
    artboards: { type: "array", items: { type: "string" }, description: `Artboard ids or names, ${MAX_LOOK_ARTBOARDS} per call (any more are named for a next call); omit for the ones changed last.` },
    html: { type: "boolean", description: "Also return each artboard's resolved markup." },
  },
} as const;

interface BranchContext {
  sessionManager: { getBranch(): unknown[] };
}

function branchCanvases(ctx: unknown): Map<string, Canvas> {
  const context = ctx as BranchContext | undefined;
  return context?.sessionManager ? canvasesFromBranch(context.sessionManager.getBranch()) : new Map();
}

/** What the model reads after a call: what changed, and every artboard's id so it can revise. */
export function summarizeCall(canvas: Canvas, details: CanvasDetails, created: boolean): string {
  const byId = new Map(canvas.artboards.map((artboard) => [artboard.id, artboard]));
  const lines = [`${created ? "Started" : "Updated"} canvas “${canvas.title}” (id: ${canvas.id}).`];
  const added = details.applied.filter((item) => item.op === "add").map((item) => byId.get(item.id)).filter((item): item is Artboard => !!item);
  const updated = details.applied.filter((item) => item.op === "update").map((item) => byId.get(item.id)).filter((item): item is Artboard => !!item);
  if (added.length) lines.push(`Added: ${added.map(describeArtboard).join("; ")}.`);
  if (updated.length) lines.push(`Revised: ${updated.map(describeArtboard).join("; ")}.`);
  if (details.removed.length) lines.push(`Removed: ${details.removed.join(", ")}.`);
  lines.push(`On the canvas now (${canvas.artboards.length}): ${canvas.artboards.map((artboard) => artboard.id).join(", ") || "nothing"}.`);
  lines.push("The user sees it inline in the chat transcript and can expand it above the composer. Check what you changed with canvas_look, then keep your reply short instead of describing what they can see.");
  return lines.join("\n");
}

interface RenderedArtboard {
  id?: string;
  width?: number;
  height?: number;
  contentWidth?: number;
  contentHeight?: number;
  errors?: unknown;
  image?: { data?: string; mimeType?: string; width?: number; height?: number };
}

export interface CanvasLookArtboard {
  id: string;
  width: number;
  height: number;
  overflowX: number;
  overflowY: number;
  errors: string[];
}

export interface CanvasLookDetails {
  v: 1;
  canvas: string;
  artboards: CanvasLookArtboard[];
  unsupported?: "vision";
}

const MAX_REPORTED_ERRORS = 5;
const MAX_ERROR_CHARS = 300;

function reportLine(artboard: Artboard, rendered: CanvasLookArtboard): string {
  const fixed = artboard.height !== undefined;
  const parts = [`${describeArtboard(artboard)}: rendered ${rendered.width}×${rendered.height}.`];
  if (rendered.overflowY > 0) parts.push(fixed ? `Content overflows the bottom by ${rendered.overflowY} px.` : `Content is cut off ${rendered.overflowY} px below ${MAX_ARTBOARD_HEIGHT} px.`);
  if (rendered.overflowX > 0) parts.push(`Content overflows the right edge by ${rendered.overflowX} px (horizontal scroll).`);
  parts.push(rendered.errors.length ? `Script errors: ${rendered.errors.join(" | ")}` : "No script errors.");
  return parts.join(" ");
}

export function createCanvasExtension(host: BuiltinHost) {
  return function canvas(pi: ExtensionAPI) {
    pi.registerTool({
      name: CANVAS_GUIDE_TOOL_NAME,
      label: "Read canvas guide",
      description: "Read the canvas authoring guide: how artboards, groups and theme variants work, how to explore UI directions, matching the project's design, design quality and checking your work. Read it before your first canvas call in a chat.",
      promptSnippet: "read the canvas guide before drawing UI explorations",
      promptGuidelines: [...CANVAS_GUIDE_LINES],
      parameters: CANVAS_GUIDE_PARAMS,
      async execute(_toolCallId, params: unknown) {
        const sections = (params as { sections?: unknown } | undefined)?.sections;
        const wanted = Array.isArray(sections) ? sections.filter((value): value is string => typeof value === "string") : undefined;
        return { content: [{ type: "text" as const, text: canvasGuide(wanted) }], details: { v: 1 } };
      },
    });

    pi.registerTool({
      name: CANVAS_TOOL_NAME,
      label: "Draw on canvas",
      description: "Add, revise or remove artboards on a canvas: a board of full HTML mockups the user pans and zooms in the chat transcript to compare UI alternatives. Start a canvas with a title; later calls pass its id. Each artboard is a self-contained, offline HTML document at a fixed width.",
      promptSnippet: "draw UI mockups side by side on a pan-and-zoom canvas",
      promptGuidelines: [...CANVAS_GUIDE_LINES],
      parameters: CANVAS_PARAMS,
      async execute(_toolCallId, params: unknown, _signal, _onUpdate, ctx) {
        const planned = planCall(branchCanvases(ctx), params);
        if (!planned.ok) throw new Error(planned.error);
        return {
          content: [{ type: "text" as const, text: summarizeCall(planned.canvas, planned.details, planned.created) }],
          details: planned.details,
        };
      },
    });

    pi.registerTool({
      name: CANVAS_LOOK_TOOL_NAME,
      label: "Look at canvas",
      description: "Render artboards the way the canvas shows them and report back: a screenshot of each (for models that can see images), its rendered size, overflow and script errors, and optionally its resolved markup.",
      promptSnippet: "check how canvas artboards actually render",
      promptGuidelines: [...CANVAS_GUIDE_LINES],
      parameters: CANVAS_LOOK_PARAMS,
      async execute(_toolCallId, params: unknown, signal, _onUpdate, ctx) {
        const input = (params && typeof params === "object" ? params : {}) as { canvas?: unknown; artboards?: unknown; html?: unknown };
        const canvases = branchCanvases(ctx);
        let canvas: Canvas | undefined;
        if (typeof input.canvas === "string" && input.canvas.trim()) {
          canvas = canvases.get(input.canvas.trim());
          if (!canvas) throw new Error(`There is no canvas “${input.canvas.trim()}” in this chat.${canvases.size ? ` Canvases: ${[...canvases.keys()].join(", ")}.` : ""}`);
        } else if (canvases.size === 1) {
          canvas = [...canvases.values()][0];
        } else {
          throw new Error(canvases.size ? `This chat has ${canvases.size} canvases; say which one: ${[...canvases.keys()].join(", ")}.` : "There is no canvas in this chat yet. Draw one with the canvas tool first.");
        }
        let picked: Artboard[];
        if (Array.isArray(input.artboards) && input.artboards.length) {
          picked = [];
          for (const ref of input.artboards) {
            const wanted = typeof ref === "string" ? ref.trim() : "";
            const artboard = canvas.artboards.find((candidate) => candidate.id === wanted)
              ?? canvas.artboards.find((candidate) => candidate.name.toLowerCase() === wanted.toLowerCase());
            if (!artboard) throw new Error(`There is no artboard “${wanted}” on “${canvas.title}”. Artboards: ${canvas.artboards.map((item) => item.id).join(", ")}.`);
            if (!picked.includes(artboard)) picked.push(artboard);
          }
        } else {
          const recent = canvas.lastChanged.map((id) => canvas!.artboards.find((artboard) => artboard.id === id)).filter((item): item is Artboard => !!item);
          picked = recent.length ? recent : canvas.artboards;
        }
        if (!picked.length) throw new Error(`“${canvas.title}” has no artboards to look at.`);
        const later = picked.slice(MAX_LOOK_ARTBOARDS).map((artboard) => artboard.id);
        picked = picked.slice(0, MAX_LOOK_ARTBOARDS);

        const vision = host.supportsVision();
        const documents = picked.map((artboard) => ({ id: artboard.id, html: resolveArtboardHtml(canvas!, artboard) }));
        const result = await host.canvas({
          op: "render",
          images: vision,
          artboards: picked.map((artboard, index) => ({
            id: artboard.id,
            html: documents[index].html,
            width: artboard.width,
            ...(artboard.height !== undefined ? { height: artboard.height } : {}),
          })),
        }, signal) as { artboards?: RenderedArtboard[] };
        const rendered = Array.isArray(result?.artboards) ? result.artboards : [];

        const content: Array<{ type: "text"; text: string } | { type: "image"; data: string; mimeType: string }> = [];
        const report: CanvasLookArtboard[] = [];
        const lines = [`Rendered ${picked.length} artboard${picked.length === 1 ? "" : "s"} from “${canvas.title}” (id: ${canvas.id}).`];
        if (!vision) lines.push("This model can't receive images, so there are no screenshots; the report below is what the render found.");
        if (later.length) lines.push(`canvas_look shows ${MAX_LOOK_ARTBOARDS} artboards at a time; look again for: ${later.join(", ")}.`);
        for (const artboard of picked) {
          const raw = rendered.find((item) => item?.id === artboard.id);
          if (!raw) {
            lines.push(`${describeArtboard(artboard)}: the host returned no render.`);
            continue;
          }
          const width = Math.round(raw.width ?? artboard.width);
          const height = Math.round(raw.height ?? artboard.height ?? 0);
          const entry: CanvasLookArtboard = {
            id: artboard.id,
            width,
            height,
            overflowX: Math.max(0, Math.round((raw.contentWidth ?? width) - width)),
            overflowY: Math.max(0, Math.round((raw.contentHeight ?? height) - height)),
            errors: (Array.isArray(raw.errors) ? raw.errors : [])
              .filter((error): error is string => typeof error === "string")
              .slice(0, MAX_REPORTED_ERRORS)
              .map((error) => host.redact(error).slice(0, MAX_ERROR_CHARS)),
          };
          report.push(entry);
          lines.push(reportLine(artboard, entry));
        }
        content.push({ type: "text", text: lines.join("\n") });
        for (const artboard of picked) {
          const image = rendered.find((item) => item?.id === artboard.id)?.image;
          if (!vision || !image?.data) continue;
          content.push({ type: "text", text: `Screenshot of ${artboard.id} (“${artboard.name}”):` });
          content.push({ type: "image", data: image.data, mimeType: image.mimeType ?? "image/png" });
        }
        if (input.html === true) {
          for (const [index, artboard] of picked.entries()) {
            content.push({ type: "text", text: `Markup of ${artboard.id} (“${artboard.name}”), resolved:\n\n\`\`\`html\n${documents[index].html}\n\`\`\`` });
          }
        }
        return {
          content,
          details: { v: 1, canvas: canvas.id, artboards: report, ...(vision ? {} : { unsupported: "vision" as const }) } satisfies CanvasLookDetails,
        };
      },
    });
  };
}
