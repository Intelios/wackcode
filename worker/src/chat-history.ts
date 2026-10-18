/**
 * Projects another chat's saved Pi session into the plain text `read_chat` returns. This is
 * read-only: it parses the file in memory (Pi's own in-memory migrations included), follows the
 * branch the chat last had on disk, and never creates a session, loads an extension, resolves a
 * model or writes the file.
 *
 * The projection is deliberately small: turns as `## User` / `## Assistant` sections, thinking
 * left out, images as `[image]`, tool calls as one-line markers. Tool results are included only
 * on request, and truncated, because a coding chat's results are where the megabytes are.
 * WackCode's own generated tail sections inside a user message (attached files, browser context,
 * referenced chats, a message a visual sent) collapse to one-line markers so a nested dump cannot
 * swallow the answer.
 */
import type { FileEntry, SessionEntry } from "@earendil-works/pi-coding-agent";
import { transcriptEntries } from "./transcript.js";
import { buildTreeIndex, type EntryLike } from "./tree.js";
import { MAX_VISUAL_TITLE, SHOW_VISUAL_TOOL_NAME } from "./builtin/visual.js";
import { MAX_SUGGESTION_TITLE, SUGGEST_TASK_TOOL_NAME } from "./builtin/suggest-task.js";

/** What the host knows about the chat, beyond its session file. */
export interface ChatHistoryMeta {
  name: string;
  project?: string;
  workspace?: string;
  /** "Provider · Model" as the chat is configured now. */
  model?: string;
  archived?: boolean;
}

export interface ChatHistoryOptions {
  /** Include tool results (truncated), not just the calls. */
  tools?: boolean;
}

/** One tool result's text, before any overall paging. */
const MAX_RESULT_CHARS = 2_000;
/** A tool call's arguments, as one line next to its name. */
const MAX_ARGS_CHARS = 120;

const TAIL_SECTIONS: readonly { open: string; close: string; marker: (inner: string) => string }[] = [
  { open: "<attached-files>", close: "</attached-files>", marker: () => "[attached files]" },
  { open: "<browser-contexts-v1>", close: "</browser-contexts-v1>", marker: () => "[browser context]" },
  { open: "<referenced-chats-v1>", close: "</referenced-chats-v1>", marker: () => "[referenced chats]" },
  {
    open: "<visual-action-v1>",
    close: "</visual-action-v1>",
    // The title is the one detail worth keeping: it says which visual the user pressed.
    marker: (inner) => {
      const title = /visual\s+"([^"]{1,60})"/.exec(inner)?.[1];
      return title ? `[sent from visual: ${title}]` : "[sent from visual]";
    },
  },
  {
    open: "<suggested-task-v1>",
    close: "</suggested-task-v1>",
    // Which suggestion this turn started, and the chat it came from when it is a new chat.
    marker: (inner) => {
      const title = /task\s+"([^"]{1,80})"/.exec(inner)?.[1];
      return title ? `[started suggested task: ${title}]` : "[started a suggested task]";
    },
  },
] as const;

/** Where a generated section starts, counting the blank line before it (or the message's start). */
function sectionStart(text: string, open: string): number {
  if (text.startsWith(`${open}\n`)) return 0;
  return text.lastIndexOf(`\n\n${open}\n`);
}

/** Replace each generated tail section with a short marker, from the outermost in. */
export function stripTailSections(text: string): string {
  let body = text;
  const markers: string[] = [];
  for (let pass = 0; pass < TAIL_SECTIONS.length * 2; pass += 1) {
    const hit = TAIL_SECTIONS.find((section) => body.endsWith(`\n${section.close}`) && sectionStart(body, section.open) >= 0);
    if (!hit) break;
    const start = sectionStart(body, hit.open);
    const inner = body.slice(start + hit.open.length, body.length - hit.close.length - 1);
    body = body.slice(0, start);
    markers.unshift(hit.marker(inner));
  }
  return [body, ...markers].filter(Boolean).join("\n\n");
}

function clip(text: string, limit: number): string {
  const compact = text.replace(/\s+/g, " ").trim();
  return compact.length > limit ? `${compact.slice(0, limit - 1)}…` : compact;
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.flatMap((block) => {
    if (!block || typeof block !== "object") return [];
    const value = block as { type?: unknown; text?: unknown };
    if (value.type === "text" && typeof value.text === "string") return [value.text];
    if (value.type === "image") return ["[image]"];
    return [];
  }).join("\n");
}

function toolCalls(content: unknown): string[] {
  if (!Array.isArray(content)) return [];
  return content.flatMap((block) => {
    if (!block || typeof block !== "object") return [];
    const value = block as { type?: unknown; name?: unknown; arguments?: unknown };
    if (value.type !== "toolCall" || typeof value.name !== "string") return [];
    // A visual's HTML is its whole point; the marker names it and leaves the markup out.
    if (value.name === SHOW_VISUAL_TOOL_NAME) {
      const title = (value.arguments as { title?: unknown } | undefined)?.title;
      return [typeof title === "string" && title.trim() ? `[visual: ${clip(title, MAX_VISUAL_TITLE)}]` : "[visual]"];
    }
    // A suggestion's brief is written for the user's card, not for another chat's context.
    if (value.name === SUGGEST_TASK_TOOL_NAME) {
      const title = (value.arguments as { title?: unknown } | undefined)?.title;
      return [typeof title === "string" && title.trim() ? `[suggested task: ${clip(title, MAX_SUGGESTION_TITLE)}]` : "[suggested task]"];
    }
    let args = "";
    try { args = JSON.stringify(value.arguments ?? {}); } catch { args = ""; }
    return [args && args !== "{}" ? `[tool: ${value.name}(${clip(args, MAX_ARGS_CHARS)})]` : `[tool: ${value.name}]`];
  });
}

/**
 * The chat's current branch, root first: the same walk a saved history uses (the last entry is
 * the leaf; every entry names its parent), so a rewind shows the branch the chat ended on.
 */
export function chatBranch(fileEntries: FileEntry[]): SessionEntry[] {
  const entries = fileEntries.filter((entry): entry is SessionEntry => entry.type !== "session" && typeof (entry as { id?: unknown }).id === "string");
  const index = buildTreeIndex(entries as EntryLike[]);
  const path: SessionEntry[] = [];
  const visited = new Set<string>();
  let current = entries.at(-1);
  while (current) {
    if (visited.has(current.id)) throw new Error("That chat's saved history has a circular branch.");
    visited.add(current.id);
    path.push(current);
    current = current.parentId ? index.byId.get(current.parentId) as SessionEntry | undefined : undefined;
  }
  return path.reverse();
}

/** The whole chat as text; paging slices this, so a repeated call renders the same bytes. */
export function renderChatHistory(fileEntries: FileEntry[], meta: ChatHistoryMeta, options: ChatHistoryOptions = {}): string {
  const path = chatBranch(fileEntries);
  const turns = transcriptEntries(path as EntryLike[]);
  let users = 0;
  let assistants = 0;
  for (const turn of turns) {
    const role = (turn.raw as { role?: unknown }).role;
    if (role === "user") users += 1;
    else if (role === "assistant") assistants += 1;
  }

  const header = [`# Chat: ${meta.name}`];
  if (meta.project) header.push(`Project: ${meta.project}`);
  if (meta.workspace) header.push(`Workspace: ${meta.workspace}`);
  if (meta.model) header.push(`Model: ${meta.model}`);
  if (meta.archived) header.push("Archived: yes");
  header.push(`Messages: ${users + assistants} (${users} user, ${assistants} assistant)`);
  header.push("Branch: this chat's current conversation, as saved to disk. Nothing else from it is included.");

  const blocks: string[] = [header.join("\n")];
  for (const turn of turns) {
    if (turn.compaction) {
      const summary = (turn.compaction as { summary?: unknown }).summary;
      blocks.push(`## Compaction summary\n\n${typeof summary === "string" ? summary.trim() : "[summary unavailable]"}`);
      continue;
    }
    const raw = turn.raw as { role?: unknown; content?: unknown; toolName?: unknown };
    if (raw.role === "user") {
      const text = stripTailSections(textOf(raw.content)).trim();
      blocks.push(`## User\n\n${text || "[empty message]"}`);
    } else if (raw.role === "assistant") {
      const prose = textOf(raw.content).trim();
      const calls = toolCalls(raw.content);
      const body = [prose, ...calls].filter(Boolean).join("\n\n");
      blocks.push(`## Assistant\n\n${body || "[no text]"}`);
    } else if (raw.role === "toolResult" && options.tools) {
      const content = stripTailSections(textOf(raw.content)).trim();
      const clipped = content.length > MAX_RESULT_CHARS ? `${content.slice(0, MAX_RESULT_CHARS)}\n[… result truncated]` : content;
      blocks.push(`[tool result: ${typeof raw.toolName === "string" ? raw.toolName : "unknown"}]\n${clipped || "[no output]"}`);
    }
  }
  if (turns.length === 0) blocks.push("[This chat has no saved messages.]");
  return blocks.join("\n\n");
}
