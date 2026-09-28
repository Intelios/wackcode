/**
 * Computer-use screenshots are the largest thing a chat sends, and a session that verifies an
 * app takes many. Before each model call only the newest few stay; older ones become a short
 * placeholder. The session itself is untouched — this rewrites the copy Pi sends.
 *
 * Removal happens in whole chunks: the count of removed screenshots only changes every
 * `PRUNE_CHUNK` captures, so between chunk boundaries the rewritten prefix is byte-identical
 * from call to call and the provider's prompt cache keeps hitting.
 */

export const KEEP_SCREENSHOTS = 3;
export const PRUNE_CHUNK = 5;
export const PRUNED_SCREENSHOT_TEXT = "[Earlier computer screenshot removed to save context. Take a new one if you need to see the app again.]";

interface ContentBlock {
  type?: unknown;
}

interface ToolResultLike {
  role?: unknown;
  toolName?: unknown;
  content?: unknown;
}

/** How many of `total` screenshots to remove, oldest first. */
export function screenshotsToRemove(total: number, keep = KEEP_SCREENSHOTS, chunk = PRUNE_CHUNK): number {
  return Math.floor(Math.max(0, total - keep) / chunk) * chunk;
}

function isImage(block: unknown): boolean {
  return typeof block === "object" && block !== null && (block as ContentBlock).type === "image";
}

function screenshotImages(message: unknown, toolNames: ReadonlySet<string>): number {
  if (typeof message !== "object" || message === null) return 0;
  const raw = message as ToolResultLike;
  if (raw.role !== "toolResult" || typeof raw.toolName !== "string" || !toolNames.has(raw.toolName)) return 0;
  return Array.isArray(raw.content) ? raw.content.filter(isImage).length : 0;
}

/**
 * Returns a new array with the oldest screenshots of `toolNames` replaced, or `undefined` when
 * nothing needs removing. Messages it changes are shallow copies; the input is never mutated.
 */
export function pruneScreenshots<T>(messages: readonly T[], toolNames: ReadonlySet<string>): T[] | undefined {
  const total = messages.reduce((sum, message) => sum + screenshotImages(message, toolNames), 0);
  let remaining = screenshotsToRemove(total);
  if (remaining === 0) return undefined;
  return messages.map((message) => {
    if (remaining === 0 || screenshotImages(message, toolNames) === 0) return message;
    const raw = message as ToolResultLike;
    const content = (raw.content as unknown[]).map((block) => {
      if (remaining === 0 || !isImage(block)) return block;
      remaining -= 1;
      return { type: "text", text: PRUNED_SCREENSHOT_TEXT };
    });
    return { ...raw, content } as T;
  });
}
