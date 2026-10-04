/** Live access settings. These are app settings, never session history or worker fingerprints. */
import type { ExecutionPolicyConfig, TaskMode } from "./protocol.js";

export const DEFAULT_EXECUTION_POLICY: ExecutionPolicyConfig = {
  unrestrictedSubagents: false,
  unrestrictedPlanning: false,
};

export function normalizeExecutionPolicy(policy?: Partial<ExecutionPolicyConfig>): ExecutionPolicyConfig {
  return {
    unrestrictedSubagents: policy?.unrestrictedSubagents === true,
    unrestrictedPlanning: policy?.unrestrictedPlanning === true,
  };
}

export function isReadOnlyPlanning(mode: TaskMode, policy: ExecutionPolicyConfig): boolean {
  return mode !== "build" && !policy.unrestrictedPlanning;
}

export function sameExecutionPolicy(a?: ExecutionPolicyConfig, b?: ExecutionPolicyConfig): boolean {
  return a?.unrestrictedSubagents === b?.unrestrictedSubagents
    && a?.unrestrictedPlanning === b?.unrestrictedPlanning;
}
