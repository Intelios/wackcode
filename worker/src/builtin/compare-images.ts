/**
 * A comparison records two saved image references, never copies pixels or reads paths. Use
 * the display branch (including context edits), not Pi's compacted model context: an older
 * screenshot remains available after compaction. Only parent chats load this extension.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { EntryLike } from "../tree.js";
import { resolveSavedImage } from "../saved-images.js";
import { parseImageReference } from "./image-references.js";
import { MAX_VISUAL_TITLE } from "./visual.js";

export const COMPARE_IMAGES_TOOL_NAME = "compare_images";
export const MAX_COMPARISON_LABEL = 32;

export const COMPARE_IMAGES_GUIDE_LINES: readonly string[] = [
  "compare_images shows image changes/alternatives from supplied references. Do not repeat its images.",
];

export const COMPARE_IMAGES_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["before", "after"],
  properties: {
    before: { type: "string", description: "The saved wackcode-image://entry/... reference for the left image." },
    after: { type: "string", description: "The saved wackcode-image://entry/... reference for the right image." },
    title: { type: "string", description: "Optional short accessible title, at most 60 characters." },
    beforeLabel: { type: "string", description: "Optional left label, at most 32 characters; defaults to Before." },
    afterLabel: { type: "string", description: "Optional right label, at most 32 characters; defaults to After." },
  },
} as const;

export interface ImageComparisonDetails {
  v: 1;
  before: string;
  after: string;
  title: string;
  beforeLabel: string;
  afterLabel: string;
}

const clean = (text: string) => text.replace(/\s+/g, " ").trim();

export function normalizeCompareImagesParams(input: unknown): { ok: true; comparison: ImageComparisonDetails } | { ok: false; error: string } {
  if (!input || typeof input !== "object" || Array.isArray(input)) return { ok: false, error: "Expected two saved image references, before and after." };
  const value = input as Record<string, unknown>;
  for (const side of ["before", "after"] as const) {
    if (!parseImageReference(value[side])) return { ok: false, error: `${side} must be a saved wackcode-image reference from this chat. Read the image first and copy its supplied reference.` };
  }
  for (const [key, limit] of [["title", MAX_VISUAL_TITLE], ["beforeLabel", MAX_COMPARISON_LABEL], ["afterLabel", MAX_COMPARISON_LABEL]] as const) {
    if (value[key] !== undefined && (typeof value[key] !== "string" || !clean(value[key]) || clean(value[key]).length > limit)) {
      return { ok: false, error: `${key} must be a non-empty string of at most ${limit} characters.` };
    }
  }
  const beforeLabel = value.beforeLabel === undefined ? "Before" : clean(value.beforeLabel as string);
  const afterLabel = value.afterLabel === undefined ? "After" : clean(value.afterLabel as string);
  const title = value.title === undefined ? `${beforeLabel} / ${afterLabel}`.slice(0, MAX_VISUAL_TITLE).trim() : clean(value.title as string);
  return { ok: true, comparison: { v: 1, before: value.before as string, after: value.after as string, title, beforeLabel, afterLabel } };
}

export function savedComparisonImage(reference: string, branch: EntryLike[]): boolean {
  return resolveSavedImage(reference, branch) !== undefined;
}

export function createCompareImagesExtension() {
  return function compareImages(pi: ExtensionAPI) {
    pi.registerTool({
      name: COMPARE_IMAGES_TOOL_NAME,
      label: "Compare images",
      description: "Show two saved chat images in an interactive comparison, inline in the answer. The user drags a Before/After divider or expands to zoom and pan both images together. Supply existing image references, never paths or URLs. Labels and title are optional.",
      promptSnippet: "compare saved images",
      promptGuidelines: [...COMPARE_IMAGES_GUIDE_LINES],
      parameters: COMPARE_IMAGES_PARAMS,
      async execute(_toolCallId, input: unknown, signal, _onUpdate, ctx) {
        const parsed = normalizeCompareImagesParams(input);
        if (!parsed.ok) throw new Error(parsed.error);
        if (signal?.aborted) throw new Error("Image comparison was stopped.");
        const branch = ctx.sessionManager.getBranch();
        for (const side of ["before", "after"] as const) {
          if (!savedComparisonImage(parsed.comparison[side], branch)) throw new Error(`The ${side} image is not available on this chat's current branch. Select a saved attachment or tool image from this branch.`);
        }
        return {
          content: [{ type: "text" as const, text: `Shown image comparison “${parsed.comparison.title}” inline. The user can drag the divider and expand it. Do not repeat the images or announce the viewer.` }],
          details: parsed.comparison,
        };
      },
    });
  };
}
