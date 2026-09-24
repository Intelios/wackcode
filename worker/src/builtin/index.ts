/**
 * WackCode's built-in extensions: compiled into the worker, loaded through Pi's inline-factory
 * mechanism rather than the trusted-package path. They are ordinary code in this repo — no
 * trust gate applies because nothing external can reach this list. All are always on except
 * sub-agents, which the user switches on in Settings (its tool stays inactive until then),
 * web fetch, which is on until the user switches it off (see `SWITCHABLE_BUILTIN_TOOLS`), and
 * MCP, whose tools come from the servers the user adds in Settings › MCP servers.
 */
import type { InlineExtension } from "@earendil-works/pi-coding-agent";
import { createAskUserQuestionExtension } from "./ask-user-question.js";
import { createAutoTitleExtension, type AutoTitleController } from "./auto-title.js";
import type { BuiltinHost } from "./host.js";
import { type McpController, createMcpExtension } from "./mcp/index.js";
import { type PlanModeController, createPlanModeExtension } from "./plan-mode/index.js";
import { type SubagentsController, createSubagentsExtension } from "./subagents/index.js";
import { type TodoHandle, createTodoExtension } from "./todo/index.js";
import { WEB_FETCH_TOOL_NAME, createWebFetchExtension } from "./web-fetch/index.js";

/**
 * Built-in tools the user's Settings denylist applies to. Every other built-in tool is part of
 * how the app works (a disabled plan_mode_complete would break Plan mode) and ignores it.
 */
export const SWITCHABLE_BUILTIN_TOOLS: ReadonlySet<string> = new Set([WEB_FETCH_TOOL_NAME]);

export interface BuiltinExtensions {
  /** Factories handed to `DefaultResourceLoader.extensionFactories`. */
  factories: InlineExtension[];
  /** Switches the plan-mode extension between Build, Plan and Ultra Plan. */
  planMode: PlanModeController;
  /** Read access to the todo list so snapshots can seed the panel. */
  todo: TodoHandle;
  /** Applies the user's sub-agent settings and says when its tool must stay off. */
  subagents: SubagentsController;
  autoTitle: AutoTitleController;
  /** The user's MCP servers: connects them and says which of their tools are usable. */
  mcp: McpController;
}

export function createBuiltinExtensions(host: BuiltinHost): BuiltinExtensions {
  const mcp = createMcpExtension(host);
  const planMode = createPlanModeExtension(host, {
    owns: (name) => mcp.controller.serverOf(name) !== undefined,
    isReadOnly: (name) => mcp.controller.isReadOnlyTool(name),
  });
  const todo = createTodoExtension(host);
  // One instance for the chat and its sub-agents, so they share its page cache.
  const webFetch: InlineExtension = { name: "wackcode-web-fetch", factory: createWebFetchExtension(), hidden: true };
  const subagents = createSubagentsExtension(host, () => planMode.controller.getState().mode, webFetch);
  const autoTitle = createAutoTitleExtension(host);
  return {
    factories: [
      {
        name: "wackcode-ask",
        // Ultra Plan's questionnaires offer "Write the plan now".
        factory: createAskUserQuestionExtension(host, () => planMode.controller.getState().mode === "ultraplan"),
        hidden: true,
      },
      { name: "wackcode-plan-mode", factory: planMode.factory, hidden: true },
      { name: "wackcode-todo", factory: todo.factory, hidden: true },
      { name: "wackcode-subagents", factory: subagents.factory, hidden: true },
      { name: "wackcode-auto-title", factory: autoTitle.factory, hidden: true },
      webFetch,
      { name: "wackcode-mcp", factory: mcp.factory, hidden: true },
    ],
    planMode: planMode.controller,
    todo: todo.handle,
    subagents: subagents.controller,
    autoTitle: autoTitle.controller,
    mcp: mcp.controller,
  };
}
