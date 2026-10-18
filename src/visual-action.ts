/**
 * A message the user sent by pressing something inside a visual reply (`wackcode.send`).
 *
 * The text travels with a generated tail section, like the other WackCode additions to a
 * message (attached files, browser context, referenced chats), so the model learns the click
 * came from its own widget instead of the user typing. The UI strips the section and shows a
 * caption under the message instead; copy and edit work on the words alone, and an edited
 * resend drops the tail.
 *
 * `worker/src/chat-history.ts` collapses the section for `read_chat`, so one chat reading
 * another never sees the transport text.
 */
export const VISUAL_ACTION_OPEN = "<visual-action-v1>";
export const VISUAL_ACTION_CLOSE = "</visual-action-v1>";
/** A widget's message is a sentence or two; anything longer is not a click. */
export const MAX_VISUAL_SEND_CHARS = 2_000;

/** Why a visual's message did not go out. The frame receives these words as `reason`. */
export type VisualSendRefusal = "stale" | "invalid" | "throttled" | "busy" | "failed";
export type VisualSendResult = { ok: true } | { ok: false; reason: VisualSendRefusal };

/** The title as the tail carries it: one line, no quotes that would end the section's phrase. */
function cleanTitle(title: string): string {
  return title.replace(/["\r\n]+/g, " ").trim().slice(0, 60);
}

export function composeVisualAction(text: string, title: string): string {
  const words = text.trim().slice(0, MAX_VISUAL_SEND_CHARS);
  const name = cleanTitle(title);
  const line = name ? `Sent from the visual "${name}".` : "Sent from a visual.";
  return `${words}\n\n${VISUAL_ACTION_OPEN}\n${line}\n${VISUAL_ACTION_CLOSE}`;
}

/**
 * The words and the visual's title, or the text unchanged when it carries no tail. Tolerates the
 * section anywhere at the end, so a message edited down to its tail still splits cleanly.
 */
export function splitVisualAction(text: string): { text: string; title?: string } {
  const close = text.lastIndexOf(VISUAL_ACTION_CLOSE);
  // Anything after the closing tag means this was not a generated tail at all.
  if (close < 0 || text.slice(close + VISUAL_ACTION_CLOSE.length).trim()) return { text };
  const open = text.lastIndexOf(VISUAL_ACTION_OPEN, close);
  if (open < 0) return { text };
  const inner = text.slice(open + VISUAL_ACTION_OPEN.length, close);
  const title = /visual\s+"([^"]{1,60})"/.exec(inner)?.[1];
  const body = text.slice(0, open).trimEnd();
  return { text: body, ...(title ? { title } : {}) };
}
