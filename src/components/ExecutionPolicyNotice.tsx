import type { ExecutionPolicyConfig, TaskMode } from "../types";
import { DEFAULT_EXECUTION_POLICY, sameExecutionPolicy } from "../execution-policy";
import { Tooltip } from "./ui/Tooltip";

/** The applied snapshot wins during a run, including when restrictions were just restored. */
export function ExecutionPolicyNotice({ saved, applied, running, mode, subagentsEnabled, appliedSubagentsEnabled }: {
  saved: ExecutionPolicyConfig;
  applied?: ExecutionPolicyConfig;
  running: boolean;
  mode: TaskMode;
  subagentsEnabled: boolean;
  appliedSubagentsEnabled?: boolean;
}) {
  const active = running ? applied ?? DEFAULT_EXECUTION_POLICY : saved;
  const pending = running && !sameExecutionPolicy(saved, active);
  const planning = mode !== "build";
  const planOff = planning && active.unrestrictedPlanning;
  const childrenEnabled = running ? appliedSubagentsEnabled ?? subagentsEnabled : subagentsEnabled;
  const childrenOff = childrenEnabled && active.unrestrictedSubagents && (!planning || planOff);
  if (!planOff && !childrenOff && !pending) return null;
  const scope = [planOff ? "Plan / Ultra Plan" : "", childrenOff ? "normally read-only sub-agents" : ""].filter(Boolean).join(" and ");
  const tip = `${scope ? `Read-only restrictions are disabled for ${scope}. Dangerous and not recommended: enabled tools can change files on your device and run unrestricted commands. ` : ""}${planOff ? "Planning can also use enabled tools that change connected services. " : ""}${pending ? "Your saved changes apply next turn. Changing Settings does not stop this run." : "Configure in Settings › Tools."}`;
  return <Tooltip label={tip}>
    <span className="execution-policy-notice" role="status" tabIndex={0} aria-label={tip}>
      {scope && <span>Read-only off</span>}
      {pending && <span className="execution-policy-pending">Applies next turn</span>}
    </span>
  </Tooltip>;
}
