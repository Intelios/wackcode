/** Resolve a child once per call; changing Settings cannot alter a running child. */
import type { ExecutionPolicyConfig, SubagentSpec, TaskMode } from "../../protocol.js";
import { isReadOnlyPlanning } from "../../execution-policy.js";

export function resolveSubagentSpec(spec: SubagentSpec, mode: TaskMode, policy: ExecutionPolicyConfig): SubagentSpec {
  const unlocked = spec.readOnly && policy.unrestrictedSubagents && !isReadOnlyPlanning(mode, policy);
  const readOnly = spec.readOnly && !unlocked;
  // Only shipped roles contain app-owned policy prose. Never rewrite a custom role's text.
  const prompt = unlocked && spec.builtin
    ? spec.prompt.replace(/^You are read-only:[^\n]*\n\n/m, "")
    : spec.prompt;
  const access = readOnly
    ? "App access policy: this child is read-only. Inspect files and run inspection commands only; never modify files."
    : "App access policy: this child may edit files and run unrestricted shell commands through its enabled tools. The app's read-only restrictions are disabled for this run. Follow the delegated task and retain any explicit task or custom-role constraints, including requests to review without editing.";
  return {
    ...spec,
    readOnly,
    tools: unlocked ? [...new Set([...spec.tools, "edit", "write"])] : spec.tools,
    description: unlocked && spec.builtin
      ? spec.description.replace(/read-only\s*/gi, "").replace(/, and never edits\./g, ".")
      : spec.description,
    prompt: `${prompt}\n\n${access}${mode !== "build" ? " You are assisting a planning turn: make changes only to prepare the plan, and do not implement the finished plan before the user approves it." : ""}`,
  };
}
