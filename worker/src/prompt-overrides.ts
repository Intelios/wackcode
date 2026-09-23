/**
 * User-customized built-in prompt texts (Settings → Prompts), held where every consumer reads
 * them per use: the loader wrapper serves the custom persona to each system-prompt rebuild, and
 * `modeContractContent` composes each Plan/Ultra contract from the current body. Defaults are
 * what an absent override leaves in place — the shipped texts stay byte-identical.
 *
 * The host delivers the same values in `init` and (live) in `set_prompts`; like the other live
 * settings they never enter the worker fingerprint, so changing a prompt never respawns a worker.
 */
import type { PromptOverrides } from "./protocol.js";

let overrides: PromptOverrides = {};

/** Blanket-undefined assignment keeps an empty object rather than stale fields from before. */
export function setPromptOverrides(next: PromptOverrides | null | undefined): void {
  overrides = {
    systemPrompt: nonEmpty(next?.systemPrompt),
    planPrompt: nonEmpty(next?.planPrompt),
    ultraPlanPrompt: nonEmpty(next?.ultraPlanPrompt)
  };
}

export function promptOverrides(): Readonly<PromptOverrides> {
  return overrides;
}

function nonEmpty(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}
