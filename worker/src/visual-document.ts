/** Saved visual documents stay in the session; only the requested current-branch call leaves it. */
export function visualDocument(messages: readonly unknown[], toolCallId: string): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index] as Record<string, unknown> | undefined;
    if (message?.role !== "toolResult" || message.toolName !== "show_visual" || message.toolCallId !== toolCallId) continue;
    const details = message.details as Record<string, unknown> | undefined;
    if (!message.isError && details?.v === 1 && details.inlined === true && typeof details.html === "string") return details.html;
    break;
  }
  throw new Error("That visual is no longer in this chat.");
}
