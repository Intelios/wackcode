/**
 * Ultra: the reasoning step above a model's own top level. The host stores it as the chat's
 * thinking level ("ultra"), but it is WackCode's, never Pi's — Pi only knows `off`…`max`.
 *
 * Invariant: "ultra" never reaches Pi. `piThinkingLevel` translates it to `max`, which Pi's
 * `clampThinkingLevel` lowers to the highest level the model record lists, and the chat's system
 * prompt gains `ultraPrompt()` in its addendum (both chat kinds). A level change respawns the
 * worker (`configure_task`), so the flag is fixed for a worker's life and the instruction is part
 * of the first prompt it builds. Sub-agents on the chat's model inherit `session.thinkingLevel`,
 * which is already the real clamped level, so they run at the top level without the instruction.
 *
 * `src/promptDefaults.ts` mirrors the default byte for byte for Settings › Prompts.
 */
import { promptOverrides } from "../prompt-overrides.js";
import { ULTRA_EFFORT, type ReasoningEffort, type ThinkingLevel } from "../protocol.js";

export const DEFAULT_ULTRA_PROMPT = `# Ultra
The user set this chat to Ultra: you are running at your highest reasoning effort, and speed and cost are not the goal. Optimise for the most exhaustive, correct answer — not the fastest or cheapest.
- Investigate fully before you act or conclude. Read the relevant files and context rather than assuming, and follow each lead to the end.
- Weigh alternatives and edge cases, and check your reasoning for gaps before you commit to an answer.
- Verify thoroughly: run the checks that would catch you being wrong, and don't settle for the first answer that looks plausible.
- Ultra changes how hard you work, not what you may do. Every other instruction still applies, and your replies stay as clear and readable as usual.`;

export function isUltra(effort: ReasoningEffort | undefined): boolean {
  return effort === ULTRA_EFFORT;
}

/** The level to hand Pi: Ultra asks for `max`, and Pi clamps it to the model's highest. */
export function piThinkingLevel(effort: ReasoningEffort): ThinkingLevel {
  return effort === ULTRA_EFFORT ? "max" : effort;
}

/** The instruction an Ultra chat's system prompt gains: the user's custom text, or the default. */
export function ultraPrompt(): string {
  return promptOverrides().ultraPrompt ?? DEFAULT_ULTRA_PROMPT;
}
