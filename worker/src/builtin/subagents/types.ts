export const SUBAGENT_TOOL_NAME = "subagent";
export const SUBAGENT_TOOL_LABEL = "Sub-agents";
export const SUBAGENT_JOB_TOOL_NAME = "subagent_job";

/** Tasks one parallel call may carry. */
export const MAX_PARALLEL_TASKS = 8;

/**
 * The tools a sub-agent may be given: Pi's own, plus the built-in `web_fetch` and `web_search`,
 * which reach a child as their own extensions (see `index.ts`). Package tools never reach a child.
 */
export const CHILD_TOOLS = ["read", "grep", "find", "ls", "bash", "edit", "write", "web_fetch", "web_search"] as const;

/** What a read-only agent may use at all; its bash is further limited by the Plan-mode policy. */
export const READ_ONLY_TOOLS = new Set<string>(["read", "grep", "find", "ls", "bash", "web_fetch", "web_search"]);

/** The tools a read-only child runs without any argument check. `web_fetch` and `web_search`
 *  only read; the first applies its own public-address policy, the second goes through Rust. */
export const INSPECTION_TOOLS = new Set<string>(["read", "grep", "find", "ls", "web_fetch", "web_search"]);

/**
 * The tools a child actually gets: its role's list, limited to `CHILD_TOOLS` (and to
 * inspection tools for a read-only agent), minus anything unavailable here or switched off in
 * Settings (Tools, or Web Fetch's card in Packages). `available` is already filtered for the
 * last two.
 */
export function resolveChildTools(tools: readonly string[], readOnly: boolean, available: readonly string[]): string[] {
  const offered = new Set(available);
  const known = new Set<string>(CHILD_TOOLS);
  return [...new Set(tools)].filter(
    (tool) => known.has(tool) && offered.has(tool) && (!readOnly || READ_ONLY_TOOLS.has(tool)),
  );
}
