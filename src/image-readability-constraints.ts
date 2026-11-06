import { parseVisualArgs, parseVisualDetails, SHOW_VISUAL_TOOL_NAME } from "./tool-utils";
import type { NormalizedMessage } from "./types";

/**
 * Fixed ink that image readability must protect without recolouring it. Inspect the whole
 * selected history, not the transcript's mounted tail: revealing older turns must not switch
 * its palette. Warning text is the exposed Stopped label; other semantic status text lives on
 * solid chips/cards. Visual replies keep their own palette, including their intentional fills.
 */
export function conversationReadabilityConstraints(messages: readonly NormalizedMessage[], partial?: NormalizedMessage): { visual: boolean; warning: boolean } {
  const history = partial ? [...messages, partial] : messages;
  const visualCalls = new Map<string, boolean>();
  let warning = false;

  for (const message of history) {
    if (message.stopReason === "aborted") warning = true;
    for (const block of message.blocks) {
      if (block.type === "tool-call" && block.toolName === SHOW_VISUAL_TOOL_NAME && block.toolCallId) {
        visualCalls.set(block.toolCallId, parseVisualArgs(block) !== undefined);
      }
    }
  }

  const visual = history.some((message) => message.blocks.some((block) =>
    block.type === "tool-result" && !block.isError
    && (block.toolName === SHOW_VISUAL_TOOL_NAME || Boolean(block.toolCallId && visualCalls.has(block.toolCallId)))
    // Legacy visuals render from the call's HTML even without the small saved metadata.
    && (parseVisualDetails(block.details) !== undefined || Boolean(block.toolCallId && visualCalls.get(block.toolCallId)))
  ));
  return { visual, warning };
}
