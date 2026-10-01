/** Shared by live workers and the read-only saved-session reader. No model or extensions run here. */
import type { NormalizedBlock, NormalizedMessage } from "./protocol.js";
import type { ThinkingDurations } from "./run-timing.js";
import { withoutTranscripts } from "./builtin/subagents/details.js";
import { SUBAGENT_TOOL_NAME } from "./builtin/subagents/types.js";
import { BROWSER_SCREENSHOT_TOOL_NAME } from "./builtin/browser.js";
import { COMPUTER_SCREENSHOT_TOOL_NAME } from "./builtin/computer-use/params.js";

export const THUMBNAIL_OPTIONS = { maxWidth: 512, maxHeight: 512, maxBytes: 128 * 1024 };
export const RESULT_THUMBNAIL_OPTIONS = { maxWidth: 480, maxHeight: 480, maxBytes: 64 * 1024 };
export const THUMBNAIL_RESULT_TOOLS: ReadonlySet<string> = new Set([COMPUTER_SCREENSHOT_TOOL_NAME, BROWSER_SCREENSHOT_TOOL_NAME]);
export type ImageNormalizer = (block: Record<string, unknown>, options?: typeof THUMBNAIL_OPTIONS) => NormalizedBlock;

export function textFromContent(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((item) => {
      if (typeof item === "string") return item;
      if (!item || typeof item !== "object") return "";
      const block = item as Record<string, unknown>;
      return typeof block.text === "string" ? block.text : typeof block.content === "string" ? block.content : "";
    })
    .join("");
}

function normalizeBlocks(content: unknown, role: string, thinking: ThinkingDurations | undefined, imageBlock: ImageNormalizer, starts?: Array<number | null>): NormalizedBlock[] {
  if (typeof content === "string") {
    return [{ type: role === "toolResult" ? "tool-result" : "text", text: content }];
  }
  if (!Array.isArray(content)) return [];

  let thought = 0;
  return content.flatMap((item): NormalizedBlock[] => {
    if (typeof item === "string") return [{ type: "text", text: item }];
    if (!item || typeof item !== "object") return [];
    const block = item as Record<string, unknown>;
    if (block.type === "text") return [{ type: "text", text: String(block.text ?? "") }];
    // A tool result's images (e.g. `read` on a PNG) still reach the model; the transcript shows
    // only its text. Left in, each would become an empty result sharing the call's id and
    // overwrite the real one. Screenshot tools' images ride their result block instead
    // (`normalizeMessage`).
    if (block.type === "image") return role === "toolResult" ? [] : [imageBlock(block)];
    if (block.type === "thinking") {
      const durationMs = thinking?.[thought];
      const startedAt = starts?.[thought];
      thought += 1;
      // A closed duration wins over any stale live start; completed blocks never keep a timer.
      return [{
        type: "thinking",
        text: String(block.thinking ?? block.text ?? ""),
        ...(typeof durationMs === "number" ? { durationMs } : typeof startedAt === "number" ? { startedAt } : {})
      }];
    }
    if (block.type === "toolCall") {
      return [{
        type: "tool-call",
        toolName: String(block.name ?? "tool"),
        toolCallId: String(block.id ?? ""),
        arguments: block.arguments
      }];
    }
    return [{ type: role === "toolResult" ? "tool-result" : "text", text: textFromContent([block]) }];
  });
}

export function normalizeMessage(message: unknown, index: number, thinking: ThinkingDurations | undefined, imageBlock: ImageNormalizer, starts?: Array<number | null>): NormalizedMessage | undefined {
  if (!message || typeof message !== "object") return undefined;
  const raw = message as Record<string, unknown>;
  const rawRole = String(raw.role ?? "system");
  const role = rawRole === "toolResult" ? "tool" : rawRole;
  if (role !== "user" && role !== "assistant" && role !== "tool" && role !== "system") return undefined;
  const blocks = normalizeBlocks(raw.content, rawRole, thinking, imageBlock, starts);
  if (rawRole === "toolResult") {
    // An image-only result still has to mark its call as finished.
    if (blocks.length === 0) blocks.push({ type: "tool-result", text: "" });
    // The transcript keeps the last block per call id, so a screenshot's previews go on that one.
    if (typeof raw.toolName === "string" && THUMBNAIL_RESULT_TOOLS.has(raw.toolName) && Array.isArray(raw.content)) {
      const images = (raw.content as unknown[])
        .filter((item): item is Record<string, unknown> => typeof item === "object" && item !== null && (item as { type?: unknown }).type === "image")
        .map((item) => imageBlock(item, RESULT_THUMBNAIL_OPTIONS))
        .map(({ imageId, thumbnail }) => ({ imageId: imageId as string, ...(thumbnail ? { thumbnail } : {}) }));
      if (images.length) blocks[blocks.length - 1].images = images;
    }
    for (const block of blocks) {
      block.type = "tool-result";
      block.toolName = typeof raw.toolName === "string" ? raw.toolName : undefined;
      block.toolCallId = typeof raw.toolCallId === "string" ? raw.toolCallId : undefined;
      block.isError = raw.isError === true;
      // A sub-agent call's saved transcripts are the side panel's alone (`watch_subagent`).
      if (raw.details !== undefined) block.details = raw.toolName === SUBAGENT_TOOL_NAME ? withoutTranscripts(raw.details) : raw.details;
    }
  }
  const timestamp = typeof raw.timestamp === "number" ? raw.timestamp : undefined;
  // A Stop during a tool call still lets Pi start the next model request, which fails at once on
  // the aborted signal ("This operation was aborted"). That is the stop, not a failure: the
  // transcript shows the quiet "Stopped" label, live and when a saved chat is restored.
  const abortStop = raw.stopReason === "error" && /operation was aborted/i.test(String(raw.errorMessage ?? ""));
  const normalized: NormalizedMessage = {
    id: `${role}-${timestamp ?? "na"}-${index}`,
    role,
    timestamp,
    blocks,
    stopReason: abortStop ? "aborted" : typeof raw.stopReason === "string" ? raw.stopReason : undefined,
    errorMessage: abortStop ? undefined : typeof raw.errorMessage === "string" ? raw.errorMessage : undefined
  };
  return normalized;
}
