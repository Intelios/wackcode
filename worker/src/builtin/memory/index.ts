/**
 * Built-in memory tools: the per-project notes described in `store.ts`. The host names the
 * project's memory directory (under the app's own data, never the workspace) and switches the
 * feature on or off; everything else lives in ordinary files the user can audit and edit in
 * Settings › Memory.
 *
 * Three tools, one job each: `memory_save` writes (or updates) a note, `memory_recall` reads
 * notes in full, `memory_forget` retires one. The system prompt gets only the generated index
 * (`store.ts::memoryIndex`), refreshed before each run — a save never rewrites the prompt
 * mid-run, so the prompt cache survives and the run sees a stable memory list.
 *
 * Like sub-agents these tools are gated by their own setting (never the tool denylist) and stay
 * registered while off, so switching memory on or off never respawns a chat. Sub-agents never
 * see them: they are inline tools the child loader doesn't include, and the index is absent
 * from their separate prompt.
 */
import { mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import type { MemoryPayload } from "../../protocol.js";
import {
  MAX_BODY_CHARS, MAX_DESCRIPTION_CHARS, MAX_TITLE_CHARS, MEMORY_TYPES,
  composeMemory, isMemoryName, isMemoryType, memoryIndex, memoryName, parseMemory,
  type MemoryNote, type MemoryType
} from "./store.js";

export const MEMORY_SAVE_TOOL_NAME = "memory_save";
export const MEMORY_RECALL_TOOL_NAME = "memory_recall";
export const MEMORY_FORGET_TOOL_NAME = "memory_forget";
export const MEMORY_TOOL_NAMES = [MEMORY_SAVE_TOOL_NAME, MEMORY_RECALL_TOOL_NAME, MEMORY_FORGET_TOOL_NAME] as const;

/** Where `memory_forget` moves notes: invisible to the index and to Settings, but reversible. */
const TRASH_DIR = ".trash";
const MAX_RECALL_NAMES = 8;

const SAVE_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["type", "title", "description", "body"],
  properties: {
    type: { type: "string", enum: [...MEMORY_TYPES], description: "user, feedback, project or reference." },
    title: { type: "string", description: "Short name for the note, shown in the memory index every session." },
    description: { type: "string", description: "One line saying when the note matters, shown next to the title in the index." },
    body: { type: "string", description: "The note itself. Keep it as small as it can be and still be useful." },
    name: { type: "string", description: "The existing note to update, as the index spells it. Omit to create a new one." }
  }
} as const;

const RECALL_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["names"],
  properties: {
    names: { type: "array", items: { type: "string" }, minItems: 1, maxItems: MAX_RECALL_NAMES, description: "Note names from the memory index." }
  }
} as const;

const FORGET_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["name"],
  properties: {
    name: { type: "string", description: "The note to retire, as the index spells it." }
  }
} as const;

export interface MemorySaveDetails {
  kind: "save";
  name: string;
  title: string;
  type: MemoryType;
}
export interface MemoryRecallDetails {
  kind: "recall";
  /** The names actually found, for the transcript row. */
  names: string[];
}
export interface MemoryForgetDetails {
  kind: "forget";
  name: string;
}

// ---------------------------------------------------------------------------------------------
// Files (sync, like user-skills: the index read happens on the run path, between turns)

/** Every valid note in the directory. Dot-files and the trash folder never load. */
function readNotes(root: string): MemoryNote[] {
  let files: string[];
  try {
    files = readdirSync(root);
  } catch {
    return [];
  }
  const notes: MemoryNote[] = [];
  for (const file of files) {
    if (!file.endsWith(".md") || file.startsWith(".")) continue;
    let text: string;
    try {
      text = readFileSync(join(root, file), "utf-8");
    } catch {
      continue;
    }
    const note = parseMemory(file.slice(0, -3), text);
    if (note) notes.push(note);
  }
  return notes;
}

/** A fresh name that no file holds yet: `note-2`, `note-3`, … after a slug collision. */
function availableName(root: string, base: string): string {
  const held = new Set(tryList(root));
  if (!held.has(`${base}.md`)) return base;
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!held.has(`${candidate}.md`)) return candidate;
  }
}

function tryList(dir: string): string[] {
  try {
    return readdirSync(dir);
  } catch {
    return [];
  }
}

function writeNote(root: string, name: string, note: Omit<MemoryNote, "name">): void {
  mkdirSync(root, { recursive: true });
  // The leading dot keeps a half-written file out of the next index read, like skills.
  const staging = join(root, `.${name}.${randomUUID()}.tmp`);
  writeFileSync(staging, composeMemory(note), "utf-8");
  renameSync(staging, join(root, `${name}.md`));
}

function trashNote(root: string, name: string): void {
  const trash = join(root, TRASH_DIR);
  mkdirSync(trash, { recursive: true });
  let target = `${name}.md`;
  const held = new Set(tryList(trash));
  for (let suffix = 2; held.has(target); suffix += 1) target = `${name}-${suffix}.md`;
  renameSync(join(root, `${name}.md`), join(trash, target));
}

function oneLine(value: string, max: number): string {
  return value.replace(/\s+/g, " ").trim().slice(0, max);
}

// ---------------------------------------------------------------------------------------------
// Extension

export interface MemoryController {
  /** The host's payload: the project's memory directory and the effective on/off. */
  configure(payload: MemoryPayload | null | undefined): void;
  /**
   * Re-read the directory and rebuild the index the system prompt serves. True when that text
   * changed, so the caller rebuilds the system prompt. Called before each run (so Settings
   * edits apply too) and on `set_memory`.
   */
  refresh(): boolean;
  /** The index section for the system prompt, or undefined when memory is off or empty. */
  appendPrompt(): string | undefined;
  /** The directory the served index came from, for the append prompt's source attribution. */
  sourcePath(): string | undefined;
  /** The tools that must stay out of the active set while memory is off. */
  inactiveTools(): string[];
}

export function createMemoryExtension() {
  let root: string | undefined;
  let enabled = false;
  let servedIndex: string | undefined;

  const controller: MemoryController = {
    configure(payload) {
      root = payload?.root;
      enabled = payload?.enabled === true && typeof payload?.root === "string" && payload.root.length > 0;
    },
    refresh() {
      const previous = servedIndex;
      servedIndex = enabled && root ? memoryIndex(readNotes(root)) : undefined;
      return servedIndex !== previous;
    },
    appendPrompt() {
      return servedIndex;
    },
    sourcePath() {
      return enabled && root ? root : undefined;
    },
    inactiveTools() {
      return enabled ? [] : [...MEMORY_TOOL_NAMES];
    }
  };

  function factory(pi: ExtensionAPI): void {
    pi.registerTool({
      name: MEMORY_SAVE_TOOL_NAME,
      label: "Save memory",
      description:
        "Save or update a small note for future conversations in this project. Four kinds: user (the user's role and lasting preferences), feedback (corrections and confirmed approaches), project (ongoing work, decisions, deadlines not derivable from the code or git history), reference (where information lives outside the project).",
      promptSnippet: "keep a small note for future conversations",
      promptGuidelines: [
        "Save a memory only when it would help a future conversation, not every session. Never save what the codebase, AGENTS.md files or the transcript already show, or anything true only of the current task.",
        "Updating beats accumulating: pass the existing note's name to rewrite it, and retire notes that no longer hold with memory_forget.",
      ],
      parameters: SAVE_PARAMS,
      async execute(_toolCallId, params: unknown) {
        if (!enabled || !root) throw new Error("Memory is switched off for this project.");
        const input = params as Record<string, unknown>;
        const type = String(input.type ?? "");
        if (!isMemoryType(type)) throw new Error(`type must be one of ${MEMORY_TYPES.join(", ")}.`);
        const title = oneLine(String(input.title ?? ""), MAX_TITLE_CHARS);
        const description = oneLine(String(input.description ?? ""), MAX_DESCRIPTION_CHARS);
        const body = String(input.body ?? "").trim().slice(0, MAX_BODY_CHARS);
        if (!title) throw new Error("Give the memory a title.");
        if (!body) throw new Error("Give the memory a body.");
        const requested = typeof input.name === "string" && input.name.trim() ? input.name.trim() : undefined;
        if (requested && !isMemoryName(requested)) {
          throw new Error("Names use only lowercase letters, numbers, hyphens and underscores, like feedback_run-tests.");
        }
        const name = requested ?? availableName(root, memoryName(type, title));
        writeNote(root, name, { type, title, description, modified: new Date().toISOString(), body });
        return {
          content: [{ type: "text" as const, text: `${requested ? "Updated" : "Saved"} ${name}: ${title}` }],
          details: { kind: "save", name, title, type } satisfies MemorySaveDetails
        };
      }
    });

    pi.registerTool({
      name: MEMORY_RECALL_TOOL_NAME,
      label: "Recall memory",
      description:
        "Read saved memory notes in full. The memory index in the system prompt lists every note's name; call this with the names that look relevant to the current request.",
      promptSnippet: "read a saved memory note in full",
      promptGuidelines: [
        "Recall a note when its index line looks relevant to the request; the title and description are only a summary of the body.",
      ],
      parameters: RECALL_PARAMS,
      async execute(_toolCallId, params: unknown) {
        if (!enabled || !root) throw new Error("Memory is switched off for this project.");
        const raw = (params as { names?: unknown }).names;
        const names = (Array.isArray(raw) ? raw : []).map((name) => String(name).trim()).filter(Boolean).slice(0, MAX_RECALL_NAMES);
        if (names.length === 0) throw new Error("Give memory_recall at least one note name.");
        const notes = new Map(readNotes(root).map((note) => [note.name, note]));
        const parts: string[] = [];
        const found: string[] = [];
        for (const name of names) {
          const note = isMemoryName(name) ? notes.get(name) : undefined;
          if (!note) {
            parts.push(`No memory named '${name}'.`);
            continue;
          }
          found.push(name);
          const when = note.modified ? `\nSaved ${note.modified}` : "";
          parts.push(`# ${note.title} (${note.type})${when}\n${note.body}`);
        }
        return {
          content: [{ type: "text" as const, text: parts.join("\n\n") }],
          details: { kind: "recall", names: found } satisfies MemoryRecallDetails
        };
      }
    });

    pi.registerTool({
      name: MEMORY_FORGET_TOOL_NAME,
      label: "Forget memory",
      description: "Retire a saved memory note that no longer holds. It moves to the memory folder's .trash, out of the index.",
      promptSnippet: "retire a saved memory note",
      promptGuidelines: [],
      parameters: FORGET_PARAMS,
      async execute(_toolCallId, params: unknown) {
        if (!enabled || !root) throw new Error("Memory is switched off for this project.");
        const name = String((params as { name?: unknown }).name ?? "").trim();
        if (!name) throw new Error("Give memory_forget the note's name.");
        // A name that could not be a note file (a path, a dot-file) is simply not found.
        if (!isMemoryName(name) || !readNotes(root).some((note) => note.name === name)) {
          throw new Error(`No memory named '${name}'.`);
        }
        trashNote(root, name);
        return {
          content: [{ type: "text" as const, text: `Forgot ${name}.` }],
          details: { kind: "forget", name } satisfies MemoryForgetDetails
        };
      }
    });
  }

  return { factory, controller };
}
