/**
 * Fallback detection of a `<proposed_plan>` block in assistant prose. Ported from
 * `@narumitw/pi-plan-mode` v0.58.3 (MIT) `message-transform.ts` — weaker models sometimes
 * write the plan as text instead of calling plan_mode_complete; a single well-formed block
 * is accepted as a completion.
 */
const PROPOSED_PLAN_PATTERN = /^<proposed_plan>[\t ]*\r?\n([\s\S]*?)\r?\n<\/proposed_plan>[\t ]*$/gm;

export type ProposedPlanParseResult =
  | { kind: "absent" }
  | { kind: "valid"; plan: string }
  | { kind: "empty" }
  | { kind: "multiple" }
  | { kind: "malformed" }
  | { kind: "unclosed" };

export function parseProposedPlan(text: string): ProposedPlanParseResult {
  const openingCount = text.match(/<proposed_plan>/gi)?.length ?? 0;
  const closingCount = text.match(/<\/proposed_plan>/gi)?.length ?? 0;
  if (openingCount === 0 && closingCount === 0) return { kind: "absent" };
  if (openingCount > 1 || closingCount > 1) return { kind: "multiple" };
  if (openingCount === 1 && closingCount === 0) return { kind: "unclosed" };
  if (openingCount !== 1 || closingCount !== 1) return { kind: "malformed" };

  const matches = Array.from(text.matchAll(PROPOSED_PLAN_PATTERN));
  if (matches.length !== 1) return { kind: "malformed" };
  const plan = matches[0]?.[1]?.trim() ?? "";
  return plan ? { kind: "valid", plan } : { kind: "empty" };
}

export function invalidPlanMessage(kind: "empty" | "multiple" | "malformed" | "unclosed") {
  const detail = {
    empty: "the block is empty",
    multiple: "more than one plan block was produced",
    malformed: "the tags must be on their own lines",
    unclosed: "the closing tag is missing",
  }[kind];
  return `The proposed plan is not ready: ${detail}. The agent continues planning and must submit the plan with plan_mode_complete.`;
}

export function latestAssistantText(messages: unknown) {
  if (!Array.isArray(messages)) return "";
  for (const entry of [...messages].reverse()) {
    const message = (entry as { message?: SessionMessage })?.message ?? (entry as SessionMessage);
    if (message?.role !== "assistant") continue;
    const text = contentText(message.content);
    if (text) return text;
  }
  return "";
}

interface SessionMessage {
  role?: string;
  content?: unknown;
}

function contentText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => {
      const textBlock = block as { type?: string; text?: string };
      return textBlock.type === "text" && typeof textBlock.text === "string" ? textBlock.text : "";
    })
    .filter(Boolean)
    .join("\n");
}
