import { createContext } from "react";
import type { AppearanceConfig } from "./types";

/** The name the app calls the agent when the user hasn't picked their own. */
export const DEFAULT_AGENT_NAME = "WackCode";

/** The resolved persona name for the app's own copy (placeholders, notices, run labels). */
export function agentName(appearance: AppearanceConfig): string {
  return appearance.agentName?.trim() || DEFAULT_AGENT_NAME;
}

/**
 * Names the assistant ("Nova is working…"). Separate from a sub-agent's own name, which the
 * `subagent` tool and its cards keep verbatim. Provided from App around the transcript; the
 * Composer instead takes the name as a prop because its strings are computed outside render.
 */
export const AssistantNameContext = createContext(DEFAULT_AGENT_NAME);
