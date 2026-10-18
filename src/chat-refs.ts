/**
 * `@` chat references: the message keeps `@"Chat name"` as plain text and one generated tail
 * section carries the ids. Nothing from the other chat rides the prompt — the agent calls
 * `read_chat` with an id when that history is relevant, so referencing a long chat costs a
 * sentence until it is read.
 *
 * The section follows `browser-context.ts`'s discipline: only a tail that parses through to the
 * very end of the message counts, the transcript strips it back out, and anything that fails
 * validation is left in the text untouched.
 *
 *     …the user's own words…
 *
 *     <referenced-chats-v1>
 *     Chats the user referenced with @. Call read_chat with a chat id when its history is
 *     relevant; nothing from it is included here.
 *     {"version":1,"chats":[{"id":"<task id>","name":"Auth refactor"}]}
 *     </referenced-chats-v1>
 */
import type { ChatRefRecord, ProjectRecord, TaskKind, TaskRecord } from "./types";
import { isSubsequence, rankMentions, type ChatMentionSuggestion, type MentionSuggestion } from "./mention-utils";

const OPEN = "<referenced-chats-v1>";
const CLOSE = "</referenced-chats-v1>";

/** One message can reference this many chats; the picker only ever inserts one per gesture. */
export const MAX_CHAT_REFS = 16;
/** A chat name is a label, not content: anything longer is not a name this app saved. */
const MAX_NAME_CHARS = 200;

/**
 * The fixed sentence above the JSON. It is the only guidance the model gets about the section
 * (no system prompt or guide changes), so keep it short and self-contained.
 */
export const CHAT_REFS_GUIDANCE = "Chats the user referenced with @. Call read_chat with a chat id when its history is relevant; nothing from it is included here.";

/** The token the draft and the transcript both use: always quoted, so spaces keep it one token. */
export function chatMentionToken(name: string): string {
  return `@"${name.replace(/["\n]/g, "")}"`;
}

/**
 * Append the referenced-chats section. Refs are de-duplicated by id and capped; none of them
 * being usable leaves the text exactly as it was.
 */
export function encodeChatRefs(text: string, refs: ChatRefRecord[]): string {
  const seen = new Set<string>();
  const chats: ChatRefRecord[] = [];
  for (const ref of refs) {
    if (!ref?.id || !ref.name || seen.has(ref.id) || chats.length >= MAX_CHAT_REFS) continue;
    seen.add(ref.id);
    chats.push({ id: ref.id, name: ref.name });
  }
  if (chats.length === 0) return text;
  const section = `${OPEN}\n${CHAT_REFS_GUIDANCE}\n${JSON.stringify({ version: 1, chats })}\n${CLOSE}`;
  return text ? `${text}\n\n${section}` : section;
}

function validRefs(value: unknown): ChatRefRecord[] | undefined {
  if (!value || typeof value !== "object") return undefined;
  const data = value as { version?: unknown; chats?: unknown };
  if (data.version !== 1 || !Array.isArray(data.chats) || data.chats.length === 0 || data.chats.length > MAX_CHAT_REFS) return undefined;
  const chats: ChatRefRecord[] = [];
  for (const item of data.chats) {
    if (!item || typeof item !== "object") return undefined;
    const { id, name } = item as { id?: unknown; name?: unknown };
    if (typeof id !== "string" || !id || typeof name !== "string" || !name || name.length > MAX_NAME_CHARS) return undefined;
    if (chats.some((ref) => ref.id === id)) return undefined;
    chats.push({ id, name });
  }
  return chats;
}

/** The section's payload: the last line that parses as a versioned record — the guidance is prose. */
function parseSection(body: string): ChatRefRecord[] | undefined {
  const lines = body.split("\n");
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const line = lines[index].trim();
    if (!line.startsWith("{")) continue;
    try {
      const parsed = validRefs(JSON.parse(line));
      if (parsed) return parsed;
    } catch { /* Not the payload; keep looking. */ }
  }
  return undefined;
}

/**
 * The message's own words and the chats it referenced — the inverse of `encodeChatRefs`.
 * A lookalike or malformed section is plain text, never a reference.
 */
export function splitChatRefSection(message: string): { text: string; chatRefs: ChatRefRecord[] } {
  const tail = `\n${CLOSE}`;
  if (!message.endsWith(tail)) return { text: message, chatRefs: [] };
  const at = message.startsWith(`${OPEN}\n`) ? 0 : message.lastIndexOf(`\n\n${OPEN}\n`);
  if (at < 0) return { text: message, chatRefs: [] };
  const start = at === 0 ? OPEN.length + 1 : at + 2 + OPEN.length + 1;
  const chatRefs = parseSection(message.slice(start, message.length - tail.length));
  if (!chatRefs) return { text: message, chatRefs: [] };
  return { text: at === 0 ? "" : message.slice(0, at), chatRefs };
}

/**
 * The refs a message still deserves: deleting the `@"Name"` token from the draft (or the
 * editor) drops that chat from the generated section.
 */
export function survivingChatRefs(text: string, refs: ChatRefRecord[]): ChatRefRecord[] {
  return refs.filter((ref) => text.includes(chatMentionToken(ref.name)));
}

/** Chat mode chats may only read other Chat mode chats; Code chats may read either kind. */
export function chatRefAllowed(requesterKind: TaskKind, targetKind: TaskKind): boolean {
  return requesterKind === "code" || targetKind === "chat";
}

function activity(task: TaskRecord): number {
  const stamp = task.lastActivityAt ?? task.updatedAt;
  const at = Date.parse(stamp);
  return Number.isFinite(at) ? at : 0;
}

/**
 * Every chat the requester may reference, newest activity first. The current chat is never one
 * of them, and the kind rule is applied here so the picker cannot offer a chat the host refuses
 * (the host still enforces it; this is the matching UX, not the guard).
 */
export function chatRefCandidates(tasks: TaskRecord[], projects: ProjectRecord[], requesterKind: TaskKind, excludeId?: string): ChatMentionSuggestion[] {
  const names = new Map(projects.map((project) => [project.id, project.name] as const));
  return tasks
    .filter((task) => task.id !== excludeId && chatRefAllowed(requesterKind, task.kind))
    .map((task) => ({
      kind: "chat" as const,
      id: task.id,
      name: task.name,
      label: [task.projectId ? names.get(task.projectId) ?? "No project" : "No project", ...(task.archived ? ["Archived"] : [])].join(" · "),
      archived: task.archived,
      at: activity(task),
    }))
    .sort((a, b) => (b.at ?? 0) - (a.at ?? 0) || a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

/** How well a chat name matches what was typed after `@`; 0 means not at all. */
function chatScore(name: string, query: string): number {
  const target = name.toLowerCase();
  if (target === query) return 100;
  if (target.startsWith(query)) return 80;
  if (target.includes(query)) return 60;
  // Letters scattered across a whole name match almost anything, as in file ranking; keep it weak.
  return isSubsequence(query, target) ? 10 : 0;
}

/** The best matching chats for an `@` query, newest activity breaking ties. */
export function rankChats(chats: ChatMentionSuggestion[], query: string, limit = 5): ChatMentionSuggestion[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  return chats
    .map((chat) => ({ chat, score: chatScore(chat.name, needle) }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || (b.chat.at ?? 0) - (a.chat.at ?? 0) || a.chat.name.localeCompare(b.chat.name))
    .slice(0, limit)
    .map((entry) => entry.chat);
}

/**
 * What the `@` picker lists: workspace files first, ranked exactly as before, then the matching
 * chats under their own heading. Files are undefined until the list loads; chats work regardless.
 */
export function rankSuggestions(files: string[] | undefined, chats: ChatMentionSuggestion[] | undefined, query: string): MentionSuggestion[] {
  const entries: MentionSuggestion[] = files ? rankMentions(files, query) : [];
  return [...entries, ...rankChats(chats ?? [], query)];
}
