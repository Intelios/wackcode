/**
 * The read-only policy for a child session. A read-only agent is only ever given
 * read/grep/find/ls/bash/web_fetch (and bash_job for its own commands). This enforces the rest
 * at call time with Plan mode's fail-closed shell policy, so "read-only" means the same thing
 * for a planning parent and a scout.
 *
 * Loaded into the child alongside shell job management and, at most, Web Fetch. Pi installs `tool_call` hooks in the
 * session constructor, so it takes effect without binding a UI; a handler that throws blocks
 * the call.
 */
import type { ExtensionAPI, InlineExtension } from "@earendil-works/pi-coding-agent";
import { DEFAULT_SAFE_SUBCOMMANDS, findBlockedCommandSegment, readCommand } from "../plan-mode/policy.js";
import { INSPECTION_TOOLS } from "./types.js";
import { BASH_JOB_TOOL_NAME } from "../bash-jobs.js";

export type GuardDecision = { block: true; reason: string } | undefined;

/** The decision for one call. Pure, so the policy can be tested without a session. */
export function readOnlyDecision(toolName: string, input: unknown, cwd: string | undefined): GuardDecision {
  // This can only inspect/wait/stop commands already admitted by this child's bash policy;
  // there is no arbitrary pid, shell text, or other session's job in its input.
  if (INSPECTION_TOOLS.has(toolName) || toolName === BASH_JOB_TOOL_NAME) return undefined;
  if (toolName === "bash") {
    const blocked = findBlockedCommandSegment(readCommand(input), DEFAULT_SAFE_SUBCOMMANDS, cwd);
    return blocked === undefined
      ? undefined
      : {
          block: true,
          reason: `This sub-agent is read-only: only inspection commands (search, status, logs, tests) are allowed.\nBlocked command: ${blocked}`,
        };
  }
  return { block: true, reason: `This sub-agent is read-only and cannot use '${toolName}'.` };
}

export function readOnlyGuard(): InlineExtension {
  return {
    name: "wackcode-subagent-read-only",
    hidden: true,
    factory: (pi: ExtensionAPI) => {
      pi.on("tool_call", (event, ctx) => readOnlyDecision(event.toolName, event.input, ctx.cwd));
    },
  };
}
