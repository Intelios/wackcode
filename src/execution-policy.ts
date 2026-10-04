/** Saved access settings; a running worker's snapshot is authoritative until its next turn. */
import type { ExecutionPolicyConfig } from "./types";

export const DEFAULT_EXECUTION_POLICY: ExecutionPolicyConfig = {
  unrestrictedSubagents: false,
  unrestrictedPlanning: false,
};

export const EXECUTION_POLICY_WARNING = "Dangerous and not recommended. This allows the agent to create, edit or delete files on your device and run unrestricted shell commands, including outside the project.";

export function sameExecutionPolicy(a: ExecutionPolicyConfig, b: ExecutionPolicyConfig): boolean {
  return a.unrestrictedSubagents === b.unrestrictedSubagents
    && a.unrestrictedPlanning === b.unrestrictedPlanning;
}
