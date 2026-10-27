/** Live canvas calls are progress only: never fold unaccepted markup into the saved board. */
import type { NormalizedBlock, NormalizedMessage } from "./types";
import { CANVAS_TOOL_NAME } from "./canvas-state";
import { callArguments } from "./tool-utils";

export function pendingCanvasCalls(messages: readonly NormalizedMessage[], partial?: NormalizedMessage): NormalizedBlock[] {
  const turn: NormalizedMessage[] = [];
  for (let index = messages.length - 1; index >= 0 && messages[index].role !== "user"; index -= 1) turn.push(messages[index]);
  turn.reverse();
  const settled = new Set(turn.flatMap((message) => message.blocks.flatMap((block) =>
    block.type === "tool-result" && block.toolCallId ? [block.toolCallId] : [])));
  const calls = new Map<string, NormalizedBlock>();
  for (const message of partial ? [...turn, partial] : turn) {
    if (message.role !== "assistant" || message.stopReason === "aborted" || message.stopReason === "error") continue;
    for (const block of message.blocks) {
      if (block.type === "tool-call" && block.toolName === CANVAS_TOOL_NAME && block.toolCallId && !settled.has(block.toolCallId)) {
        calls.set(block.toolCallId, block);
      }
    }
  }
  return [...calls.values()];
}

export function canvasProgress(call: NormalizedBlock) {
  const args = callArguments(call);
  const entries = Array.isArray(args.artboards) ? args.artboards : [];
  const artboards = entries.flatMap((entry: unknown, index) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) return [];
    const item = entry as Record<string, unknown>;
    const name = typeof item.name === "string" ? item.name : typeof item.id === "string" ? item.id : `Artboard ${index + 1}`;
    const characters = [item.html, item.css].reduce<number>((total, value) => total + (typeof value === "string" ? value.length : 0), 0);
    return [{ id: typeof item.id === "string" ? item.id : undefined, name, characters }];
  });
  return {
    canvasId: typeof args.canvas === "string" ? args.canvas : undefined,
    title: typeof args.title === "string" ? args.title : undefined,
    artboards,
    characters: artboards.reduce((sum, item) => sum + item.characters, 0),
    removed: Array.isArray(args.remove) ? args.remove.length : 0,
    // Metadata changes count as fresh input too, even before the first HTML character.
    signature: JSON.stringify(args),
  };
}
