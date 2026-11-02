/** The agent can enter either planning variant, but only the user can return it to Build. */
import type { PlanVariant } from "./prompt.js";

export const PLAN_MODE_ENTER_TOOL_NAME = "plan_mode_enter";

export const PLAN_MODE_ENTER_PARAMS = {
  type: "object",
  additionalProperties: false,
  properties: {
    mode: {
      type: "string",
      enum: ["plan", "ultraplan"],
      description: "Planning variant: plan (the default), or ultraplan for an exhaustive interview.",
    },
  },
} as const;

export function normalizePlanModeEntry(input: unknown): PlanVariant {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("Provide an object with an optional mode: plan or ultraplan.");
  }
  const mode = (input as Record<string, unknown>).mode;
  if (mode === undefined) return "plan";
  if (mode !== "plan" && mode !== "ultraplan") {
    throw new Error("Planning mode must be plan or ultraplan; only the user can switch back to Build.");
  }
  return mode;
}
