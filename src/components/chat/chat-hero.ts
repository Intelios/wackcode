/**
 * The Chat hero's words, as pure functions: a greeting for the time of day and a few prompt
 * ideas. The ideas only fill the composer; nothing is sent until the user does.
 */

export function greeting(hour: number): string {
  if (hour < 5) return "Up late";
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

export interface PromptIdea {
  /** Chip text. */
  label: string;
  /** What it puts in the composer. */
  prompt: string;
}

export const PROMPT_IDEAS: readonly PromptIdea[] = [
  { label: "Think something through", prompt: "Help me think through " },
  { label: "Explain it simply", prompt: "Explain like I'm new to it: " },
  { label: "Draft a message", prompt: "Help me write a friendly message to " },
  { label: "Plan my week", prompt: "Help me plan my week. Here's what's on my plate: " },
  { label: "Brainstorm names", prompt: "Brainstorm ten names for " },
  { label: "Compare options", prompt: "Compare these options and help me choose: " },
  { label: "Summarise a page", prompt: "Summarise this page for me: " },
  { label: "Learn something new", prompt: "Teach me the basics of " },
  { label: "Cook with what I have", prompt: "What can I cook with " },
  { label: "Polish my writing", prompt: "Make this read better, keep my voice:\n\n" },
  { label: "Gift ideas", prompt: "Gift ideas for someone who loves " },
  { label: "Settle a debate", prompt: "Settle a friendly debate: " }
];

/** `count` distinct ideas for a visit, chosen deterministically from `seed`. */
export function pickIdeas(seed: number, count = 3, ideas: readonly PromptIdea[] = PROMPT_IDEAS): PromptIdea[] {
  const pool = [...ideas];
  const picked: PromptIdea[] = [];
  let state = Math.abs(Math.floor(seed)) % 2147483647 || 1;
  while (picked.length < Math.min(count, ideas.length)) {
    state = (state * 48271) % 2147483647;
    picked.push(pool.splice(state % pool.length, 1)[0]);
  }
  return picked;
}
