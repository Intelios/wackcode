/**
 * Suggested tasks: the follow-ups the agent filed with `suggest_task` (worker's
 * `builtin/suggest-task.ts`), the card each one draws under the turn's final answer, and the
 * message a Start action sends.
 *
 * The tool call's arguments are the card: title, kind and a self-contained brief. Pi saves them
 * with the call, so a saved chat and a cold reopen need nothing extra — the same discipline as a
 * visual reply. A settled, successful call is a card; a refused or unfinished one stays an
 * ordinary tool row inside the turn's work.
 *
 * A Start action sends the suggestion as a user message with one generated tail section, like
 * the other WackCode additions (attached files, a visual's click, referenced chats):
 *
 *     Dismiss button stays disabled after a failed save
 *
 *     **Where:** `src/components/Composer.tsx` … (the agent's own brief)
 *
 *     <suggested-task-v1>
 *     Started from a suggested task "Dismiss button stays disabled after a failed save" in the
 *     chat "Settings cleanup". Call read_chat with its id when you need the context it was found in.
 *     {"version":1,"id":"call-abc","kind":"bug","title":"…","origin":{"chatId":"t1","chatName":"…"}}
 *     </suggested-task-v1>
 *
 * The id is the tool-call id, which is the suggestion's identity everywhere: the origin chat's
 * card finds the new chat through `TaskRecord.originSuggestion`, and "started here" is read back
 * out of this chat's own later messages, so a rewind re-opens the card. Only a tail that parses
 * through to the end of the message counts, and anything that fails validation stays in the text
 * untouched — the transcript then shows the words as typed.
 */
import type { NormalizedBlock, NormalizedMessage, SuggestionKind, SuggestionOrigin } from "./types";

/** The built-in tool that files a suggestion; its calls render as cards, not rows. */
export const SUGGEST_TASK_TOOL_NAME = "suggest_task";
export const SUGGESTED_TASK_OPEN = "<suggested-task-v1>";
export const SUGGESTED_TASK_CLOSE = "</suggested-task-v1>";
/** One message's brief is bounded by the tool itself; anything longer is not a suggestion. */
const MAX_BRIEF_CHARS = 4_000;
const MAX_NAME_CHARS = 200;

export const SUGGESTION_KINDS: readonly SuggestionKind[] = ["bug", "polish", "improvement"];

/** The words each kind wears on its card. */
export const SUGGESTION_KIND_LABELS: Record<SuggestionKind, string> = {
  bug: "Bug",
  polish: "Polish",
  improvement: "Improvement",
};

export function isSuggestionKind(value: unknown): value is SuggestionKind {
  return typeof value === "string" && (SUGGESTION_KINDS as readonly string[]).includes(value);
}

/** A suggestion as the tray draws it. */
export interface SuggestionCard {
  /** The `suggest_task` tool-call id: the suggestion's identity everywhere. */
  id: string;
  title: string;
  kind: SuggestionKind;
  /** The agent's Markdown brief, meant to be readable on its own. */
  description: string;
}

/** A card's versioned result details; the brief itself stays in the call's arguments. */
export interface SuggestionDetails {
  v: 1;
  kind: SuggestionKind;
  title: string;
}

export function parseSuggestionDetails(value: unknown): SuggestionDetails | undefined {
  if (!value || typeof value !== "object") return undefined;
  const details = value as Record<string, unknown>;
  if (details.v !== 1 || typeof details.title !== "string" || !details.title.trim() || !isSuggestionKind(details.kind)) return undefined;
  return { v: 1, kind: details.kind, title: details.title };
}

function callArgs(block: NormalizedBlock): Record<string, unknown> {
  const value = block.arguments;
  if (value && typeof value === "object" && !Array.isArray(value)) return value as Record<string, unknown>;
  if (typeof value === "string") {
    try {
      const parsed = JSON.parse(value) as unknown;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed as Record<string, unknown>;
    } catch { /* still streaming, or not ours */ }
  }
  return {};
}

/**
 * One `suggest_task` call as a card: its arguments plus the settled result that says the call
 * was accepted. Undefined while it runs, when it was refused, or when its arguments do not
 * parse — the ordinary tool row already says what happened.
 */
export function suggestionFromCall(call: NormalizedBlock, result?: NormalizedBlock): SuggestionCard | undefined {
  if (call.type !== "tool-call" || call.toolName !== SUGGEST_TASK_TOOL_NAME || !call.toolCallId) return undefined;
  if (!result || result.isError) return undefined;
  const args = callArgs(call);
  const title = typeof args.title === "string" ? args.title.replace(/\s+/g, " ").trim() : "";
  const description = typeof args.description === "string" ? args.description.trim() : "";
  if (!title || !description || description.length > MAX_BRIEF_CHARS || !isSuggestionKind(args.kind)) return undefined;
  return { id: call.toolCallId, title, kind: args.kind, description };
}

/**
 * Every accepted suggestion in one turn's slice of messages, in the order it was filed. A call
 * with no result yet (a stopped run) is not a card: nothing confirmed it was recorded.
 */
export function turnSuggestions(
  messages: NormalizedMessage[],
  results: Map<string, NormalizedBlock>,
  from: number,
  to: number
): SuggestionCard[] {
  const cards: SuggestionCard[] = [];
  for (let index = from; index < to; index += 1) {
    for (const block of messages[index].blocks) {
      const card = suggestionFromCall(block, block.toolCallId ? results.get(block.toolCallId) : undefined);
      if (card) cards.push(card);
    }
  }
  return cards;
}

/** The origin of a started suggestion, as the tail carries it. */
export interface StartedSuggestion {
  id: string;
  kind: SuggestionKind;
  title: string;
  origin?: { chatId: string; chatName: string };
}

/** The title as the tail's guidance sentence spells it: one line, no quotes that end it. */
function cleanTitle(title: string): string {
  return title.replace(/[\s"]+/g, " ").trim().slice(0, MAX_NAME_CHARS);
}

/**
 * The message a Start action sends. `origin` is present only for a new chat: it names the chat
 * the suggestion was found in so the new agent can call `read_chat` for the context.
 */
export function composeSuggestedTask(
  suggestion: { id: string; kind: SuggestionKind; title: string; description: string },
  origin?: { chatId: string; chatName: string }
): string {
  const title = cleanTitle(suggestion.title);
  const brief = suggestion.description.trim();
  const words = [title, brief].filter(Boolean).join("\n\n");
  const sentence = origin
    ? `Started from a suggested task "${title}" in the chat "${cleanTitle(origin.chatName)}". Call read_chat with its id when you need the context it was found in.`
    : `Started from a suggested task "${title}".`;
  const payload = JSON.stringify({
    version: 1,
    id: suggestion.id,
    kind: suggestion.kind,
    title,
    ...(origin ? { origin: { chatId: origin.chatId, chatName: origin.chatName } } : {}),
  });
  return `${words}\n\n${SUGGESTED_TASK_OPEN}\n${sentence}\n${payload}\n${SUGGESTED_TASK_CLOSE}`;
}

/** The section's payload: the last line that parses as a versioned record — the guidance is prose. */
function parseSection(body: string): StartedSuggestion | undefined {
  const lines = body.split("\n");
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const line = lines[index].trim();
    if (!line.startsWith("{")) continue;
    let parsed: unknown;
    try { parsed = JSON.parse(line); } catch { return undefined; }
    if (!parsed || typeof parsed !== "object") return undefined;
    const record = parsed as Record<string, unknown>;
    if (record.version !== 1 || typeof record.id !== "string" || !record.id || record.id.length > 128) return undefined;
    if (typeof record.title !== "string" || !record.title.trim() || !isSuggestionKind(record.kind)) return undefined;
    const origin = record.origin;
    if (origin === undefined) return { id: record.id, kind: record.kind, title: record.title, };
    if (!origin || typeof origin !== "object") return undefined;
    const { chatId, chatName } = origin as Record<string, unknown>;
    if (typeof chatId !== "string" || !chatId || typeof chatName !== "string" || !chatName || chatName.length > MAX_NAME_CHARS) return undefined;
    return { id: record.id, kind: record.kind, title: record.title, origin: { chatId, chatName } };
  }
  return undefined;
}

/**
 * A message's own words and the suggestion it started, or the text unchanged when it carries no
 * tail. A tail anywhere at the end counts, so a message edited down to its section still splits.
 */
export function splitSuggestedTask(text: string): { text: string; suggestion?: StartedSuggestion } {
  const close = text.lastIndexOf(SUGGESTED_TASK_CLOSE);
  // Anything after the closing tag means this was not a generated tail at all.
  if (close < 0 || text.slice(close + SUGGESTED_TASK_CLOSE.length).trim()) return { text };
  const open = text.lastIndexOf(SUGGESTED_TASK_OPEN, close);
  if (open < 0) return { text };
  const inner = text.slice(open + SUGGESTED_TASK_OPEN.length, close);
  const suggestion = parseSection(inner);
  if (!suggestion) return { text };
  const body = text.slice(0, open).trimEnd();
  if (!body) return { text };
  return { text: body, suggestion };
}

/**
 * The suggestions started in this chat, by their own later messages. Read from the transcript
 * rather than stored: a rewind past the message re-opens the card, exactly as it removes the
 * work that suggestion's chat did.
 */
export function startedHere(messages: NormalizedMessage[], partial?: NormalizedMessage): Map<string, StartedSuggestion> {
  const started = new Map<string, StartedSuggestion>();
  for (const message of partial ? [...messages, partial] : messages) {
    if (message.role !== "user") continue;
    const text = message.blocks.map((block) => block.text ?? "").join("\n");
    if (!text.includes(SUGGESTED_TASK_CLOSE)) continue;
    const { suggestion } = splitSuggestedTask(text);
    if (suggestion) started.set(suggestion.id, suggestion);
  }
  return started;
}

/** Where a started suggestion's chat lives, for the origin chat's card. */
export interface SuggestionLink {
  taskId: string;
  name: string;
}

/**
 * Every chat started from a suggested task, keyed by `origin chat id → suggestion id`. The
 * origin card reads this to say "Opened in …"; a chat the user deleted simply isn't here, which
 * re-opens the card.
 */
export function suggestionLinks(tasks: Array<{ id: string; name: string; originSuggestion: SuggestionOrigin | null }>): Map<string, Map<string, SuggestionLink>> {
  const links = new Map<string, Map<string, SuggestionLink>>();
  for (const task of tasks) {
    const origin = task.originSuggestion;
    if (!origin) continue;
    const bySuggestion = links.get(origin.taskId) ?? new Map<string, SuggestionLink>();
    bySuggestion.set(origin.suggestionId, { taskId: task.id, name: task.name });
    links.set(origin.taskId, bySuggestion);
  }
  return links;
}
