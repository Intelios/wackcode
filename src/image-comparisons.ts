/** Accepted tool results are the record; partial arguments never create a comparison. */
import type { NormalizedBlock } from "./types";
import { parseReplyImageReference } from "./reply-images";

export const COMPARE_IMAGES_TOOL_NAME = "compare_images";

export interface ImageComparisonDetails {
  v: 1;
  before: string;
  after: string;
  title: string;
  beforeLabel: string;
  afterLabel: string;
}

export function completedImageComparison(result?: NormalizedBlock): ImageComparisonDetails | undefined {
  if (!result || result.isError || !result.details || typeof result.details !== "object" || Array.isArray(result.details)) return undefined;
  const value = result.details as Record<string, unknown>;
  if (value.v !== 1 || typeof value.before !== "string" || !parseReplyImageReference(value.before)
    || typeof value.after !== "string" || !parseReplyImageReference(value.after)) return undefined;
  for (const [key, limit] of [["title", 60], ["beforeLabel", 32], ["afterLabel", 32]] as const) {
    if (typeof value[key] !== "string" || !value[key].trim() || value[key].length > limit) return undefined;
  }
  return value as unknown as ImageComparisonDetails;
}
