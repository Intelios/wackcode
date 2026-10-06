import type { ExecutionPolicyConfig } from "../types";
import { DEFAULT_EXECUTION_POLICY, sameExecutionPolicy } from "../execution-policy";
import { Tooltip } from "./ui/Tooltip";

/**
 * Settings › Tools changes queue between turns: the worker's applied snapshot wins for the rest
 * of a run, and only the mismatch with saved Settings is announced here. It never states the
 * active policy — effective access is shown in Settings (› Tools and › Sub-agents), not the chat.
 */
export function ExecutionPolicyNotice({ saved, applied, running }: {
  saved: ExecutionPolicyConfig;
  applied?: ExecutionPolicyConfig;
  running: boolean;
}) {
  const active = running ? applied ?? DEFAULT_EXECUTION_POLICY : saved;
  if (!running || sameExecutionPolicy(saved, active)) return null;
  const tip = "Your saved changes apply next turn. Changing Settings does not stop this run.";
  return <Tooltip label={tip}>
    <span className="execution-policy-notice" role="status" tabIndex={0} aria-label={tip}>
      <span className="execution-policy-pending">Applies next turn</span>
    </span>
  </Tooltip>;
}
