import type { SlashCommand } from "./types";

/**
 * WackCode's own slash commands, run by the app itself rather than a worker. The Settings
 * "WackCode" group renders this same list, so the two always agree. The picker's resolved
 * names reserve these first (see `resolveCommandNames` in worker/src/slash.ts).
 */
export const APP_SLASH_COMMANDS: SlashCommand[] = [
  { id: "app:compact", name: "compact", description: "Summarize older conversation context", source: "app", sourceLabel: "WackCode" },
  { id: "app:init", name: "init", description: "Create or refine project AGENTS.md", source: "app", sourceLabel: "WackCode" },
  { id: "app:new", name: "new", description: "Open a new chat", source: "app", sourceLabel: "WackCode" },
  { id: "app:name", name: "name", description: "Rename this chat", source: "app", sourceLabel: "WackCode" },
  { id: "app:copy", name: "copy", description: "Copy the latest assistant message", source: "app", sourceLabel: "WackCode" },
  { id: "app:goal", name: "goal", description: "Keep iterating until the goal is verified. /goal pause, resume, clear control it.", source: "app", sourceLabel: "WackCode" }
];

export const COMMAND_NAME_MAX = 64;
export const COMMAND_DESCRIPTION_MAX = 1_024;
export const COMMAND_HINT_MAX = 256;
export const COMMAND_BODY_MAX = 200_000;

/** Mirrors `validate_name` in `slash_commands.rs`; returns an error sentence or undefined. */
export function validateCommandName(name: string): string | undefined {
  const trimmed = name.trim();
  if (!trimmed) return "Give the command a name.";
  if ([...trimmed].length > COMMAND_NAME_MAX) return `A command name can be at most ${COMMAND_NAME_MAX} characters.`;
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(trimmed)) return "Use only lowercase letters, numbers and single hyphens, like review-diff.";
  return undefined;
}

export function validateCommandDescription(description: string): string | undefined {
  return [...description.trim()].length > COMMAND_DESCRIPTION_MAX
    ? `A description can be at most ${COMMAND_DESCRIPTION_MAX} characters.`
    : undefined;
}

export function validateCommandHint(hint: string): string | undefined {
  return [...hint.trim()].length > COMMAND_HINT_MAX
    ? `An argument hint can be at most ${COMMAND_HINT_MAX} characters.`
    : undefined;
}

export function validateCommandBody(body: string): string | undefined {
  const trimmed = body.trim();
  if (!trimmed) return "The command needs instructions to expand to.";
  return [...trimmed].length > COMMAND_BODY_MAX
    ? `A command's instructions can be at most ${COMMAND_BODY_MAX} characters.`
    : undefined;
}

/**
 * Pi 0.86.1's prompt-template argument rules, for the editor's live preview — the same rules
 * the worker's `expandTemplate` (worker/src/slash.ts) applies when the command runs.
 */
export function expandCommandPreview(content: string, input: string): string {
  const args: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const char of input) {
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
    } else if (char === "'" || char === '"') quote = char;
    else if (/\s/.test(char)) {
      if (current) { args.push(current); current = ""; }
    } else current += char;
  }
  if (current) args.push(current);
  const all = args.join(" ");
  return content.replace(/\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g,
    (_match, target: string | undefined, fallback: string | undefined, sliceFrom: string | undefined, sliceCount: string | undefined, simple: string | undefined) => {
      if (target) {
        const value = target === "@" || target === "ARGUMENTS" ? all : args[Number(target) - 1];
        return value || fallback || "";
      }
      if (sliceFrom) {
        const start = Math.max(0, Number(sliceFrom) - 1);
        return args.slice(start, sliceCount ? start + Number(sliceCount) : undefined).join(" ");
      }
      if (simple === "@" || simple === "ARGUMENTS") return all;
      return args[Number(simple) - 1] ?? "";
    });
}
