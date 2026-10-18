/**
 * Built-in `suggest_task` tool: the agent files a follow-up task it noticed while working — a
 * bug, a rough edge, a missing bit of polish — that is outside the task it was asked to do.
 *
 * It does not fix the thing and it does not bury it in a footnote. The call records a title, a
 * kind and a self-contained brief; the desktop draws one card per accepted suggestion under the
 * turn's final answer (`src/components/SuggestionTray.tsx`), where the user can start it here,
 * start it in a new chat, or dismiss it. The content lives in the call's saved arguments, so
 * saved history and cold reopening need nothing extra from this tool — like `show_visual`.
 *
 * The tool only records. It is read-only, so Plan and Ultra Plan allow it (the plan-mode
 * helper list), and it writes nothing to disk. Guards, because a model that can file tasks will
 * file too many: at most `MAX_SUGGESTIONS_PER_RUN` per run, and a title already suggested on
 * this session branch is refused. Chat mode never loads it (no coding work there) and
 * sub-agents never get it (`childToolNames`): a child has no transcript to draw a card in, so it
 * reports what it noticed and the parent decides. The user can switch it off in Settings ›
 * Tools › Built-ins (`SWITCHABLE_BUILTIN_TOOLS`); saved cards still render.
 */
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

export const SUGGEST_TASK_TOOL_NAME = "suggest_task";
/** Read by the worker's tool filters: sub-agents never get it. */
export const SUGGEST_TASK_TOOL_NAMES = [SUGGEST_TASK_TOOL_NAME] as const;
export const MAX_SUGGESTION_TITLE = 80;
export const MAX_SUGGESTION_DESCRIPTION = 2_000;
/** Suggestions one run may file. Further calls are refused, so a turn cannot become a backlog. */
export const MAX_SUGGESTIONS_PER_RUN = 5;

export const SUGGESTION_KINDS = ["bug", "polish", "improvement"] as const;
export type SuggestionKind = typeof SUGGESTION_KINDS[number];

export const SUGGEST_TASK_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["title", "kind", "description"],
  properties: {
    title: {
      type: "string",
      description: `A short imperative title for the follow-up task, 1-${MAX_SUGGESTION_TITLE} characters, e.g. "Dismiss button stays disabled after a failed save".`,
    },
    kind: {
      type: "string",
      enum: [...SUGGESTION_KINDS],
      description: "bug: something is broken. polish: it works but looks or feels rough. improvement: a missing convenience.",
    },
    description: {
      type: "string",
      description:
        `A self-contained Markdown brief for a fresh agent with none of this conversation: what is wrong, where (file paths and lines), how you noticed it, and the fix you would make. At most ${MAX_SUGGESTION_DESCRIPTION} characters.`,
    },
  },
} as const;

/**
 * What the model must know about the tool. Pi drops these once a chat prompt is set, so Chat
 * mode's guide would carry them too — except Chat mode never loads this tool.
 */
export const SUGGEST_TASK_GUIDE_LINES: readonly string[] = [
  "Use suggest_task for a concrete problem you noticed that is outside the task you were asked to do: a bug nearby, a rough edge, a missing bit of polish or convenience. It records the task; the user sees a card with it after your final message and decides whether to start it.",
  "Never use it for work you are going to do in this task, for ideas you are not sure about, or for a list of nice-to-haves. Prefer no suggestion to a weak one.",
  "Write the description as a brief a fresh agent could act on with no access to this conversation: what is wrong, where it is (file paths and line numbers), how you noticed it, and what you would change.",
  "File each suggestion once; a repeated title is refused. Don't work on a suggested task now, and mention it only in passing in your final message — the card carries the details.",
];

type Normalized =
  | { ok: true; suggestion: { title: string; kind: SuggestionKind; description: string } }
  | { ok: false; error: string };

/** A title as both the card and the duplicate check see it: one line, one space between words. */
function cleanTitle(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

export function isSuggestionKind(value: unknown): value is SuggestionKind {
  return typeof value === "string" && (SUGGESTION_KINDS as readonly string[]).includes(value);
}

export function normalizeSuggestTaskParams(input: unknown): Normalized {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "Expected an object with a title, a kind and a description." };
  }
  const { title, kind, description } = input as Record<string, unknown>;
  if (typeof title !== "string" || !cleanTitle(title)) return { ok: false, error: "title must be a short non-empty string." };
  const clean = cleanTitle(title);
  if (clean.length > MAX_SUGGESTION_TITLE) {
    return { ok: false, error: `title must be at most ${MAX_SUGGESTION_TITLE} characters.` };
  }
  if (!isSuggestionKind(kind)) {
    return { ok: false, error: `kind must be one of ${SUGGESTION_KINDS.join(", ")}.` };
  }
  if (typeof description !== "string" || !description.trim()) {
    return { ok: false, error: "description must be a non-empty Markdown brief." };
  }
  const brief = description.trim();
  if (brief.length > MAX_SUGGESTION_DESCRIPTION) {
    return {
      ok: false,
      error: `description is ${brief.length} characters, over the ${MAX_SUGGESTION_DESCRIPTION} limit. Keep the brief to what a fresh agent needs.`,
    };
  }
  return { ok: true, suggestion: { title: clean, kind, description: brief } };
}

/** The duplicate key: case and spacing never make a second suggestion distinct. */
export function suggestionKey(title: string): string {
  return cleanTitle(title).toLowerCase();
}

export interface SuggestTaskDetails {
  /** Versioned result shape, like the other WackCode cards. */
  v: 1;
  kind: SuggestionKind;
  title: string;
}

export function isSuggestTaskDetails(value: unknown): value is SuggestTaskDetails {
  if (!value || typeof value !== "object") return false;
  const details = value as Record<string, unknown>;
  return details.v === 1 && typeof details.title === "string" && isSuggestionKind(details.kind);
}

type SessionEntry = {
  type?: string;
  message?: {
    role?: string;
    toolName?: string;
    details?: unknown;
  };
};

/**
 * Titles already suggested on this session branch, oldest first. The tool *results* are the
 * record — a call that was refused is an error and carries no details — so this follows a
 * rewind the same way the transcript does, and survives a restart without any disk write.
 */
export function suggestedTitlesFromBranch(branch: Iterable<unknown>): string[] {
  const titles: string[] = [];
  for (const raw of branch) {
    const entry = raw as SessionEntry;
    if (entry?.type !== "message") continue;
    const message = entry.message;
    if (message?.role !== "toolResult" || message.toolName !== SUGGEST_TASK_TOOL_NAME) continue;
    if (!isSuggestTaskDetails(message.details)) continue;
    titles.push(message.details.title);
  }
  return titles;
}

interface SuggestContext {
  sessionManager: { getBranch(): unknown[] };
}

export function createSuggestTaskExtension() {
  // Per run, not per session: a long chat may still learn something new each turn.
  let accepted = 0;

  return function suggestTask(pi: ExtensionAPI) {
    pi.registerTool({
      name: SUGGEST_TASK_TOOL_NAME,
      label: "Suggest task",
      description:
        "File a follow-up task you noticed while working — a bug, a rough edge or missing polish outside what you were asked to do. It is recorded, not worked on: the user gets a card after your final message and decides whether to start it here or in a new chat. Write the description as a brief a fresh agent could act on without this conversation.",
      promptSnippet: "file a follow-up task you noticed but are not doing now",
      promptGuidelines: [...SUGGEST_TASK_GUIDE_LINES],
      parameters: SUGGEST_TASK_PARAMS,
      async execute(_toolCallId, params: unknown, _signal, _onUpdate, ctx) {
        const parsed = normalizeSuggestTaskParams(params);
        if (!parsed.ok) throw new Error(parsed.error);
        const { title, kind } = parsed.suggestion;
        const key = suggestionKey(title);
        const context = ctx as unknown as SuggestContext | undefined;
        const already = context?.sessionManager
          ? suggestedTitlesFromBranch(context.sessionManager.getBranch())
          : [];
        if (already.some((existing) => suggestionKey(existing) === key)) {
          throw new Error(`“${title}” is already suggested in this chat. File a different task, or do this one instead.`);
        }
        if (accepted >= MAX_SUGGESTIONS_PER_RUN) {
          throw new Error(
            `This run already filed ${MAX_SUGGESTIONS_PER_RUN} suggested tasks, which is the limit. Keep only the important ones, or finish the work you were asked to do.`
          );
        }
        accepted += 1;
        return {
          content: [{
            type: "text" as const,
            text: `Suggested “${title}”. It appears to the user as a card under your final message, with actions to start it later. Don't work on it now, and don't repeat its details in your reply.`,
          }],
          details: { v: 1, kind, title } satisfies SuggestTaskDetails,
        };
      },
    });

    // A run's limit resets once it settles; a goal loop's next round starts counting again.
    pi.on("agent_end", () => {
      accepted = 0;
    });
  };
}
