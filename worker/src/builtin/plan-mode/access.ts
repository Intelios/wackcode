/** Transient access guidance; the persisted mode contracts must remain byte-identical. */
import { latestModeContract } from "./contract.js";

export const EXECUTION_POLICY_MESSAGE_TYPE = "wackcode-execution-policy";

export function reconcilePlanningAccess<T>(messages: T[], unrestricted: boolean): T[] {
  const clean = messages.filter((message) =>
    (message as { customType?: string }).customType !== EXECUTION_POLICY_MESSAGE_TYPE);
  if (!unrestricted) return clean.length === messages.length ? messages : clean;
  // Keep the actual request last and assistant/tool exchanges adjacent. Pi converts custom
  // messages to user messages; appending here would replace the current request in context.
  const insertion = (latestModeContract(clean)?.index ?? -1) + 1;
  return [...clean.slice(0, insertion), {
    role: "custom",
    customType: EXECUTION_POLICY_MESSAGE_TYPE,
    display: false,
    timestamp: 0,
    content: "[WACKCODE CURRENT PLANNING ACCESS POLICY]\nThe user explicitly removed the app's read-only restrictions for Plan and Ultra Plan. This supersedes earlier app-provided instructions prohibiting mutating actions or restricting planning tools to inspection. You may run unrestricted shell commands and use all enabled tools, including file edits, packages, MCP, browser and permitted computer actions, when they help prepare the plan. Tool switches, access controls and explicit task constraints still apply. Remain in the current planning mode: retain the interview and plan submission/review workflow, and do not implement the finished plan before the user approves it. Sub-agents retain their own effective access policy.",
  } as unknown as T, ...clean.slice(insertion)];
}
