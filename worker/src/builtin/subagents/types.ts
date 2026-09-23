export const SUBAGENT_TOOL_NAME = "subagent";
export const SUBAGENT_TOOL_LABEL = "Sub-agents";

/** Tasks one parallel call may carry. */
export const MAX_PARALLEL_TASKS = 8;

/** Pi's own tools a sub-agent may be given. Package tools never reach a child. */
export const CHILD_TOOLS = ["read", "grep", "find", "ls", "bash", "edit", "write"] as const;

/** What a read-only agent may use at all; its bash is further limited by the Plan-mode policy. */
export const READ_ONLY_TOOLS = new Set<string>(["read", "grep", "find", "ls", "bash"]);

/** The tools a read-only child runs without any argument check. */
export const INSPECTION_TOOLS = new Set<string>(["read", "grep", "find", "ls"]);

/**
 * The tools a child actually gets: its role's list, limited to Pi's own tools (and to
 * inspection tools for a read-only agent), minus anything unavailable here or switched off in
 * Settings → Tools. `available` is already filtered for the last two.
 */
export function resolveChildTools(tools: readonly string[], readOnly: boolean, available: readonly string[]): string[] {
  const offered = new Set(available);
  const known = new Set<string>(CHILD_TOOLS);
  return [...new Set(tools)].filter(
    (tool) => known.has(tool) && offered.has(tool) && (!readOnly || READ_ONLY_TOOLS.has(tool)),
  );
}
