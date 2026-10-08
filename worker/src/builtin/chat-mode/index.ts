/**
 * Chat mode's guard: the built-in that enforces `policy.ts` on every tool call in a Chat mode
 * chat. Loaded only into those chats (`chatFactories` in `../index.ts`), in place of the coding
 * built-ins.
 *
 * Keeping a tool out of the active set is not enforcement on its own: a navigation restores the
 * transcript's declared tools, and an MCP server can register a tool late. So this blocks at call
 * time, fail-closed, whatever the active set says. Pi installs `tool_call` hooks in the session
 * constructor, and a handler that throws blocks the call.
 */
import type { ExtensionAPI, InlineExtension } from "@earendil-works/pi-coding-agent";
import { chatToolDecision } from "./policy.js";

export interface ChatGuardDeps {
  /** The chat's scratchpad, for a call that arrives without a working directory. */
  workspace(): string | undefined;
  /** Whether a tool belongs to one of the user's MCP servers. */
  ownsMcpTool(name: string): boolean;
}

export function createChatGuardExtension(deps: ChatGuardDeps): InlineExtension {
  return {
    name: "wackcode-chat-mode",
    hidden: true,
    factory: (pi: ExtensionAPI) => {
      pi.on("tool_call", (event, ctx) => chatToolDecision(event.toolName, event.input, ctx.cwd || deps.workspace(), deps.ownsMcpTool));
    },
  };
}
